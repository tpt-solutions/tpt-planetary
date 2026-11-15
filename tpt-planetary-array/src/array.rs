//! The [`Array`] handle: opening, creating, reading and writing chunked
//! arrays through a [`Store`].

use alloc::format;
use alloc::string::ToString;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::codec::{Pipeline, Registry};
use crate::dtype::DType;
use crate::error::{Error, Result};
use crate::fill::FillValue;
use crate::grid::{ChunkGrid, c_strides};
use crate::hierarchy;
use crate::meta::{ArrayMetadata, NodeMetadata};
use crate::plan::{ChunkTask, RegionPlan};
use crate::selection::{Selection, output_shape, resolve_all};
use crate::store::{Store, join};

/// An array stored in a [`Store`] under a path.
///
/// The handle holds the parsed metadata and the codec pipeline; chunk data is
/// only read when a read method is called.
pub struct Array {
    store: Arc<dyn Store>,
    path: alloc::string::String,
    meta: ArrayMetadata,
    grid: ChunkGrid,
    pipeline: Pipeline,
}

impl core::fmt::Debug for Array {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Array")
            .field("path", &self.path)
            .field("shape", &self.meta.shape)
            .field("chunk_shape", &self.meta.chunk_shape)
            .field("dtype", &self.meta.dtype)
            .finish_non_exhaustive()
    }
}

impl Array {
    fn from_parts(
        store: Arc<dyn Store>,
        path: alloc::string::String,
        meta: ArrayMetadata,
        registry: &Registry,
    ) -> Result<Self> {
        let grid = meta.grid()?;
        let pipeline = meta.pipeline(registry)?;
        Ok(Self {
            store,
            path,
            meta,
            grid,
            pipeline,
        })
    }

    /// Creates a new array at `path`, writing its metadata. Fails if a node
    /// already exists there.
    pub fn create(store: Arc<dyn Store>, path: &str, meta: ArrayMetadata) -> Result<Self> {
        if hierarchy::read_node(store.as_ref(), path)?.is_some() {
            return Err(Error::InvalidMetadata(format!(
                "a node already exists at '{path}'"
            )));
        }
        let registry = Registry::builtin();
        let array = Self::from_parts(store, path.trim_matches('/').to_string(), meta, &registry)?;
        hierarchy::write_array(array.store.as_ref(), &array.path, &array.meta)?;
        Ok(array)
    }

    /// Opens the array at `path` using the builtin codec registry.
    pub fn open(store: Arc<dyn Store>, path: &str) -> Result<Self> {
        Self::open_with(store, path, &Registry::builtin())
    }

    /// Opens the array at `path`, building codecs from `registry`.
    pub fn open_with(store: Arc<dyn Store>, path: &str, registry: &Registry) -> Result<Self> {
        let path = path.trim_matches('/').to_string();
        match hierarchy::read_node(store.as_ref(), &path)? {
            Some(NodeMetadata::Array(meta)) => Self::from_parts(store, path, meta, registry),
            Some(NodeMetadata::Group(_)) => {
                Err(Error::InvalidMetadata(format!("'{path}' is a group")))
            }
            None => Err(Error::NotFound(format!("no node at '{path}'"))),
        }
    }

    /// The path of the array within its store (empty for a root array).
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The array metadata.
    pub fn metadata(&self) -> &ArrayMetadata {
        &self.meta
    }

    /// The array shape.
    pub fn shape(&self) -> &[u64] {
        &self.meta.shape
    }

    /// The chunk shape.
    pub fn chunk_shape(&self) -> &[u64] {
        &self.meta.chunk_shape
    }

    /// The element type.
    pub fn dtype(&self) -> DType {
        self.meta.dtype
    }

    /// The fill value.
    pub fn fill_value(&self) -> &FillValue {
        &self.meta.fill_value
    }

    /// The chunk grid.
    pub fn grid(&self) -> &ChunkGrid {
        &self.grid
    }

    /// The store that holds this array.
    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    fn chunk_key(&self, coord: &[u64]) -> alloc::string::String {
        join(&self.path, &self.meta.chunk_key.key(coord))
    }

    fn chunk_bytes_len(&self) -> Result<usize> {
        let elems = self.grid.chunk_elements()? as usize;
        Ok(elems * self.meta.dtype.item_size())
    }

    /// Reads and decodes the full chunk at `coord`. Returns `None` when the
    /// chunk has never been written (its contents are the fill value).
    pub fn read_chunk(&self, coord: &[u64]) -> Result<Option<Vec<u8>>> {
        self.grid.check_coord(coord)?;
        let Some(raw) = self.store.get(&self.chunk_key(coord))? else {
            return Ok(None);
        };
        let decoded = self.pipeline.decode(&raw)?;
        let expected = self.chunk_bytes_len()?;
        if decoded.len() != expected {
            return Err(Error::SizeMismatch {
                expected,
                actual: decoded.len(),
            });
        }
        Ok(Some(decoded))
    }

