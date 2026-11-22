//! Zarr v3 `sharding_indexed` array-to-bytes codec.
//!
//! A shard packs a grid of inner chunks into one stored object. The shard
//! shape is the outer chunk shape; the inner chunk shape must divide it.
//!
//! Configuration (the `configuration` member of the codec):
//!
//! - `chunk_shape`: inner chunk shape (required).
//! - `codecs`: codec chain for each inner chunk, in encoding order. The
//!   `bytes` codec is accepted if its endianness matches the array; the
//!   remaining codecs must be bytes-to-bytes.
//! - `index_codecs`: codecs for the index, default `[bytes (little)]`. The
//!   accepted forms are `[bytes]`, `[bytes, crc32c]` and `[crc32c]`.
//! - `index_location`: `"end"` (default) or `"start"`.
//!
//! Layout: each present inner chunk is encoded with `codecs` and stored
//! back-to-back. The index holds, for every inner chunk in C order, a pair of
//! little-endian `u64` values `(offset, nbytes)` relative to the shard start.
//! A missing inner chunk has both values equal to `2^64 - 1`. When `crc32c`
//! is in `index_codecs`, a little-endian CRC-32C of the index pairs follows
//! them.
//!
//! The codec is an array-to-bytes stage: [`Codec::decode`] returns the whole
//! outer chunk in C order, with missing inner chunks set to the fill value.
//! [`Codec::encode`] splits the chunk into inner chunks; inner chunks equal
//! everywhere to the fill value are written as missing.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use serde_json::{Value, json};

use super::crc32c::crc32c;
use super::transpose::TransposeCodec;
use super::{Codec, Pipeline, Registry};
use crate::dtype::{DType, Endian};
use crate::error::{Error, Result};

/// Index entry value marking a missing inner chunk.
const MISSING: u64 = u64::MAX;

/// Size of one index entry (offset and nbytes, both `u64`).
const ENTRY_LEN: usize = 16;

/// Where the index sits in the shard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexLocation {
    /// The index follows the inner chunks.
    End,
    /// The index precedes the inner chunks.
    Start,
}

/// Parsed shard index: for each inner chunk in C order, its `(offset, nbytes)`
/// within the shard, or `None` if the chunk is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardIndex {
    entries: Vec<Option<(u64, u64)>>,
}

impl ShardIndex {
    /// Index entries in C order of the inner chunk grid.
    pub fn entries(&self) -> &[Option<(u64, u64)>] {
        &self.entries
    }

    /// Number of inner chunks.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True if the index has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Number of elements in a C-ordered shape.
fn numel(shape: &[u64]) -> usize {
    shape.iter().map(|&s| s as usize).product()
}

/// C-order element strides of `shape`.
fn c_strides(shape: &[u64]) -> Vec<usize> {
    let nd = shape.len();
    let mut strides = vec![1usize; nd];
    for k in (0..nd.saturating_sub(1)).rev() {
        strides[k] = strides[k + 1] * shape[k + 1] as usize;
    }
    strides
}

/// A C-ordered buffer view: bytes, shape, and the origin of a region within it.
type View<'a> = (&'a [u8], &'a [u64], &'a [u64]);

