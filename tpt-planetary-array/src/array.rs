//! The [`Array`] handle: opening, creating, reading and writing chunked
//! arrays through a [`Store`].

use alloc::format;
use alloc::string::ToString;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

#[cfg(feature = "std")]
use crate::cache::ChunkCache;
use crate::codec::sharding::ShardingCodec;
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
/// only read when a read method is called. With
/// [`Array::with_chunk_cache`] the handle also keeps recently used decoded
/// chunks in memory.
pub struct Array {
    store: Arc<dyn Store>,
    path: alloc::string::String,
    meta: ArrayMetadata,
    grid: ChunkGrid,
    pipeline: Pipeline,
    /// The sharding codec, when the chunk pipeline is exactly one
    /// `sharding_indexed` codec. Reads then fetch only the index and the inner
    /// chunks a selection needs.
    shard: Option<ShardingCodec>,
    /// Decoded-chunk cache, present only when enabled (`std` feature: the
    /// mutex that makes the handle `Sync` needs `std`).
    #[cfg(feature = "std")]
    chunk_cache: Option<std::sync::Mutex<ChunkCache>>,
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
        let shard = shard_codec(&meta, registry)?;
        Ok(Self {
            store,
            path,
            meta,
            grid,
            pipeline,
            shard,
            #[cfg(feature = "std")]
            chunk_cache: None,
        })
    }

    /// Enables an LRU cache of decoded chunks holding at most `budget_bytes`
    /// bytes. Reads that hit the cache do not call the store. Chunks written
    /// through this handle are cached as they are written.
    ///
    /// The cache belongs to this handle. Writes made through another handle
    /// (or directly to the store) are not seen until the cached entry is
    /// evicted, so use one handle per array when caching. A chunk larger than
    /// the budget is never cached.
    #[cfg(feature = "std")]
    pub fn with_chunk_cache(mut self, budget_bytes: usize) -> Self {
        self.chunk_cache = Some(std::sync::Mutex::new(ChunkCache::new(budget_bytes)));
        self
    }

    /// Bytes of decoded chunks currently held by the chunk cache, or `None`
    /// when no cache is enabled.
    #[cfg(feature = "std")]
    pub fn chunk_cache_bytes(&self) -> Result<Option<usize>> {
        match &self.chunk_cache {
            None => Ok(None),
            Some(cache) => Ok(Some(lock_cache(cache)?.used_bytes())),
        }
    }

    #[cfg(feature = "std")]
    fn cache_get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let Some(cache) = &self.chunk_cache else {
            return Ok(None);
        };
        Ok(lock_cache(cache)?.get(key).map(<[u8]>::to_vec))
    }

    #[cfg(feature = "std")]
    fn cache_put(&self, key: &str, decoded: &[u8]) -> Result<()> {
        if let Some(cache) = &self.chunk_cache {
            lock_cache(cache)?.insert(key, decoded.to_vec());
        }
        Ok(())
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
        let key = self.chunk_key(coord);
        #[cfg(feature = "std")]
        {
            if let Some(hit) = self.cache_get(&key)? {
                return Ok(Some(hit));
            }
        }
        let Some(raw) = self.store.get(&key)? else {
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
        #[cfg(feature = "std")]
        {
            self.cache_put(&key, &decoded)?;
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
        let key = self.chunk_key(coord);
        let encoded = self.pipeline.encode(data)?;
        self.store.set(&key, &encoded)?;
        #[cfg(feature = "std")]
        {
            self.cache_put(&key, data)?;
        }
        Ok(())
    }

    /// Reads the chunk at `coord` for a read whose pieces are `pieces`. For a
    /// sharded array this fetches only the index and the inner chunks the
    /// pieces touch; other arrays read the whole chunk.
    fn read_task_chunk(
        &self,
        coord: &[u64],
        pieces: &[crate::plan::DimPiece],
    ) -> Result<Option<Vec<u8>>> {
        match &self.shard {
            None => self.read_chunk(coord),
            Some(shard) => {
                let selections: Vec<(u64, u64, u64)> = pieces
                    .iter()
                    .map(|p| (p.local_start, p.local_step, p.count))
                    .collect();
                self.read_shard_partial(shard, coord, &selections)
            }
        }
    }

    /// Partial read of one shard: range-reads the index, then only the inner
    /// chunks in `selections`. Bypasses the chunk cache.
    fn read_shard_partial(
        &self,
        shard: &ShardingCodec,
        coord: &[u64],
        selections: &[(u64, u64, u64)],
    ) -> Result<Option<Vec<u8>>> {
        self.grid.check_coord(coord)?;
        let key = self.chunk_key(coord);
        let Some(size) = self.store.size(&key)? else {
            return Ok(None);
        };
        let il = shard.index_size() as u64;
        if size < il {
            return Err(Error::Codec(
                "sharding: shard shorter than its index".into(),
            ));
        }
        let index_bytes = self
            .store
            .get_range(&key, shard.index_offset(size)?, il)?
            .ok_or_else(|| Error::NotFound(format!("shard index '{key}'")))?;
        if index_bytes.len() as u64 != il {
            return Err(Error::Codec(
                "sharding: short read of the shard index".into(),
            ));
        }
        let index = shard.parse_index_section(&index_bytes, size as usize)?;
        let wanted = shard.inner_indices_touched(selections)?;
        let store = &self.store;
        let bytes = shard.decode_selected(&index, &wanted, |off, len| {
            let v = store
                .get_range(&key, off, len)?
                .ok_or_else(|| Error::NotFound(format!("shard '{key}'")))?;
            if v.len() as u64 != len {
                return Err(Error::Codec(
                    "sharding: short read of an inner chunk".into(),
                ));
            }
            Ok(v)
        })?;
        Ok(Some(bytes))
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
            let Some(chunk) = self.read_task_chunk(&task.coord, &task.pieces)? else {
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

#[cfg(feature = "std")]
fn lock_cache(
    cache: &std::sync::Mutex<ChunkCache>,
) -> Result<std::sync::MutexGuard<'_, ChunkCache>> {
    cache
        .lock()
        .map_err(|_| Error::Io("chunk cache lock poisoned".to_string()))
}

/// The sharding codec of `meta`, when its only codec is `sharding_indexed`.
/// Any other pipeline (transpose, or bytes-to-bytes codecs around the shard)
/// returns `None` and is read whole.
fn shard_codec(meta: &ArrayMetadata, registry: &Registry) -> Result<Option<ShardingCodec>> {
    match meta.codecs.as_slice() {
        [spec] if spec.name == "sharding_indexed" => {
            let fill = meta.fill_value.to_element_bytes(&meta.dtype)?;
            Ok(Some(ShardingCodec::new(
                &spec.config,
                &meta.chunk_shape,
                &meta.dtype,
                &fill,
                registry,
            )?))
        }
        _ => Ok(None),
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
    if counts.contains(&0) {
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
    use crate::plan::DimPiece;
    use crate::selection::ResolvedDim;

    #[test]
    fn chunk_geometry_saturates_instead_of_wrapping() {
        // In range: offsets are plain products of the element strides.
        let task = ChunkTask {
            coord: vec![0, 0],
            pieces: vec![
                DimPiece {
                    chunk: 0,
                    local_start: 3,
                    local_step: 2,
                    count: 2,
                    out_start: 1,
                },
                DimPiece {
                    chunk: 0,
                    local_start: 1,
                    local_step: 1,
                    count: 1,
                    out_start: 0,
                },
            ],
        };
        let resolved = [
            ResolvedDim {
                start: 0,
                step: 1,
                count: 2,
                kept: true,
            },
            ResolvedDim {
                start: 0,
                step: 1,
                count: 1,
                kept: true,
            },
        ];
        let g = Geometry::new(&task, &resolved, &[4, 1], &[2, 1]);
        assert_eq!(g.chunk_base, 3 * 4 + 1);
        assert_eq!(g.chunk_steps, vec![2 * 4, 1]);
        assert_eq!(g.sel_base, 2);
        assert_eq!(g.sel_steps, vec![2, 1]);

        // Out of range: an offset that does not fit in usize must saturate.
        // It must not wrap to a small, plausible index, which would copy the
        // wrong elements without any error.
        let huge = ChunkTask {
            coord: vec![0],
            pieces: vec![DimPiece {
                chunk: 0,
                local_start: u64::MAX / 2,
                local_step: 1,
                count: 1,
                out_start: 0,
            }],
        };
        let g = Geometry::new(
            &huge,
            &[ResolvedDim {
                start: 0,
                step: 1,
                count: 1,
                kept: true,
            }],
            &[4],
            &[1],
        );
        assert_eq!(g.chunk_base, usize::MAX);
    }

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

    /// Counts `get` calls that reach the underlying store.
    struct CountingStore {
        inner: MemoryStore,
        gets: std::sync::atomic::AtomicUsize,
    }

    impl CountingStore {
        fn gets(&self) -> usize {
            self.gets.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl Store for CountingStore {
        fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
            self.gets.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.get(key)
        }
        fn set(&self, key: &str, value: &[u8]) -> Result<()> {
            self.inner.set(key, value)
        }
        fn delete(&self, key: &str) -> Result<()> {
            self.inner.delete(key)
        }
        fn list(&self, prefix: &str) -> Result<Vec<String>> {
            self.inner.list(prefix)
        }
    }

    /// A 4x4 f64 array in 2x2 chunks (four chunks of 32 bytes each), written
    /// through a cache-less handle, then reopened with a cache of `budget`.
    fn cached_handle(budget: usize) -> (Arc<CountingStore>, Array) {
        let s = Arc::new(CountingStore {
            inner: MemoryStore::new(),
            gets: std::sync::atomic::AtomicUsize::new(0),
        });
        let meta =
            ArrayMetadata::new(ZarrFormat::V2, vec![4, 4], vec![2, 2], DType::FLOAT64).unwrap();
        let writer = Array::create(s.clone(), "c", meta).unwrap();
        writer
            .write_f64(&[Selection::ALL, Selection::ALL], &values(16))
            .unwrap();
        let reader = Array::open(s.clone(), "c")
            .unwrap()
            .with_chunk_cache(budget);
        // Opening reads metadata keys; only chunk reads are counted below.
        s.gets.store(0, std::sync::atomic::Ordering::SeqCst);
        (s, reader)
    }

    #[test]
    fn cache_hits_avoid_store_reads() {
        let (s, a) = cached_handle(1024);
        let all = [Selection::ALL, Selection::ALL];
        assert_eq!(a.read_f64(&all).unwrap(), values(16));
        assert_eq!(s.gets(), 4);
        assert_eq!(a.read_f64(&all).unwrap(), values(16));
        assert_eq!(s.gets(), 4, "second read is served from the cache");
        assert_eq!(a.chunk_cache_bytes().unwrap(), Some(4 * 32));
    }

    #[test]
    fn cache_budget_is_respected_and_eviction_keeps_data_correct() {
        // Room for two of the four 32-byte chunks.
        let (s, a) = cached_handle(64);
        let all = [Selection::ALL, Selection::ALL];
        assert_eq!(a.read_f64(&all).unwrap(), values(16));
        assert_eq!(s.gets(), 4);
        assert!(a.chunk_cache_bytes().unwrap().unwrap() <= 64);

        // After the full read the cache holds chunks 2 and 3 (LRU order: 2, 3).
        let chunk = |c: &[u64]| a.read_chunk(c).unwrap().unwrap();
        assert_eq!(chunk(&[1, 0]).len(), 32);
        assert_eq!(s.gets(), 4, "chunk [1,0] is a hit");
        // Chunk [0,0] was evicted; reading it reaches the store and evicts the
        // least recently used entry, which is now [1,1].
        assert_eq!(
            a.read_f64(&[
                Selection::Range(Slice::range(0, 1)),
                Selection::Range(Slice::range(0, 2))
            ])
            .unwrap(),
            vec![0.0, 1.0]
        );
        assert_eq!(s.gets(), 5);
        assert!(a.chunk_cache_bytes().unwrap().unwrap() <= 64);
        // Results stay correct across repeated evicting reads.
        for _ in 0..3 {
            assert_eq!(a.read_f64(&all).unwrap(), values(16));
        }
    }

    #[test]
    fn oversized_chunks_bypass_the_cache() {
        let (s, a) = cached_handle(16);
        let all = [Selection::ALL, Selection::ALL];
        assert_eq!(a.read_f64(&all).unwrap(), values(16));
        assert_eq!(a.read_f64(&all).unwrap(), values(16));
        assert_eq!(s.gets(), 8);
        assert_eq!(a.chunk_cache_bytes().unwrap(), Some(0));
    }

    #[test]
    fn writes_through_the_handle_update_the_cache() {
        let (s, a) = cached_handle(1024);
        let all = [Selection::ALL, Selection::ALL];
        a.read_f64(&all).unwrap();
        let before = s.gets();
        a.write_f64(
            &[Selection::Index(0), Selection::ALL],
            &[-1.0, -2.0, -3.0, -4.0],
        )
        .unwrap();
        assert_eq!(
            a.read_f64(&[Selection::Index(0), Selection::ALL]).unwrap(),
            vec![-1.0, -2.0, -3.0, -4.0]
        );
        assert_eq!(s.gets(), before, "written chunks are served from the cache");
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

#[cfg(all(test, feature = "std"))]
mod shard_read_tests {
    use super::*;
    use crate::meta::v3::parse_node;
    use crate::selection::Slice;
    use crate::store::MemoryStore;
    use core::sync::atomic::{AtomicU64, Ordering};

    /// Wraps a store and counts the bytes returned to the caller.
    struct Counting {
        inner: MemoryStore,
        bytes_read: AtomicU64,
    }

    impl Store for Counting {
        fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
            let v = self.inner.get(key)?;
            if let Some(b) = &v {
                self.bytes_read.fetch_add(b.len() as u64, Ordering::SeqCst);
            }
            Ok(v)
        }
        fn get_range(&self, key: &str, start: u64, len: u64) -> Result<Option<Vec<u8>>> {
            let v = self.inner.get_range(key, start, len)?;
            if let Some(b) = &v {
                self.bytes_read.fetch_add(b.len() as u64, Ordering::SeqCst);
            }
            Ok(v)
        }
        fn size(&self, key: &str) -> Result<Option<u64>> {
            self.inner.size(key)
        }
        fn set(&self, key: &str, value: &[u8]) -> Result<()> {
            self.inner.set(key, value)
        }
        fn delete(&self, key: &str) -> Result<()> {
            self.inner.delete(key)
        }
        fn list(&self, prefix: &str) -> Result<Vec<String>> {
            self.inner.list(prefix)
        }
    }

    /// One 8x8 float64 shard of 2x2 inner chunks (16 inner chunks), with an
    /// uncompressed inner chain and an index without a checksum.
    const SHARD_FIXTURE: &str = r#"{
        "zarr_format": 3,
        "node_type": "array",
        "shape": [8, 8],
        "data_type": "float64",
        "chunk_grid": {"name": "regular", "configuration": {"chunk_shape": [8, 8]}},
        "chunk_key_encoding": {"name": "default", "configuration": {"separator": "/"}},
        "fill_value": 0.0,
        "codecs": [
            {"name": "sharding_indexed", "configuration": {
                "chunk_shape": [2, 2],
                "codecs": [{"name": "bytes", "configuration": {"endian": "little"}}],
                "index_codecs": [{"name": "bytes", "configuration": {"endian": "little"}}],
                "index_location": "end"
            }}
        ]
    }"#;

    fn make() -> (Arc<Counting>, Array) {
        let store = Arc::new(Counting {
            inner: MemoryStore::new(),
            bytes_read: AtomicU64::new(0),
        });
        let NodeMetadata::Array(meta) = parse_node(SHARD_FIXTURE.as_bytes()).unwrap() else {
            panic!("expected an array");
        };
        let array = Array::create(store.clone(), "a", meta).unwrap();
        (store, array)
    }

    #[test]
    fn partial_read_fetches_only_the_index_and_touched_inner_chunks() {
        let (store, array) = make();
        let data: Vec<f64> = (0..64).map(|i| i as f64 + 1.0).collect();
        array.write_f64(&[], &data).unwrap();

        store.bytes_read.store(0, Ordering::SeqCst);
        // Rows 2..4 and columns 4..6: exactly one inner chunk.
        let sel = [
            Selection::Range(Slice::range(2, 4)),
            Selection::Range(Slice::range(4, 6)),
        ];
        let got = array.read_f64(&sel).unwrap();
        let expected: Vec<f64> = [(2, 4), (2, 5), (3, 4), (3, 5)]
            .iter()
            .map(|&(r, c)| data[r * 8 + c])
            .collect();
        assert_eq!(got, expected);
        // Index: 16 entries of 16 bytes. Data: one 2x2 float64 inner chunk.
        let fetched = store.bytes_read.load(Ordering::SeqCst);
        assert_eq!(fetched, 16 * 16 + 4 * 8);
        // The whole shard is 64 * 8 bytes of data plus the index.
        assert!(fetched < 64 * 8 + 16 * 16);
    }

    #[test]
    fn partial_read_matches_full_read_for_strided_and_missing_chunks() {
        let (_store, array) = make();
        // Write only part of the shard; the other inner chunks stay missing.
        let block: Vec<f64> = (0..4).map(|i| 100.0 + i as f64).collect();
        array
            .write_f64(
                &[
                    Selection::Range(Slice::range(0, 2)),
                    Selection::Range(Slice::range(0, 2)),
                ],
                &block,
            )
            .unwrap();
        // A strided selection that touches the written chunk and a missing one.
        let sel = [
            Selection::Range(Slice {
                start: Some(0),
                stop: Some(8),
                step: 3,
            }),
            Selection::Range(Slice::range(0, 8)),
        ];
        let got = array.read_f64(&sel).unwrap();
        // Rows 0 and 3 and 6; column 0..8.
        let (_, full) = array.read_bytes(&[]).unwrap();
        let full: Vec<f64> = full
            .chunks_exact(8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let mut expected = Vec::new();
        for r in [0usize, 3, 6] {
            for c in 0..8 {
                expected.push(full[r * 8 + c]);
            }
        }
        assert_eq!(got, expected);
        assert_eq!(full[0], 100.0);
        assert_eq!(full[1], 101.0);
        assert_eq!(full[8], 102.0);
        assert_eq!(full[2], 0.0); // missing chunk reads as the fill value
    }
}