    /// Encodes and stores a full chunk at `coord`. `data` must contain exactly
    /// one full chunk (including padding for edge chunks) in C order.
    pub fn write_chunk(&self, coord: &[u64], data: &[u8]) -> Result<()> {
        self.grid.check_coord(coord)?;
        let expected = self.chunk_bytes_len()?;
        if data.len() != expected {
            return Err(Error::SizeMismatch {
                expected,
                actual: data.len(),
            });
        }
        let encoded = self.pipeline.encode(data)?;
        self.store.set(&self.chunk_key(coord), &encoded)
    }

    /// Reads a selection into a new byte buffer in C order. Returns the output
    /// shape (integer-indexed dimensions removed) and the bytes, which use the
    /// array's element type and byte order.
    pub fn read_bytes(&self, selection: &[Selection]) -> Result<(Vec<u64>, Vec<u8>)> {
        let resolved = resolve_all(&self.meta.shape, selection)?;
        let out_shape = output_shape(&resolved);
        let item = self.meta.dtype.item_size();
        let n_out: u64 = out_shape.iter().product();
        let mut out = self
            .meta
            .fill_value
            .repeat_bytes(&self.meta.dtype, n_out as usize)?;
        let plan = RegionPlan::new(&resolved, &self.meta.chunk_shape);
        let chunk_strides = c_strides(&self.meta.chunk_shape);
        let out_strides = c_strides(&out_shape);
        for task in plan.tasks() {
            let Some(chunk) = self.read_chunk(&task.coord)? else {
                continue; // already filled with the fill value
            };
            let geom = Geometry::new(&task, &resolved, &chunk_strides, &out_strides);
            copy_nd(
                &chunk,
                geom.chunk_base,
                &geom.chunk_steps,
                &mut out,
                geom.sel_base,
                &geom.sel_steps,
                &geom.counts,
                item,
            );
        }
        Ok((out_shape, out))
    }

    /// Reads a selection as `f64` values (see [`DType::decode_f64`]).
    pub fn read_f64(&self, selection: &[Selection]) -> Result<Vec<f64>> {
        let (_, bytes) = self.read_bytes(selection)?;
        self.meta.dtype.decode_f64(&bytes)
    }

    /// Writes raw bytes into a selection. `data` must hold exactly the number
    /// of elements in the selection, in C order, using the array's dtype.
    ///
    /// Chunks that are only partly covered are read, updated and re-encoded
    /// (read-modify-write). Chunks that are fully covered are replaced.
    pub fn write_bytes(&self, selection: &[Selection], data: &[u8]) -> Result<()> {
        let resolved = resolve_all(&self.meta.shape, selection)?;
        let out_shape = output_shape(&resolved);
        let item = self.meta.dtype.item_size();
        let n_out: u64 = out_shape.iter().product();
        let expected = n_out as usize * item;
        if data.len() != expected {
            return Err(Error::SizeMismatch {
                expected,
                actual: data.len(),
            });
        }
        let plan = RegionPlan::new(&resolved, &self.meta.chunk_shape);
        let chunk_strides = c_strides(&self.meta.chunk_shape);
        let out_strides = c_strides(&out_shape);
        for task in plan.tasks() {
            let geom = Geometry::new(&task, &resolved, &chunk_strides, &out_strides);
            let full = task
                .pieces
                .iter()
                .zip(&self.meta.chunk_shape)
                .all(|(p, &cs)| p.local_start == 0 && p.local_step == 1 && p.count == cs);
            let mut chunk = if full {
                self.meta
                    .fill_value
                    .repeat_bytes(&self.meta.dtype, self.grid.chunk_elements()? as usize)?
            } else {
                match self.read_chunk(&task.coord)? {
                    Some(c) => c,
                    None => self
                        .meta
                        .fill_value
                        .repeat_bytes(&self.meta.dtype, self.grid.chunk_elements()? as usize)?,
                }
            };
            copy_nd(
                data,
                geom.sel_base,
                &geom.sel_steps,
                &mut chunk,
                geom.chunk_base,
                &geom.chunk_steps,
                &geom.counts,
                item,
            );
            self.write_chunk(&task.coord, &chunk)?;
        }
        Ok(())
    }