/// Copies a block of `region` elements from `src` into `dst`. Each view gives
/// its buffer, the buffer's shape and the origin of the region within it.
/// Elements are `item` bytes. The caller guarantees that the region fits in
/// both buffers.
fn copy_region(src: View<'_>, dst: (&mut [u8], &[u64], &[u64]), region: &[u64], item: usize) {
    let (src, src_shape, src_origin) = src;
    let (dst, dst_shape, dst_origin) = dst;
    let nd = region.len();
    if region.contains(&0) {
        return;
    }
    if nd == 0 {
        dst[..item].copy_from_slice(&src[..item]);
        return;
    }
    let s_str = c_strides(src_shape);
    let d_str = c_strides(dst_shape);
    let row = region[nd - 1] as usize * item;
    let rows: usize = region[..nd - 1].iter().map(|&r| r as usize).product();
    let mut idx = vec![0u64; nd - 1];
    for _ in 0..rows {
        let mut s = src_origin[nd - 1] as usize * s_str[nd - 1];
        let mut d = dst_origin[nd - 1] as usize * d_str[nd - 1];
        for k in 0..nd - 1 {
            s += (src_origin[k] + idx[k]) as usize * s_str[k];
            d += (dst_origin[k] + idx[k]) as usize * d_str[k];
        }
        let (s, d) = (s * item, d * item);
        dst[d..d + row].copy_from_slice(&src[s..s + row]);
        for k in (0..nd - 1).rev() {
            idx[k] += 1;
            if idx[k] < region[k] {
                break;
            }
            idx[k] = 0;
        }
    }
}

/// Reads an array of non-negative integers.
fn u64_list(v: &Value, what: &str) -> Result<Vec<u64>> {
    v.as_array()
        .ok_or_else(|| Error::InvalidMetadata(format!("{what} must be an array")))?
        .iter()
        .map(|x| {
            x.as_u64()
                .filter(|&n| n > 0)
                .ok_or_else(|| Error::InvalidMetadata(format!("{what} must be positive integers")))
        })
        .collect()
}

/// The `sharding_indexed` codec for one shard shape.
#[derive(Debug)]
pub struct ShardingCodec {
    shard_shape: Vec<u64>,
    inner_shape: Vec<u64>,
    grid: Vec<u64>,
    item_size: usize,
    fill: Vec<u8>,
    inner: Pipeline,
    /// Array-to-array transpose at the start of the inner chain, if any.
    inner_transpose: Option<TransposeCodec>,
    index_crc: bool,
    location: IndexLocation,
}

impl ShardingCodec {
    /// Builds the codec from its `configuration` object.
    ///
    /// `shard_shape` is the shape of the chunk being encoded (after any
    /// preceding array-to-array codecs), `fill` is one element of the fill
    /// value in the array's byte order, and `registry` supplies the
    /// bytes-to-bytes codecs of the inner chain.
    pub fn new(
        config: &Value,
        shard_shape: &[u64],
        dtype: &DType,
        fill: &[u8],
        registry: &Registry,
    ) -> Result<Self> {
        let inner_shape = u64_list(
            config
                .get("chunk_shape")
                .ok_or_else(|| Error::InvalidMetadata("sharding needs 'chunk_shape'".into()))?,
            "sharding chunk_shape",
        )?;
        if inner_shape.len() != shard_shape.len() {
            return Err(Error::InvalidMetadata(
                "sharding chunk_shape has the wrong number of dimensions".into(),
            ));
        }
        let mut grid = Vec::with_capacity(shard_shape.len());
        for (&s, &i) in shard_shape.iter().zip(&inner_shape) {
            if s % i != 0 {
                return Err(Error::InvalidMetadata(format!(
                    "inner chunk {i} does not divide shard extent {s}"
                )));
            }
            grid.push(s / i);
        }

        let item_size = dtype.item_size();
        if fill.len() != item_size {
            return Err(Error::InvalidMetadata(
                "sharding fill value has the wrong size".into(),
            ));
        }

        let (inner, inner_transpose) = build_inner(config, &inner_shape, dtype, registry)?;
        let (index_crc, location) = parse_index_options(config)?;

        Ok(Self {
            shard_shape: shard_shape.to_vec(),
            inner_shape,
            grid,
            item_size,
            fill: fill.to_vec(),
            inner,
            inner_transpose,
            index_crc,
            location,
        })
    }

    /// Inner chunk shape.
    pub fn inner_shape(&self) -> &[u64] {
        &self.inner_shape
    }

    /// Number of inner chunks along each dimension.
    pub fn grid_shape(&self) -> &[u64] {
        &self.grid
    }