    /// Writes `f64` values into a selection, converting to the array dtype.
    pub fn write_f64(&self, selection: &[Selection], values: &[f64]) -> Result<()> {
        let bytes = self.meta.dtype.encode_f64(values)?;
        self.write_bytes(selection, &bytes)
    }

    /// Returns the chunk coordinates that would be touched by `selection`,
    /// without reading any data.
    pub fn plan(&self, selection: &[Selection]) -> Result<Vec<Vec<u64>>> {
        let resolved = resolve_all(&self.meta.shape, selection)?;
        let plan = RegionPlan::new(&resolved, &self.meta.chunk_shape);
        Ok(plan.tasks().map(|t| t.coord).collect())
    }
}

/// Offsets and counts for one chunk's contribution, expressed in elements.
struct Geometry {
    chunk_base: usize,
    chunk_steps: Vec<usize>,
    sel_base: usize,
    sel_steps: Vec<usize>,
    counts: Vec<usize>,
}

impl Geometry {
    fn new(
        task: &ChunkTask,
        resolved: &[crate::selection::ResolvedDim],
        chunk_strides: &[u64],
        out_strides: &[u64],
    ) -> Self {
        let mut g = Geometry {
            chunk_base: 0,
            chunk_steps: Vec::with_capacity(task.pieces.len()),
            sel_base: 0,
            sel_steps: Vec::with_capacity(task.pieces.len()),
            counts: Vec::with_capacity(task.pieces.len()),
        };
        let mut kept = 0usize;
        for (d, piece) in task.pieces.iter().enumerate() {
            let cs = chunk_strides[d] as usize;
            g.chunk_base += (piece.local_start as usize).saturating_mul(cs);
            g.chunk_steps
                .push((piece.local_step as usize).saturating_mul(cs));
            g.counts.push(piece.count as usize);
            if resolved[d].kept {
                let os = out_strides[kept] as usize;
                g.sel_base += (piece.out_start as usize).saturating_mul(os);
                g.sel_steps.push(os);
                kept += 1;
            } else {
                g.sel_steps.push(0);
            }
        }
        g
    }
}