    /// Total number of inner chunks.
    pub fn num_inner_chunks(&self) -> usize {
        numel(&self.grid)
    }

    /// Whether a CRC-32C follows the index.
    pub fn index_has_crc32c(&self) -> bool {
        self.index_crc
    }

    /// Where the index is stored.
    pub fn index_location(&self) -> IndexLocation {
        self.location
    }

    fn index_len(&self) -> usize {
        ENTRY_LEN * self.num_inner_chunks() + if self.index_crc { 4 } else { 0 }
    }

    fn inner_numel(&self) -> usize {
        numel(&self.inner_shape)
    }

    fn inner_bytes(&self) -> usize {
        self.inner_numel() * self.item_size
    }

    /// Inner chunk coordinates (in chunks) of C-order inner index `n`.
    fn coords(&self, n: usize) -> Vec<u64> {
        let mut rem = n as u64;
        let mut c = vec![0u64; self.grid.len()];
        for k in (0..self.grid.len()).rev() {
            c[k] = rem % self.grid[k];
            rem /= self.grid[k];
        }
        c
    }

    /// Encodes the inner chunks of a shard. `chunks` has one entry per inner
    /// chunk in C order, each the raw element bytes of that inner chunk
    /// (`None` marks a missing chunk).
    pub fn encode_shard(&self, chunks: &[Option<&[u8]>]) -> Result<Vec<u8>> {
        if chunks.len() != self.num_inner_chunks() {
            return Err(Error::InvalidMetadata(format!(
                "shard has {} inner chunks, got {}",
                self.num_inner_chunks(),
                chunks.len()
            )));
        }
        let index_len = self.index_len();
        let mut body = Vec::new();
        let mut entries: Vec<(u64, u64)> = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            match chunk {
                None => entries.push((MISSING, MISSING)),
                Some(bytes) => {
                    if bytes.len() != self.inner_bytes() {
                        return Err(Error::SizeMismatch {
                            expected: self.inner_bytes(),
                            actual: bytes.len(),
                        });
                    }
                    let payload = self.encode_inner(bytes)?;
                    let base = match self.location {
                        IndexLocation::End => 0,
                        IndexLocation::Start => index_len,
                    };
                    entries.push(((base + body.len()) as u64, payload.len() as u64));
                    body.extend_from_slice(&payload);
                }
            }
        }
        let mut index = Vec::with_capacity(index_len);
        for (off, n) in &entries {
            index.extend_from_slice(&off.to_le_bytes());
            index.extend_from_slice(&n.to_le_bytes());
        }
        if self.index_crc {
            index.extend_from_slice(&crc32c(&index).to_le_bytes());
        }
        Ok(match self.location {
            IndexLocation::End => {
                body.extend_from_slice(&index);
                body
            }
            IndexLocation::Start => {
                index.extend_from_slice(&body);
                index
            }
        })
    }

    /// Size in bytes of the shard index (entries plus optional checksum).
    pub fn index_size(&self) -> usize {
        self.index_len()
    }

    /// Byte offset of the index within a shard of `shard_len` bytes.
    pub fn index_offset(&self, shard_len: u64) -> Result<u64> {
        let il = self.index_len() as u64;
        match self.location {
            IndexLocation::End => shard_len
                .checked_sub(il)
                .ok_or_else(|| Error::Codec("sharding: shard shorter than its index".into())),
            IndexLocation::Start => Ok(0),
        }
    }

    /// Parses and validates the shard index.
    pub fn parse_index(&self, shard: &[u8]) -> Result<ShardIndex> {
        let il = self.index_len();
        if shard.len() < il {
            return Err(Error::Codec(
                "sharding: shard shorter than its index".into(),
            ));
        }
        let index_bytes = match self.location {
            IndexLocation::End => &shard[shard.len() - il..],
            IndexLocation::Start => &shard[..il],
        };
        self.parse_index_section(index_bytes, shard.len())
    }

    /// Parses the index from its own bytes, given the total length of the
    /// shard it belongs to. Partial reads use this to fetch the index alone.
    pub fn parse_index_section(&self, index_bytes: &[u8], shard_len: usize) -> Result<ShardIndex> {
        let il = self.index_len();
        if index_bytes.len() != il || shard_len < il {
            return Err(Error::Codec(
                "sharding: index has the wrong length for this shard".into(),
            ));
        }
        let (data_start, data_end) = match self.location {
            IndexLocation::End => (0, shard_len - il),
            IndexLocation::Start => (il, shard_len),
        };
        let pairs_len = ENTRY_LEN * self.num_inner_chunks();
        let pairs = &index_bytes[..pairs_len];
        if self.index_crc {
            let stored = u32::from_le_bytes([
                index_bytes[pairs_len],
                index_bytes[pairs_len + 1],
                index_bytes[pairs_len + 2],
                index_bytes[pairs_len + 3],
            ]);
            let computed = crc32c(pairs);
            if stored != computed {
                return Err(Error::Codec(format!(
                    "sharding: index checksum mismatch (stored {stored:#010x}, computed {computed:#010x})"
                )));
            }
        }
        let read = |at: usize| {
            let b = &pairs[at..at + 8];
            u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
        };
        let mut entries = Vec::with_capacity(self.num_inner_chunks());
        for n in 0..self.num_inner_chunks() {
            let off = read(ENTRY_LEN * n);
            let len = read(ENTRY_LEN * n + 8);
            if off == MISSING && len == MISSING {
                entries.push(None);
                continue;
            }
            let in_range = off >= data_start as u64
                && off.checked_add(len).is_some_and(|e| e <= data_end as u64);
            if !in_range {
                return Err(Error::Codec(format!(
                    "sharding: inner chunk {n} lies outside the shard data"
                )));
            }
            entries.push(Some((off, len)));
        }
        Ok(ShardIndex { entries })
    }

    /// Decodes only the inner chunks listed in `wanted` (C-order inner
    /// indices). `read(offset, len)` returns the stored bytes of one inner chunk,
    /// so a caller can fetch them with range reads. Other inner chunks, and
    /// missing ones, are left at the fill value. Returns the whole shard in C
    /// order.
    pub fn decode_selected(
        &self,
        index: &ShardIndex,
        wanted: &[usize],
        mut read: impl FnMut(u64, u64) -> Result<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        let mut out = self.fill_chunk();
        let zero = vec![0u64; self.grid.len()];
        for &n in wanted {
            let entry = *index
                .entries()
                .get(n)
                .ok_or_else(|| Error::InvalidSelection(format!("inner chunk {n} out of range")))?;
            let Some((off, len)) = entry else { continue };
            let stored = read(off, len)?;
            let decoded = self.decode_inner(&stored)?;
            if decoded.len() != self.inner_bytes() {
                return Err(Error::SizeMismatch {
                    expected: self.inner_bytes(),
                    actual: decoded.len(),
                });
            }
            let origin: Vec<u64> = self
                .coords(n)
                .iter()
                .zip(&self.inner_shape)
                .map(|(c, i)| c * i)
                .collect();
            copy_region(
                (&decoded, &self.inner_shape, &zero),
                (&mut out, &self.shard_shape, &origin),
                &self.inner_shape,
                self.item_size,
            );
        }
        Ok(out)
    }

    /// C-order indices of the inner chunks touched by a selection. `selections`
    /// has one `(start, step, count)` triple per shard dimension, the same
    /// positions as a chunk piece (offsets within the shard).
    pub fn inner_indices_touched(&self, selections: &[(u64, u64, u64)]) -> Result<Vec<usize>> {
        if selections.len() != self.grid.len() {
            return Err(Error::InvalidSelection(
                "selection rank differs from the shard rank".into(),
            ));
        }
        // Per dimension: the distinct inner positions touched. The touched set is
        // a product over dimensions, because selections are regular slices.
        let mut out: Vec<usize> = vec![0];
        for (d, &(start, step, count)) in selections.iter().enumerate() {
            let mut positions: Vec<u64> = (0..count)
                .map(|k| (start + k * step) / self.inner_shape[d])
                .collect();
            positions.sort_unstable();
            positions.dedup();
            let mut next = Vec::with_capacity(out.len() * positions.len());
            for &base in &out {
                for &c in &positions {
                    next.push(base * self.grid[d] as usize + c as usize);
                }
            }
            out = next;
        }
        out.sort_unstable();
        Ok(out)
    }

    /// Fill-value bytes of a whole shard, for callers that assemble one.
    pub fn fill_shard(&self) -> Vec<u8> {
        self.fill_chunk()
    }

    /// Decodes one inner chunk from `shard`: `Ok(None)` for a missing chunk,
    /// otherwise the chunk's raw element bytes.
    pub fn extract_inner(&self, shard: &[u8], index: usize) -> Result<Option<Vec<u8>>> {
        let idx = self.parse_index(shard)?;
        let entry = *idx
            .entries()
            .get(index)
            .ok_or_else(|| Error::InvalidSelection(format!("inner chunk {index} out of range")))?;
        match entry {
            None => Ok(None),
            Some((off, len)) => {
                let bytes = &shard[off as usize..(off + len) as usize];
                let decoded = self.decode_inner(bytes)?;
                if decoded.len() != self.inner_bytes() {
                    return Err(Error::SizeMismatch {
                        expected: self.inner_bytes(),
                        actual: decoded.len(),
                    });
                }
                Ok(Some(decoded))
            }
        }
    }

    /// Encodes one inner chunk given in C order: the inner transpose (if any)
    /// first, then the inner chain.
    fn encode_inner(&self, bytes: &[u8]) -> Result<Vec<u8>> {
        match &self.inner_transpose {
            Some(t) => self.inner.encode(&t.encode(bytes)?),
            None => self.inner.encode(bytes),
        }
    }

    /// Decodes one stored inner chunk to C-order element bytes: the inner chain,
    /// then the inverse inner transpose (if any).
    fn decode_inner(&self, stored: &[u8]) -> Result<Vec<u8>> {
        let decoded = self.inner.decode(stored)?;
        match &self.inner_transpose {
            Some(t) => t.decode(&decoded),
            None => Ok(decoded),
        }
    }

    /// Fill-value bytes of a whole outer chunk.
    fn fill_chunk(&self) -> Vec<u8> {
        self.fill.repeat(numel(&self.shard_shape))
    }

    /// True if an inner chunk's raw bytes equal the fill value everywhere.
    fn is_fill_only(&self, bytes: &[u8]) -> bool {
        bytes
            .chunks_exact(self.item_size)
            .all(|e| e == self.fill.as_slice())
    }
}

/// Reads `index_codecs` and `index_location` from the configuration.
fn parse_index_options(config: &Value) -> Result<(bool, IndexLocation)> {
    let location = match config.get("index_location").and_then(Value::as_str) {
        None | Some("end") => IndexLocation::End,
        Some("start") => IndexLocation::Start,
        Some(other) => {
            return Err(Error::InvalidMetadata(format!(
                "index_location '{other}' must be 'start' or 'end'"
            )));
        }
    };
    let names: Vec<(&str, &Value)> = match config.get("index_codecs") {
        None | Some(Value::Null) => return Ok((false, location)),
        Some(v) => v
            .as_array()
            .ok_or_else(|| Error::InvalidMetadata("index_codecs must be an array".into()))?
            .iter()
            .map(|c| {
                let name = c
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::InvalidMetadata("index codec needs a name".into()))?;
                Ok((name, c.get("configuration").unwrap_or(&Value::Null)))
            })
            .collect::<Result<_>>()?,
    };
    // Accepted: [bytes], [bytes, crc32c], [crc32c]. Index bytes are little-endian.
    let mut crc = false;
    for (i, (name, cfg)) in names.iter().enumerate() {
        match *name {
            "bytes" if i == 0 => {
                if endian_of(cfg)? != Endian::Little {
                    return Err(Error::Unsupported(
                        "sharding index with big-endian bytes codec".into(),
                    ));
                }
            }
            "crc32c" if i == names.len() - 1 => crc = true,
            other => {
                return Err(Error::Unsupported(format!(
                    "sharding index codec '{other}' in this position"
                )));
            }
        }
    }
    Ok((crc, location))
}