/// Copies `counts`-shaped blocks of elements between two byte buffers.
///
/// Each side is addressed as `base + sum(index_d * step_d)` in units of
/// `itemsize` bytes. The two buffers must not alias.
#[allow(clippy::too_many_arguments)]
fn copy_nd(
    src: &[u8],
    src_base: usize,
    src_steps: &[usize],
    dst: &mut [u8],
    dst_base: usize,
    dst_steps: &[usize],
    counts: &[usize],
    itemsize: usize,
) {
    if counts.iter().any(|&c| c == 0) {
        return;
    }
    let nd = counts.len();
    let mut idx = vec![0usize; nd];
    let (mut s, mut d) = (src_base, dst_base);
    loop {
        let so = s * itemsize;
        let dst_off = d * itemsize;
        dst[dst_off..dst_off + itemsize].copy_from_slice(&src[so..so + itemsize]);
        let mut k = nd;
        loop {
            if k == 0 {
                return;
            }
            k -= 1;
            idx[k] += 1;
            s += src_steps[k];
            d += dst_steps[k];
            if idx[k] < counts[k] {
                break;
            }
            s -= src_steps[k] * counts[k];
            d -= dst_steps[k] * counts[k];
            idx[k] = 0;
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::meta::{CodecSpec, ZarrFormat};
    use crate::selection::Slice;
    use crate::store::MemoryStore;
    use serde_json::json;

    fn store() -> Arc<MemoryStore> {
        Arc::new(MemoryStore::new())
    }

    fn values(n: usize) -> Vec<f64> {
        (0..n).map(|i| i as f64).collect()
    }

    #[test]
    fn full_write_then_read_back_all_selections() {
        let s = store();
        let meta =
            ArrayMetadata::new(ZarrFormat::V2, vec![5, 7], vec![2, 3], DType::FLOAT64).unwrap();
        let a = Array::create(s.clone(), "x", meta).unwrap();
        let v = values(35);
        a.write_f64(&[Selection::ALL, Selection::ALL], &v).unwrap();
        assert_eq!(a.read_f64(&[Selection::ALL, Selection::ALL]).unwrap(), v);
        // Row 3, columns 2..6
        let row = a
            .read_f64(&[Selection::Index(3), Selection::Range(Slice::range(2, 6))])
            .unwrap();
        assert_eq!(row, vec![23.0, 24.0, 25.0, 26.0]);
        // Strided: rows 0,2,4 and columns 1,4
        let sub = a
            .read_f64(&[
                Selection::Range(Slice {
                    start: None,
                    stop: None,
                    step: 2,
                }),
                Selection::Range(Slice {
                    start: Some(1),
                    stop: None,
                    step: 3,
                }),
            ])
            .unwrap();
        assert_eq!(sub, vec![1.0, 4.0, 15.0, 18.0, 29.0, 32.0]);
    }

    #[test]
    fn partial_update_is_read_modify_write() {
        let s = store();
        let meta = ArrayMetadata::new(ZarrFormat::V3, vec![4, 4], vec![2, 2], DType::INT32)
            .unwrap()
            .with_fill_value(FillValue::Int(-1));
        let a = Array::create(s.clone(), "arr", meta).unwrap();
        // Untouched chunks read as fill.
        assert_eq!(
            a.read_f64(&[Selection::ALL, Selection::ALL]).unwrap(),
            vec![-1.0; 16]
        );
        a.write_f64(
            &[
                Selection::Range(Slice::range(1, 3)),
                Selection::Range(Slice::range(1, 3)),
            ],
            &[1.0, 2.0, 3.0, 4.0],
        )
        .unwrap();
        let all = a.read_f64(&[Selection::ALL, Selection::ALL]).unwrap();
        let expected = vec![
            -1.0, -1.0, -1.0, -1.0, //
            -1.0, 1.0, 2.0, -1.0, //
            -1.0, 3.0, 4.0, -1.0, //
            -1.0, -1.0, -1.0, -1.0,
        ];
        assert_eq!(all, expected);
    }

    #[test]
    fn edge_chunks_are_padded_and_trimmed() {
        let s = store();
        let meta = ArrayMetadata::new(ZarrFormat::V2, vec![5], vec![4], DType::UINT64).unwrap();
        let a = Array::create(s.clone(), "e", meta).unwrap();
        a.write_f64(&[Selection::ALL], &[1.0, 2.0, 3.0, 4.0, 5.0])
            .unwrap();
        // Second chunk stores 4 elements, only the first is in the array.
        let chunk = a.read_chunk(&[1]).unwrap().unwrap();
        assert_eq!(chunk.len(), 4 * 8);
        assert_eq!(
            a.read_f64(&[Selection::Range(Slice::range(3, 5))]).unwrap(),
            vec![4.0, 5.0]
        );
        assert!(a.read_chunk(&[2]).is_err());
    }

    #[test]
    fn compressed_big_endian_round_trip() {
        let s = store();
        let dtype = DType::from_v2(">i4").unwrap();
        let meta = ArrayMetadata::new(ZarrFormat::V2, vec![6, 5], vec![4, 4], dtype)
            .unwrap()
            .with_codecs(vec![
                CodecSpec {
                    name: "shuffle".into(),
                    config: json!({"id": "shuffle", "elementsize": 4}),
                },
                CodecSpec {
                    name: "gzip".into(),
                    config: json!({"id": "gzip", "level": 5}),
                },
            ]);
        let a = Array::create(s.clone(), "c", meta).unwrap();
        let v = values(30);
        a.write_f64(&[Selection::ALL, Selection::ALL], &v).unwrap();
        let reopened = Array::open(s.clone(), "c").unwrap();
        assert_eq!(
            reopened
                .read_f64(&[Selection::ALL, Selection::ALL])
                .unwrap(),
            v
        );
        assert_eq!(reopened.dtype(), dtype);
    }

    #[test]
    fn v3_array_with_slash_keys_and_lz4() {
        let s = store();
        let meta = ArrayMetadata::new(ZarrFormat::V3, vec![3, 3], vec![2, 2], DType::FLOAT32)
            .unwrap()
            .with_codecs(vec![CodecSpec {
                name: "lz4".into(),
                config: json!({}),
            }]);
        let a = Array::create(s.clone(), "grp/arr", meta).unwrap();
        a.write_f64(&[Selection::ALL, Selection::ALL], &values(9))
            .unwrap();
        assert!(s.contains("grp/arr/c/1/1").unwrap());
        let r = Array::open(s.clone(), "grp/arr").unwrap();
        assert_eq!(
            r.read_f64(&[Selection::Index(2), Selection::ALL]).unwrap(),
            vec![6.0, 7.0, 8.0]
        );
    }

    #[test]
    fn rejects_existing_path_and_bad_sizes() {
        let s = store();
        let meta = ArrayMetadata::new(ZarrFormat::V2, vec![2], vec![2], DType::FLOAT64).unwrap();
        let a = Array::create(s.clone(), "a", meta.clone()).unwrap();
        assert!(Array::create(s.clone(), "a", meta).is_err());
        assert!(a.write_bytes(&[Selection::ALL], &[0u8; 3]).is_err());
        assert!(Array::open(s.clone(), "missing").is_err());
    }
}