/// Byte order named by a `bytes` codec configuration (little when absent).
fn endian_of(cfg: &Value) -> Result<Endian> {
    match cfg.get("endian").and_then(Value::as_str) {
        None | Some("little") => Ok(Endian::Little),
        Some("big") => Ok(Endian::Big),
        Some(other) => Err(Error::InvalidMetadata(format!("endian '{other}'"))),
    }
}

/// Builds the inner bytes-to-bytes chain from the `codecs` option.
fn build_inner(
    config: &Value,
    inner_shape: &[u64],
    dtype: &DType,
    registry: &Registry,
) -> Result<(Pipeline, Option<TransposeCodec>)> {
    let specs = match config.get("codecs") {
        None | Some(Value::Null) => return Ok((Pipeline::empty(), None)),
        Some(v) => v
            .as_array()
            .ok_or_else(|| Error::InvalidMetadata("sharding codecs must be an array".into()))?,
    };
    let mut codecs: Vec<Box<dyn Codec>> = Vec::new();
    let mut transpose = None;
    // Array-to-array codecs come first in the chain; `past_array_stage` is set
    // by the first codec of a later stage.
    let mut past_array_stage = false;
    for c in specs {
        let name = c
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidMetadata("inner codec needs a name".into()))?;
        let cfg = c.get("configuration").cloned().unwrap_or_else(|| json!({}));
        match name {
            "transpose" => {
                if past_array_stage || transpose.is_some() {
                    return Err(Error::InvalidMetadata(
                        "inner transpose must be the only array-to-array codec, before the bytes codec".into(),
                    ));
                }
                transpose = Some(TransposeCodec::new(&cfg, inner_shape, dtype.item_size())?);
            }
            "bytes" => {
                past_array_stage = true;
                if dtype.item_size() > 1 && endian_of(&cfg)? != dtype.endian() {
                    return Err(Error::Unsupported(
                        "sharding inner bytes codec with a different byte order".into(),
                    ));
                }
            }
            "sharding_indexed" => {
                return Err(Error::Unsupported(format!(
                    "'{name}' inside a sharding inner chain"
                )));
            }
            _ => {
                past_array_stage = true;
                codecs.push(registry.build(name, &cfg, dtype)?);
            }
        }
    }
    Ok((Pipeline::new(codecs), transpose))
}

impl Codec for ShardingCodec {
    fn name(&self) -> &str {
        "sharding_indexed"
    }

    fn encode(&self, input: &[u8]) -> Result<Vec<u8>> {
        let total = numel(&self.shard_shape) * self.item_size;
        if input.len() != total {
            return Err(Error::SizeMismatch {
                expected: total,
                actual: input.len(),
            });
        }
        let inner_numel = self.inner_numel();
        let mut owned: Vec<Option<Vec<u8>>> = Vec::with_capacity(self.num_inner_chunks());
        let zero = vec![0u64; self.grid.len()];
        for n in 0..self.num_inner_chunks() {
            let origin: Vec<u64> = self
                .coords(n)
                .iter()
                .zip(&self.inner_shape)
                .map(|(c, i)| c * i)
                .collect();
            let mut buf = vec![0u8; inner_numel * self.item_size];
            copy_region(
                (input, &self.shard_shape, &origin),
                (&mut buf, &self.inner_shape, &zero),
                &self.inner_shape,
                self.item_size,
            );
            owned.push(if self.is_fill_only(&buf) {
                None
            } else {
                Some(buf)
            });
        }
        let refs: Vec<Option<&[u8]>> = owned.iter().map(|o| o.as_deref()).collect();
        self.encode_shard(&refs)
    }

    fn decode(&self, input: &[u8]) -> Result<Vec<u8>> {
        let idx = self.parse_index(input)?;
        let mut out = self.fill_chunk();
        let zero = vec![0u64; self.grid.len()];
        for (n, entry) in idx.entries().iter().enumerate() {
            let Some((off, len)) = *entry else { continue };
            let decoded = self.decode_inner(&input[off as usize..(off + len) as usize])?;
            if decoded.len() != self.inner_bytes() {
                return Err(Error::SizeMismatch {
                    expected: self.inner_bytes(),
                    actual: decoded.len(),
                });
            }
            let origin: Vec<u64> = self
                .coords(n)
                .iter()
                .zip(&self.inner_shape)
                .map(|(c, i)| c * i)
                .collect();
            copy_region(
                (&decoded, &self.inner_shape, &zero),
                (&mut out, &self.shard_shape, &origin),
                &self.inner_shape,
                self.item_size,
            );
        }
        Ok(out)
    }
}

/// Factory placeholder: sharding needs the chunk shape and fill value, which
/// the registry does not have. Pipelines build it through
/// `ArrayMetadata::pipeline`.
pub(crate) fn sharding_factory(_config: &Value, _dtype: &DType) -> Result<Box<dyn Codec>> {
    Err(Error::Unsupported(
        "sharding_indexed needs the chunk shape; build it through ArrayMetadata::pipeline"
            .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Registry;
    use crate::codec::crc32c::crc32c;

    fn config(index_codecs: Value, location: &str) -> Value {
        json!({
            "chunk_shape": [2, 2],
            "codecs": [{"name": "bytes", "configuration": {"endian": "little"}}],
            "index_codecs": index_codecs,
            "index_location": location,
        })
    }

    fn codec(index_codecs: Value, location: &str, fill: u8) -> ShardingCodec {
        ShardingCodec::new(
            &config(index_codecs, location),
            &[4, 4],
            &DType::from_v3("uint8", Endian::Little).unwrap(),
            &[fill],
            &Registry::builtin(),
        )
        .unwrap()
    }

    fn default_index() -> Value {
        json!([{"name": "bytes", "configuration": {"endian": "little"}}, {"name": "crc32c"}])
    }

    #[test]
    fn two_by_two_inner_chunks_round_trip_and_layout() {
        let c = codec(default_index(), "end", 0);
        // 4x4 row-major values 0..16. Inner chunks in C order:
        // (0,0) = [0,1,4,5], (0,1) = [2,3,6,7], (1,0) = [8,9,12,13], (1,1) = [10,11,14,15].
        let data: Vec<u8> = (0..16u8).collect();
        let enc = c.encode(&data).unwrap();
        let index = c.parse_index(&enc).unwrap();
        assert_eq!(
            index.entries(),
            &[Some((0, 4)), Some((4, 4)), Some((8, 4)), Some((12, 4))]
        );
        assert_eq!(enc.len(), 16 + 4 * 16 + 4);
        assert_eq!(&enc[..4], &[0, 1, 4, 5]);
        assert_eq!(&enc[4..8], &[2, 3, 6, 7]);
        assert_eq!(c.decode(&enc).unwrap(), data);
        assert_eq!(
            c.extract_inner(&enc, 3).unwrap(),
            Some(vec![10, 11, 14, 15])
        );
    }

    #[test]
    fn index_checksum_is_crc32c_of_pairs() {
        let c = codec(default_index(), "end", 0);
        let enc = c.encode(&(0..16u8).collect::<Vec<_>>()).unwrap();
        let pairs = &enc[enc.len() - 68..enc.len() - 4];
        let stored = u32::from_le_bytes(enc[enc.len() - 4..].try_into().unwrap());
        assert_eq!(stored, crc32c(pairs));
        let mut bad = enc.clone();
        let n = bad.len();
        bad[n - 5] ^= 1;
        assert!(c.decode(&bad).is_err());
    }

    #[test]
    fn missing_inner_chunk_reads_as_fill() {
        let c = codec(default_index(), "end", 0xEE);
        let a = [0u8, 1, 4, 5];
        let b = [2u8, 3, 6, 7];
        let d = [10u8, 11, 14, 15];
        let shard = c
            .encode_shard(&[Some(&a[..]), Some(&b[..]), None, Some(&d[..])])
            .unwrap();
        let index = c.parse_index(&shard).unwrap();
        assert_eq!(index.entries()[2], None);
        assert_eq!(c.extract_inner(&shard, 2).unwrap(), None);
        let out = c.decode(&shard).unwrap();
        #[rustfmt::skip]
        let expected = [
            0, 1, 2, 3,
            4, 5, 6, 7,
            0xEE, 0xEE, 10, 11,
            0xEE, 0xEE, 14, 15,
        ];
        assert_eq!(out, expected.to_vec());
    }

    #[test]
    fn fill_only_inner_chunks_are_written_missing() {
        let c = codec(default_index(), "end", 0xEE);
        let mut data = vec![0xEEu8; 16];
        data[0] = 1;
        let shard = c.encode(&data).unwrap();
        let index = c.parse_index(&shard).unwrap();
        assert!(index.entries()[0].is_some());
        assert!(index.entries()[1..].iter().all(Option::is_none));
        assert_eq!(c.decode(&shard).unwrap(), data);
    }

    #[test]
    fn index_at_start_and_without_checksum() {
        let c = codec(json!([{"name": "bytes"}]), "start", 0);
        assert!(!c.index_has_crc32c());
        let data: Vec<u8> = (0..16u8).rev().collect();
        let shard = c.encode(&data).unwrap();
        // The 4 chunks' index (4 x 16 bytes) precedes the data.
        assert_eq!(shard.len(), 64 + 16);
        assert_eq!(c.parse_index(&shard).unwrap().entries()[0], Some((64, 4)));
        assert_eq!(c.decode(&shard).unwrap(), data);
    }

    #[test]
    fn rejects_bad_index_configuration() {
        let reg = Registry::builtin();
        let dt = DType::from_v3("uint8", Endian::Little).unwrap();
        let bad_codecs = [
            json!([{"name": "crc32c"}, {"name": "bytes"}]),
            json!([{"name": "gzip"}]),
            json!([{"name": "bytes", "configuration": {"endian": "big"}}]),
        ];
        for ic in bad_codecs {
            let r = ShardingCodec::new(&config(ic, "end"), &[4, 4], &dt, &[0], &reg);
            assert!(r.is_err());
        }
        let bad_divisor = json!({"chunk_shape": [3, 2]});
        assert!(ShardingCodec::new(&bad_divisor, &[4, 4], &dt, &[0], &reg).is_err());
    }

    #[test]
    fn out_of_range_index_entry_is_rejected() {
        let c = codec(default_index(), "end", 0);
        let mut shard = c.encode(&(0..16u8).collect::<Vec<_>>()).unwrap();
        // Rewrite entry 0 with an offset past the data, then fix the checksum.
        let pairs_end = shard.len() - 4;
        let pairs_start = pairs_end - 16 * 4;
        shard[pairs_start..pairs_start + 8].copy_from_slice(&1000u64.to_le_bytes());
        let sum = crc32c(&shard[pairs_start..pairs_end]).to_le_bytes();
        shard[pairs_end..].copy_from_slice(&sum);
        assert!(c.parse_index(&shard).is_err());
    }
}
