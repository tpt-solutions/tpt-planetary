//! Blosc frame decoding (pure Rust), for Zarr v2 `blosc` compressors.
//!
//! Layout as documented in the c-blosc1 format description (`blosc.h`, the
//! "Blosc format" notes in the c-blosc repository). All header integers are
//! little-endian.
//!
//! | offset | size | field                                                   |
//! |-------:|-----:|---------------------------------------------------------|
//! | 0      | 1    | `version` (format version, 1 or 2 accepted)             |
//! | 1      | 1    | `versionlz` (compressor version, informational)         |
//! | 2      | 1    | `flags`: 0x01 byte shuffle, 0x02 memcpyed, 0x04 bit shuffle, 0x10 don't split, bits 5-7 compressor (0 blosclz, 1 lz4, 2 snappy, 3 zlib, 4 zstd) |
//! | 3      | 1    | `typesize` (element size used by the shuffle filters)   |
//! | 4      | 4    | `nbytes`: uncompressed size                             |
//! | 8      | 4    | `blocksize`: uncompressed bytes per block               |
//! | 12     | 4    | `cbytes`: total frame size, header included             |
//! | 16     | 4·nb | block start table: frame offset of each block           |
//!
//! Memcpyed frames (flag 0x02) store the `nbytes` payload directly at offset
//! 16 and have no block table. Otherwise each block is a sequence of streams,
//! each prefixed by its 4-byte compressed size; a stream whose size equals its
//! uncompressed size is stored raw. A block is split into `typesize` streams
//! (one per byte plane) when it is large enough and the don't-split flag is
//! clear; the bytes of the block that do not fill a whole element follow the
//! streams. Decoding inverts the compressor per stream, then the byte or bit
//! shuffle over the whole block.
//!
//! Supported: memcpy, LZ4 (raw LZ4 block, via `lz4_flex`) and zlib (via
//! `miniz_oxide`), zstd (`ruzstd`), blosclz and snappy, with byte and bit
//! shuffle. Encoding is not implemented.
//!
//! Verification: zarr-python fixtures (`tests/zarr_conformance.rs`) cover every
//! back end with single-stream and multi-stream (split) blocks, and read through
//! the whole Zarr path. The blosclz decoder and the don't-split flag (0x10) were
//! fixed against those fixtures. Snappy is checked against the raw-format
//! reference encoder (cramjam) in `codec/testdata`. The far-distance blosclz
//! form (distances above 8191) is checked against a python-blosc frame in
//! `codec/testdata/blosclz_far.blosc`.

use alloc::boxed::Box;
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use serde_json::Value;

use super::Codec;
use super::bitshuffle::{MAX_EXPANSION, bit_unshuffle};
use super::filters::ShuffleCodec;
use crate::dtype::DType;
use crate::error::{Error, Result};

/// Header size in bytes.
pub const HEADER_LEN: usize = 16;

const FLAG_SHUFFLE: u8 = 0x01;
const FLAG_MEMCPYED: u8 = 0x02;
const FLAG_BITSHUFFLE: u8 = 0x04;
const FLAG_DONT_SPLIT: u8 = 0x10;

const COMPRESSOR_BLOSCLZ: u8 = 0;
const COMPRESSOR_LZ4: u8 = 1;
const COMPRESSOR_SNAPPY: u8 = 2;
const COMPRESSOR_ZLIB: u8 = 3;
const COMPRESSOR_ZSTD: u8 = 4;

/// Most streams a block is split into (c-blosc1 `MAX_SPLITS`).
const MAX_SPLITS: usize = 16;
/// Smallest per-stream size, in bytes, for which a block is split.
const MIN_BUFFERSIZE: usize = 128;

fn u32_le(bytes: &[u8], at: usize) -> Result<u32> {
    let s = bytes
        .get(at..at + 4)
        .ok_or_else(|| Error::Codec("blosc: truncated frame".into()))?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Parsed Blosc header fields (see the module documentation for offsets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BloscHeader {
    /// Format version (byte 0).
    pub version: u8,
    /// Compressor version (byte 1).
    pub versionlz: u8,
    /// Flag byte (byte 2), including the compressor code in bits 5-7.
    pub flags: u8,
    /// Element size in bytes (byte 3).
    pub typesize: u8,
    /// Uncompressed size in bytes (bytes 4-7, little-endian).
    pub nbytes: u32,
    /// Block size in bytes (bytes 8-11, little-endian).
    pub blocksize: u32,
    /// Total frame size in bytes, header included (bytes 12-15, little-endian).
    pub cbytes: u32,
}

impl BloscHeader {
    /// Parses the 16-byte header at the start of `frame`.
    pub fn parse(frame: &[u8]) -> Result<Self> {
        if frame.len() < HEADER_LEN {
            return Err(Error::Codec("blosc: frame shorter than header".into()));
        }
        let h = Self {
            version: frame[0],
            versionlz: frame[1],
            flags: frame[2],
            typesize: frame[3],
            nbytes: u32_le(frame, 4)?,
            blocksize: u32_le(frame, 8)?,
            cbytes: u32_le(frame, 12)?,
        };
        if h.version == 0 || h.version > 2 {
            return Err(Error::Unsupported(format!(
                "blosc format version {}",
                h.version
            )));
        }
        if (h.cbytes as usize) < HEADER_LEN || h.cbytes as usize > frame.len() {
            return Err(Error::Codec(format!(
                "blosc: cbytes {} outside 16..={}",
                h.cbytes,
                frame.len()
            )));
        }
        Ok(h)
    }

    /// Compressor code (0 blosclz, 1 lz4, 2 snappy, 3 zlib, 4 zstd).
    pub fn compressor(&self) -> u8 {
        self.flags >> 5
    }
}

/// Decodes a blosclz stream (the FastLZ-derived format used by Blosc) of
/// `expected` uncompressed bytes.
///
/// The first control byte is read with its top three bits cleared (`& 31`).
/// A control byte below 32 starts a literal run of `ctrl + 1` bytes. Otherwise
/// it starts a back-reference: the top three bits give the length (7 means
/// extra length bytes follow, summed while they equal 255), the low five bits
/// are the high part of the distance, and the next byte is the low part. The
/// copied length is the sum plus 2, and the distance is the 13-bit value plus 1.
///
/// Verified against the blosclz streams of zarr-python fixtures (see
/// `tests/zarr_conformance.rs`), and against a python-blosc frame that uses the
/// 16-bit far-distance form for distances beyond 8191.
fn blosclz_decompress(data: &[u8], expected: usize) -> Result<Vec<u8>> {
    let bad = || Error::Codec("blosc: malformed blosclz stream".into());
    let mut out: Vec<u8> = Vec::with_capacity(expected);
    if data.is_empty() {
        return Err(bad());
    }
    let mut ctrl = (data[0] & 31) as usize;
    let mut ip = 1usize;
    loop {
        if ctrl < 32 {
            let n = ctrl + 1;
            let lit = data.get(ip..ip + n).ok_or_else(bad)?;
            out.extend_from_slice(lit);
            ip += n;
        } else {
            let mut len = ctrl >> 5;
            let ofs = (ctrl & 31) << 8;
            if len == 7 {
                loop {
                    let code = *data.get(ip).ok_or_else(bad)? as usize;
                    ip += 1;
                    len += code;
                    if code != 255 {
                        break;
                    }
                }
            }
            let code = *data.get(ip).ok_or_else(bad)? as usize;
            ip += 1;
            len += 2;
            // The 16-bit distance form (c-blosc's MAX_DISTANCE case): the offset is
            // the big-endian pair that follows, and the distance is that value
            // plus 8192.
            let dist = if code == 255 && ofs == 31 << 8 {
                let hi = *data.get(ip).ok_or_else(bad)? as usize;
                let lo = *data.get(ip + 1).ok_or_else(bad)? as usize;
                ip += 2;
                ((hi << 8) | lo) + 8192
            } else {
                ofs + code + 1
            };
            if dist > out.len() {
                return Err(bad());
            }
            let start = out.len() - dist;
            // Copy byte by byte: the source may overlap the output being written.
            for k in 0..len {
                let b = out[start + k];
                out.push(b);
            }
        }
        if out.len() > expected {
            return Err(bad());
        }
        if ip >= data.len() {
            break;
        }
        ctrl = data[ip] as usize;
        ip += 1;
    }
    if out.len() != expected {
        return Err(Error::Codec(format!(
            "blosc: blosclz stream produced {} bytes, expected {expected}",
            out.len()
        )));
    }
    Ok(out)
}

/// Decodes a raw Snappy stream (the block format, no framing) of `expected`
/// uncompressed bytes, as used by Blosc's snappy back end.
///
/// The stream starts with the uncompressed length as a little-endian varint,
/// then tagged elements. The low two bits of a tag select a literal (0) or a
/// copy with a 1-, 2- or 4-byte offset (1, 2, 3). Copies may overlap the output.
fn snappy_decompress(data: &[u8], expected: usize) -> Result<Vec<u8>> {
    let bad = |what: &str| Error::Codec(format!("blosc: malformed snappy stream: {what}"));
    let mut ip = 0usize;
    let mut declared: usize = 0;
    let mut shift = 0u32;
    loop {
        let b = *data.get(ip).ok_or_else(|| bad("truncated length"))?;
        ip += 1;
        if shift > 28 {
            return Err(bad("length too long"));
        }
        declared |= ((b & 0x7f) as usize) << shift;
        shift += 7;
        if b & 0x80 == 0 {
            break;
        }
    }
    if declared != expected {
        return Err(Error::Codec(format!(
            "blosc: snappy stream declares {declared} bytes, expected {expected}"
        )));
    }
    let mut out: Vec<u8> = Vec::with_capacity(expected);
    while ip < data.len() {
        let tag = data[ip];
        ip += 1;
        match tag & 3 {
            0 => {
                let mut len = (tag >> 2) as usize;
                if len >= 60 {
                    let nbytes = len - 59;
                    let mut v = 0usize;
                    for k in 0..nbytes {
                        let b = *data.get(ip + k).ok_or_else(|| bad("truncated literal"))?;
                        v |= (b as usize) << (8 * k);
                    }
                    ip += nbytes;
                    len = v;
                }
                let len = len + 1;
                let lit = data
                    .get(ip..ip + len)
                    .ok_or_else(|| bad("truncated literal"))?;
                out.extend_from_slice(lit);
                ip += len;
            }
            kind => {
                let (len, offset) = match kind {
                    1 => {
                        let b = *data.get(ip).ok_or_else(|| bad("truncated copy"))?;
                        ip += 1;
                        let len = ((tag >> 2) & 7) as usize + 4;
                        let offset = (((tag >> 5) as usize) << 8) | b as usize;
                        (len, offset)
                    }
                    2 => {
                        let b = data.get(ip..ip + 2).ok_or_else(|| bad("truncated copy"))?;
                        ip += 2;
                        (
                            (tag >> 2) as usize + 1,
                            u16::from_le_bytes([b[0], b[1]]) as usize,
                        )
                    }
                    _ => {
                        let b = data.get(ip..ip + 4).ok_or_else(|| bad("truncated copy"))?;
                        ip += 4;
                        (
                            (tag >> 2) as usize + 1,
                            u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize,
                        )
                    }
                };
                if offset == 0 || offset > out.len() {
                    return Err(bad("copy offset outside the output"));
                }
                if out.len() + len > expected {
                    return Err(bad("output longer than declared"));
                }
                let start = out.len() - offset;
                for k in 0..len {
                    let byte = out[start + k];
                    out.push(byte);
                }
            }
        }
    }
    if out.len() != expected {
        return Err(Error::Codec(format!(
            "blosc: snappy stream produced {} bytes, expected {expected}",
            out.len()
        )));
    }
    Ok(out)
}

/// Decodes one stream of `expected` uncompressed bytes.
fn decompress_stream(compressor: u8, data: &[u8], expected: usize) -> Result<Vec<u8>> {
    match compressor {
        COMPRESSOR_LZ4 => {
            let mut buf = vec![0u8; expected];
            let got = lz4_flex::block::decompress_into(data, &mut buf)
                .map_err(|e| Error::Codec(format!("blosc: lz4: {e}")))?;
            if got != expected {
                return Err(Error::Codec(format!(
                    "blosc: lz4 stream produced {got} bytes, expected {expected}"
                )));
            }
            Ok(buf)
        }
        COMPRESSOR_ZLIB => {
            let out = miniz_oxide::inflate::decompress_to_vec_zlib(data)
                .map_err(|e| Error::Codec(format!("blosc: zlib: {e:?}")))?;
            if out.len() != expected {
                return Err(Error::Codec(format!(
                    "blosc: zlib stream produced {} bytes, expected {expected}",
                    out.len()
                )));
            }
            Ok(out)
        }
        COMPRESSOR_ZSTD => {
            let out = super::compress::zstd_decode(data)?;
            if out.len() != expected {
                return Err(Error::Codec(format!(
                    "blosc: zstd stream produced {} bytes, expected {expected}",
                    out.len()
                )));
            }
            Ok(out)
        }
        COMPRESSOR_BLOSCLZ => blosclz_decompress(data, expected),
        COMPRESSOR_SNAPPY => snappy_decompress(data, expected),
        _ => Err(Error::Unsupported(format!(
            "blosc compressor code {compressor}"
        ))),
    }
}

/// Decodes the streams of the block starting at frame offset `start`, which
/// holds `bsize` uncompressed bytes. Returns the block before unshuffling.
fn decode_block(
    frame: &[u8],
    start: usize,
    bsize: usize,
    typesize: usize,
    compressor: u8,
    dont_split: bool,
) -> Result<Vec<u8>> {
    let nsplits = if !dont_split && typesize <= MAX_SPLITS && bsize / typesize >= MIN_BUFFERSIZE {
        typesize
    } else {
        1
    };
    let neblock = bsize / nsplits;
    let leftover = bsize - neblock * nsplits;

    let mut block = Vec::with_capacity(bsize);
    let mut pos = start;
    for _ in 0..nsplits {
        let csize = u32_le(frame, pos)? as usize;
        pos += 4;
        let data = frame
            .get(pos..pos + csize)
            .ok_or_else(|| Error::Codec("blosc: stream extends past frame".into()))?;
        pos += csize;
        if csize == neblock {
            block.extend_from_slice(data);
        } else {
            block.extend_from_slice(&decompress_stream(compressor, data, neblock)?);
        }
    }
    let tail = frame
        .get(pos..pos + leftover)
        .ok_or_else(|| Error::Codec("blosc: block tail extends past frame".into()))?;
    block.extend_from_slice(tail);
    Ok(block)
}

/// Decodes a complete Blosc frame.
pub fn decode_frame(input: &[u8]) -> Result<Vec<u8>> {
    let h = BloscHeader::parse(input)?;
    let cbytes = h.cbytes as usize;
    let frame = &input[..cbytes];
    let nbytes = h.nbytes as usize;
    if nbytes == 0 {
        return Ok(Vec::new());
    }
    if h.flags & FLAG_MEMCPYED != 0 {
        if cbytes != HEADER_LEN + nbytes {
            return Err(Error::Codec(format!(
                "blosc: memcpyed frame has cbytes {cbytes}, expected {}",
                HEADER_LEN + nbytes
            )));
        }
        return Ok(frame[HEADER_LEN..].to_vec());
    }
    let compressor = h.compressor();
    match compressor {
        COMPRESSOR_LZ4 | COMPRESSOR_ZLIB | COMPRESSOR_BLOSCLZ | COMPRESSOR_ZSTD
        | COMPRESSOR_SNAPPY => {}
        other => {
            return Err(Error::InvalidMetadata(format!(
                "blosc compressor code {other}"
            )));
        }
    }
    if h.flags & FLAG_SHUFFLE != 0 && h.flags & FLAG_BITSHUFFLE != 0 {
        return Err(Error::InvalidMetadata(
            "blosc frame sets both byte and bit shuffle".into(),
        ));
    }
    let typesize = h.typesize as usize;
    if typesize == 0 {
        return Err(Error::InvalidMetadata("blosc typesize is zero".into()));
    }
    let blocksize = h.blocksize as usize;
    if blocksize == 0 {
        return Err(Error::InvalidMetadata("blosc blocksize is zero".into()));
    }
    if nbytes > cbytes.saturating_mul(MAX_EXPANSION) {
        return Err(Error::Codec(
            "blosc: nbytes is implausible for the frame size".into(),
        ));
    }

    let nblocks = nbytes.div_ceil(blocksize);
    if HEADER_LEN + 4 * nblocks > cbytes {
        return Err(Error::Codec("blosc: block table extends past frame".into()));
    }
    let dont_split = h.flags & FLAG_DONT_SPLIT != 0;
    let mut out = Vec::with_capacity(nbytes);
    for j in 0..nblocks {
        let start = u32_le(frame, HEADER_LEN + 4 * j)? as usize;
        if start < HEADER_LEN + 4 * nblocks || start >= cbytes {
            return Err(Error::Codec(format!(
                "blosc: block {j} start {start} invalid"
            )));
        }
        let bsize = blocksize.min(nbytes - j * blocksize);
        let block = decode_block(frame, start, bsize, typesize, compressor, dont_split)?;
        if h.flags & FLAG_BITSHUFFLE != 0 {
            out.extend_from_slice(&bit_unshuffle(&block, typesize));
        } else if h.flags & FLAG_SHUFFLE != 0 {
            out.extend_from_slice(&ShuffleCodec::new(typesize)?.decode(&block)?);
        } else {
            out.extend_from_slice(&block);
        }
    }
    if out.len() != nbytes {
        return Err(Error::Codec(format!(
            "blosc: decoded {} bytes, header says {nbytes}",
            out.len()
        )));
    }
    Ok(out)
}

/// Shuffle mode of a Blosc frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BloscShuffle {
    /// No shuffle.
    None,
    /// Byte shuffle (`shuffle = 1` in Zarr metadata).
    Byte,
    /// Bit shuffle (`shuffle = 2`).
    Bit,
}

/// Blosc codec. Decodes every back end that [`decode_frame`] supports, and
/// encodes LZ4, zlib and zstd frames as one block, split per byte plane the way
/// c-blosc does for blocks of at least 128 elements per plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BloscCodec {
    /// Compressor code to write, or `None` for back ends that are decode-only.
    compressor: Option<u8>,
    clevel: u8,
    shuffle: BloscShuffle,
    typesize: usize,
}

/// Compressor code for a Blosc `cname`. `None` for back ends that decode but
/// do not encode (blosclz, snappy). Unknown names are an error.
fn compressor_code(name: &str) -> Result<Option<u8>> {
    match name {
        "lz4" => Ok(Some(COMPRESSOR_LZ4)),
        "zlib" => Ok(Some(COMPRESSOR_ZLIB)),
        "zstd" => Ok(Some(COMPRESSOR_ZSTD)),
        "blosclz" | "snappy" => Ok(None),
        "lz4hc" => Ok(Some(COMPRESSOR_LZ4)),
        other => Err(Error::InvalidMetadata(format!(
            "unknown blosc cname '{other}'"
        ))),
    }
}

/// Compresses one stream with the given compressor code.
fn compress_stream(compressor: u8, clevel: u8, data: &[u8]) -> Result<Vec<u8>> {
    match compressor {
        COMPRESSOR_LZ4 => Ok(lz4_flex::block::compress(data)),
        COMPRESSOR_ZLIB => Ok(miniz_oxide::deflate::compress_to_vec_zlib(
            data,
            clevel.clamp(1, 9),
        )),
        COMPRESSOR_ZSTD => {
            let mut out = Vec::new();
            ruzstd::encoding::compress(data, &mut out, ruzstd::encoding::CompressionLevel::Fastest);
            Ok(out)
        }
        other => Err(Error::Unsupported(format!("blosc encode code {other}"))),
    }
}

impl BloscCodec {
    /// Creates a Blosc codec that encodes with back end `cname` (`lz4`, `zlib`
    /// or `zstd`; `blosclz` and `snappy` are decode-only), compression level
    /// `clevel` (0 to 9), the given shuffle, and element size `typesize` in bytes.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown `cname` or a `clevel` above 9.
    pub fn new(cname: &str, clevel: u8, shuffle: BloscShuffle, typesize: usize) -> Result<Self> {
        if clevel > 9 {
            return Err(Error::InvalidMetadata(format!(
                "blosc clevel {clevel} exceeds 9"
            )));
        }
        Ok(Self {
            compressor: compressor_code(cname)?,
            clevel,
            shuffle,
            typesize,
        })
    }

    /// Encodes `input` as one Blosc frame (a single block, so `blocksize` is the
    /// input length). Falls back to a memcpy frame when compression does not
    /// make the data smaller.
    fn encode_frame(&self, input: &[u8]) -> Result<Vec<u8>> {
        let nbytes = input.len();
        let nbytes_u32 = u32::try_from(nbytes)
            .map_err(|_| Error::Unsupported("blosc encode of more than 4 GiB".into()))?;
        let typesize = self.typesize.clamp(1, 255);
        let mut flags = match self.shuffle {
            BloscShuffle::None => 0,
            BloscShuffle::Byte => FLAG_SHUFFLE,
            BloscShuffle::Bit => FLAG_BITSHUFFLE,
        };
        let compressor = self.compressor.ok_or_else(|| {
            Error::Unsupported(
                "blosc encode supports lz4, zlib and zstd (blosclz and snappy decode only)".into(),
            )
        })?;
        flags |= compressor << 5;
        let header = |flags: u8, cbytes: usize| -> Vec<u8> {
            let mut h = vec![2u8, 1, flags, typesize as u8];
            h.extend_from_slice(&nbytes_u32.to_le_bytes());
            h.extend_from_slice(&nbytes_u32.to_le_bytes());
            h.extend_from_slice(&(cbytes as u32).to_le_bytes());
            h
        };
        if nbytes == 0 {
            return Ok(header(flags, HEADER_LEN));
        }

        let prepared = match self.shuffle {
            BloscShuffle::None => input.to_vec(),
            BloscShuffle::Byte => ShuffleCodec::new(typesize)?.encode(input)?,
            BloscShuffle::Bit => super::bitshuffle::bit_shuffle(input, typesize),
        };
        // One block holds the whole input, so the split rule of the decoder applies to nbytes.
        let nsplits = if typesize <= MAX_SPLITS && nbytes / typesize >= MIN_BUFFERSIZE {
            typesize
        } else {
            1
        };
        let neblock = nbytes / nsplits;
        let mut block = Vec::with_capacity(nbytes + 4 * nsplits);
        for part in 0..nsplits {
            let chunk = &prepared[part * neblock..(part + 1) * neblock];
            let packed = compress_stream(compressor, self.clevel, chunk)?;
            let stream: &[u8] = if packed.len() >= neblock {
                chunk
            } else {
                &packed
            };
            block.extend_from_slice(&(stream.len() as u32).to_le_bytes());
            block.extend_from_slice(stream);
        }
        block.extend_from_slice(&prepared[nsplits * neblock..]);

        let table_end = HEADER_LEN + 4;
        let cbytes = table_end + block.len();
        if cbytes >= HEADER_LEN + nbytes {
            // Incompressible: store the input as it is.
            let mut out = header(flags | FLAG_MEMCPYED, HEADER_LEN + nbytes);
            out.extend_from_slice(input);
            return Ok(out);
        }
        let mut out = header(flags, cbytes);
        out.extend_from_slice(&(table_end as u32).to_le_bytes());
        out.extend_from_slice(&block);
        Ok(out)
    }
}

impl Codec for BloscCodec {
    fn name(&self) -> &str {
        "blosc"
    }

    fn encode(&self, input: &[u8]) -> Result<Vec<u8>> {
        self.encode_frame(input)
    }

    fn decode(&self, input: &[u8]) -> Result<Vec<u8>> {
        decode_frame(input)
    }
}

/// Builds the codec from a Zarr `blosc` compressor object: `cname` (`lz4`,
/// `zlib` or `zstd` for encoding), `clevel` (default 5), and `shuffle` (0 none,
/// 1 byte, 2 bit, or the names `noshuffle`, `shuffle`, `bitshuffle`). The
/// element size comes from the data type.
pub(crate) fn blosc_factory(config: &Value, dtype: &DType) -> Result<Box<dyn Codec>> {
    let cname = config.get("cname").and_then(Value::as_str).unwrap_or("lz4");
    let compressor = compressor_code(cname)?;
    let clevel = config
        .get("clevel")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .min(9) as u8;
    let shuffle = match config.get("shuffle") {
        None | Some(Value::Null) => BloscShuffle::Byte,
        Some(Value::Number(n)) => match n.as_u64() {
            Some(0) => BloscShuffle::None,
            Some(1) => BloscShuffle::Byte,
            Some(2) => BloscShuffle::Bit,
            _ => {
                return Err(Error::InvalidMetadata(
                    "blosc shuffle must be 0, 1 or 2".into(),
                ));
            }
        },
        Some(Value::String(s)) => match s.as_str() {
            "noshuffle" => BloscShuffle::None,
            "shuffle" => BloscShuffle::Byte,
            "bitshuffle" => BloscShuffle::Bit,
            other => {
                return Err(Error::InvalidMetadata(format!(
                    "unknown blosc shuffle '{other}'"
                )));
            }
        },
        Some(_) => {
            return Err(Error::InvalidMetadata(
                "blosc shuffle has the wrong type".into(),
            ));
        }
    };
    Ok(Box::new(BloscCodec {
        compressor,
        clevel,
        shuffle,
        typesize: dtype.item_size(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a frame from a header and already-encoded block payloads.
    /// Each payload is the full block area (stream size prefixes included).
    fn frame(flags: u8, typesize: u8, nbytes: u32, blocksize: u32, blocks: &[Vec<u8>]) -> Vec<u8> {
        let table_end = HEADER_LEN + 4 * blocks.len();
        let mut starts = Vec::new();
        let mut body = Vec::new();
        for b in blocks {
            starts.push((table_end + body.len()) as u32);
            body.extend_from_slice(b);
        }
        let cbytes = (table_end + body.len()) as u32;
        let mut out = vec![2u8, 1, flags, typesize];
        out.extend_from_slice(&nbytes.to_le_bytes());
        out.extend_from_slice(&blocksize.to_le_bytes());
        out.extend_from_slice(&cbytes.to_le_bytes());
        for s in starts {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out.extend_from_slice(&body);
        out
    }

    /// One stream: 4-byte LE size, then the data.
    fn stream(data: &[u8]) -> Vec<u8> {
        let mut v = (data.len() as u32).to_le_bytes().to_vec();
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn header_offsets() {
        let f = frame(0x22, 4, 8, 8, &[stream(&[0; 8])]);
        let h = BloscHeader::parse(&f).unwrap();
        assert_eq!(h.version, 2);
        assert_eq!(h.versionlz, 1);
        assert_eq!(h.flags, 0x22);
        assert_eq!(h.compressor(), COMPRESSOR_LZ4);
        assert_eq!(h.typesize, 4);
        assert_eq!(h.nbytes, 8);
        assert_eq!(h.blocksize, 8);
        assert_eq!(h.cbytes as usize, f.len());
    }

    #[test]
    fn memcpy_frame() {
        // Flag 0x02 (memcpyed) with the LZ4 compressor bits set.
        let payload = [9u8, 8, 7, 6, 5, 4, 3, 2];
        let mut f = vec![2u8, 1, 0x22, 4];
        f.extend_from_slice(&8u32.to_le_bytes());
        f.extend_from_slice(&8u32.to_le_bytes());
        f.extend_from_slice(&24u32.to_le_bytes());
        f.extend_from_slice(&payload);
        assert_eq!(decode_frame(&f).unwrap(), payload.to_vec());
    }

    #[test]
    fn byte_shuffled_raw_frame() {
        // Two float32-sized elements, typesize 4, 8 bytes in one block.
        // Elements: [01 02 03 04] and [11 12 13 14]. Byte shuffle groups byte
        // plane 0, then 1, 2, 3: [01 11 02 12 03 13 04 14]. The block holds
        // 2 elements so it is not split; the stream equals its size, so it is
        // stored raw.
        let shuffled = [0x01u8, 0x11, 0x02, 0x12, 0x03, 0x13, 0x04, 0x14];
        let f = frame(0x21, 4, 8, 8, &[stream(&shuffled)]);
        assert_eq!(
            decode_frame(&f).unwrap(),
            vec![0x01, 0x02, 0x03, 0x04, 0x11, 0x12, 0x13, 0x14]
        );
    }

    #[test]
    fn bit_shuffled_raw_frame() {
        // Eight 1-byte elements 0..=7 bit-shuffled to [AA CC F0 00 .. 00]
        // (see bitshuffle tests); flag 0x04 selects bit unshuffle.
        let shuffled = [0xAAu8, 0xCC, 0xF0, 0, 0, 0, 0, 0];
        let f = frame(0x04 | 0x20, 1, 8, 8, &[stream(&shuffled)]);
        assert_eq!(decode_frame(&f).unwrap(), (0u8..8).collect::<Vec<u8>>());
    }

    #[test]
    fn lz4_frame_with_byte_shuffle_and_two_blocks() {
        // 64 bytes of typesize 4 in blocks of 40: block 0 is 40 bytes, block 1
        // is 24 bytes. Blocks are below the split threshold (40/4 < 128), so
        // each is one LZ4 stream.
        let data: Vec<u8> = (0..64u32).map(|i| (i * 7 % 13) as u8).collect();
        // Each block is shuffled on its own, as the encoder does.
        let mut blocks = Vec::new();
        for chunk in data.chunks(40) {
            let sh = ShuffleCodec::new(4).unwrap().encode(chunk).unwrap();
            let comp = lz4_flex::block::compress(&sh);
            blocks.push(if comp.len() < sh.len() {
                stream(&comp)
            } else {
                stream(&sh)
            });
        }
        let f = frame(0x01 | 0x20, 4, 64, 40, &blocks);
        assert_eq!(decode_frame(&f).unwrap(), data);
    }

    #[test]
    fn lz4_stream_compressed_below_raw_size() {
        let data = vec![0x5Au8; 512];
        let comp = lz4_flex::block::compress(&data);
        assert!(comp.len() < data.len());
        let f = frame(0x20, 1, 512, 512, &[stream(&comp)]);
        assert_eq!(decode_frame(&f).unwrap(), data);
    }

    #[test]
    fn zlib_frame() {
        let data: Vec<u8> = (0..300u32).map(|i| (i % 17) as u8).collect();
        let comp = miniz_oxide::deflate::compress_to_vec_zlib(&data, 6);
        let f = frame(0x03 << 5, 1, 300, 300, &[stream(&comp)]);
        assert_eq!(decode_frame(&f).unwrap(), data);
    }

    #[test]
    fn split_block_streams_per_byte_plane() {
        // typesize 2, 256 bytes = 128 elements, so the block splits into two
        // streams of 128 bytes (byte planes). Each stream is stored raw.
        let data: Vec<u8> = (0..256u32).map(|i| (i % 256) as u8).collect();
        let shuffled = ShuffleCodec::new(2).unwrap().encode(&data).unwrap();
        let mut block = Vec::new();
        block.extend(stream(&shuffled[..128]));
        block.extend(stream(&shuffled[128..]));
        let f = frame(0x01 | 0x20, 2, 256, 256, &[block]);
        assert_eq!(decode_frame(&f).unwrap(), data);
    }

    #[test]
    fn snappy_raw_stream_matches_reference_encoder() {
        let raw = include_bytes!("testdata/snappy_cramjam.raw");
        let packed = include_bytes!("testdata/snappy_cramjam.bin");
        assert_eq!(snappy_decompress(packed, raw.len()).unwrap(), raw.to_vec());
    }

    #[test]
    fn snappy_copy_element_kinds() {
        // Literal "abcd", then a copy of length 4 at offset 4, once per copy kind.
        let copy1 = [8u8, 0x0c, b'a', b'b', b'c', b'd', 0x01, 0x04];
        let copy2 = [8u8, 0x0c, b'a', b'b', b'c', b'd', 0x0e, 0x04, 0x00];
        let copy4 = [
            8u8, 0x0c, b'a', b'b', b'c', b'd', 0x0f, 0x04, 0x00, 0x00, 0x00,
        ];
        for packed in [&copy1[..], &copy2[..], &copy4[..]] {
            assert_eq!(snappy_decompress(packed, 8).unwrap(), b"abcdabcd".to_vec());
        }
        // Overlapping copy: "a" then length 7 at offset 1 gives eight a's.
        let overlap = [8u8, 0x00, b'a', 0x0d, 0x01];
        assert_eq!(snappy_decompress(&overlap, 8).unwrap(), vec![b'a'; 8]);
        // Offsets before the start and length mismatches are errors.
        assert!(snappy_decompress(&[4u8, 0x00, b'a', 0x01, 0x05], 4).is_err());
        assert!(snappy_decompress(&copy2, 9).is_err());
    }

    #[test]
    fn snappy_frame_decodes_through_blosc() {
        let raw = include_bytes!("testdata/snappy_cramjam.raw");
        let packed = include_bytes!("testdata/snappy_cramjam.bin");
        let f = frame(
            COMPRESSOR_SNAPPY << 5,
            1,
            raw.len() as u32,
            raw.len() as u32,
            &[stream(packed)],
        );
        assert_eq!(decode_frame(&f).unwrap(), raw.to_vec());
    }

    #[test]
    fn unsupported_compressors_are_rejected() {
        // A blosclz literal run: control byte 3 copies four bytes.
        let f = frame(
            COMPRESSOR_BLOSCLZ << 5,
            1,
            4,
            4,
            &[stream(&[3, 1, 2, 3, 4])],
        );
        assert_eq!(decode_frame(&f).unwrap(), vec![1, 2, 3, 4]);
        let f = frame(7 << 5, 1, 4, 4, &[stream(&[1, 2, 3, 4])]);
        assert!(matches!(decode_frame(&f), Err(Error::InvalidMetadata(_))));
    }

    #[test]
    fn rejects_bad_headers() {
        assert!(decode_frame(&[2u8; 8]).is_err());
        let mut f = frame(0x20, 1, 4, 4, &[stream(&[1, 2, 3, 4])]);
        f[0] = 9;
        assert!(matches!(decode_frame(&f), Err(Error::Unsupported(_))));
        let mut f = frame(0x20, 1, 4, 4, &[stream(&[1, 2, 3, 4])]);
        f[12..16].copy_from_slice(&1000u32.to_le_bytes());
        assert!(decode_frame(&f).is_err());
        let f = frame(0x21 | 0x04, 1, 4, 4, &[stream(&[1, 2, 3, 4])]);
        assert!(decode_frame(&f).is_err(), "byte and bit shuffle together");
    }

    fn codec(cname: &str, shuffle: BloscShuffle, typesize: usize) -> BloscCodec {
        BloscCodec {
            compressor: compressor_code(cname).unwrap(),
            clevel: 5,
            shuffle,
            typesize,
        }
    }

    #[test]
    fn constructor_rejects_bad_names_and_levels() {
        assert!(BloscCodec::new("lz4", 9, BloscShuffle::Byte, 8).is_ok());
        assert!(BloscCodec::new("nope", 5, BloscShuffle::None, 1).is_err());
        assert!(BloscCodec::new("lz4", 10, BloscShuffle::None, 1).is_err());
        // Decode-only back ends are accepted as names but cannot encode.
        let snappy = BloscCodec::new("snappy", 5, BloscShuffle::None, 1).unwrap();
        assert!(snappy.encode(&[0u8; 64]).is_err());
    }

    #[test]
    fn encode_round_trips_every_compressor_and_shuffle() {
        // Compressible float-like data (8-byte elements, slowly varying), 4096 elements.
        let data: Vec<u8> = (0..4096u32)
            .flat_map(|i| (i as f64 * 0.25).to_le_bytes())
            .collect();
        for cname in ["lz4", "zlib", "zstd"] {
            for shuffle in [BloscShuffle::None, BloscShuffle::Byte, BloscShuffle::Bit] {
                let c = codec(cname, shuffle, 8);
                let frame = c.encode(&data).unwrap();
                assert!(
                    frame.len() < data.len(),
                    "{cname} {shuffle:?} did not compress"
                );
                assert_eq!(decode_frame(&frame).unwrap(), data, "{cname} {shuffle:?}");
            }
        }
    }

    #[test]
    fn encode_falls_back_to_memcpy_and_handles_tails_and_empty_input() {
        // Pseudo-random bytes do not compress: the frame is a memcpy frame.
        let mut x: u32 = 0x1234_5678;
        let noise: Vec<u8> = (0..999)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        let frame = codec("lz4", BloscShuffle::Byte, 4).encode(&noise).unwrap();
        assert_ne!(frame[2] & FLAG_MEMCPYED, 0);
        assert_eq!(decode_frame(&frame).unwrap(), noise);
        // Length not a multiple of the element size: the tail is stored unchanged.
        let odd: Vec<u8> = (0..1001u32).map(|i| (i / 3) as u8).collect();
        let frame = codec("zlib", BloscShuffle::Byte, 8).encode(&odd).unwrap();
        assert_eq!(decode_frame(&frame).unwrap(), odd);
        // Empty input is a header-only frame that decodes to nothing.
        let frame = codec("lz4", BloscShuffle::None, 8).encode(&[]).unwrap();
        assert_eq!(frame.len(), HEADER_LEN);
        assert!(decode_frame(&frame).unwrap().is_empty());
    }

    #[test]
    fn encode_rejects_back_ends_it_does_not_write() {
        let cfg = serde_json::json!({"cname": "blosclz"});
        let dtype = DType::FLOAT64;
        // blosclz is accepted for reading; writing it is refused.
        let c = blosc_factory(&cfg, &dtype).unwrap();
        assert!(matches!(c.encode(&[1, 2, 3]), Err(Error::Unsupported(_))));
        let cfg = serde_json::json!({"cname": "nope"});
        assert!(matches!(
            blosc_factory(&cfg, &dtype),
            Err(Error::InvalidMetadata(_))
        ));
    }

    #[test]
    fn factory_reads_zarr_compressor_options() {
        let dtype = DType::FLOAT64;
        let cfg = serde_json::json!({"id": "blosc", "cname": "zstd", "clevel": 3, "shuffle": 2});
        let c = blosc_factory(&cfg, &dtype).unwrap();
        let data = vec![7u8; 2048];
        assert_eq!(decode_frame(&c.encode(&data).unwrap()).unwrap(), data);
        let cfg = serde_json::json!({"cname": "lz4", "shuffle": "noshuffle"});
        assert!(blosc_factory(&cfg, &dtype).is_ok());
        let cfg = serde_json::json!({"cname": "lz4", "shuffle": 9});
        assert!(blosc_factory(&cfg, &dtype).is_err());
    }
}

#[cfg(test)]
mod numcodecs_reference_tests {
    //! Decoding of frames written by the reference Blosc library through
    //! numcodecs 0.17.1 (`Blosc` codec), with the raw data they encode.
    use super::*;

    fn check(frame: &[u8], raw: &[u8]) {
        assert_eq!(decode_frame(frame).unwrap().as_slice(), raw);
    }

    #[test]
    fn decodes_lz4_without_shuffle() {
        check(
            include_bytes!("testdata/blosc_lz4_noshuffle.blosc"),
            include_bytes!("testdata/blosc_lz4_noshuffle.raw"),
        );
    }

    #[test]
    fn decodes_lz4_with_byte_shuffle() {
        check(
            include_bytes!("testdata/blosc_lz4_byteshuffle.blosc"),
            include_bytes!("testdata/blosc_lz4_byteshuffle.raw"),
        );
    }

    #[test]
    fn decodes_lz4_with_bit_shuffle() {
        check(
            include_bytes!("testdata/blosc_lz4_bitshuffle.blosc"),
            include_bytes!("testdata/blosc_lz4_bitshuffle.raw"),
        );
    }

    #[test]
    fn decodes_blosclz_far_distance_frame() {
        // python-blosc output whose second half is matched 12600 bytes back, which
        // needs the 16-bit distance form.
        check(
            include_bytes!("testdata/blosclz_far.blosc"),
            include_bytes!("testdata/blosclz_far.raw"),
        );
    }

    #[test]
    fn decodes_memcpy_frame() {
        check(
            include_bytes!("testdata/blosc_memcpy.blosc"),
            include_bytes!("testdata/blosc_memcpy.raw"),
        );
    }

    #[test]
    fn decodes_zlib_with_byte_shuffle() {
        check(
            include_bytes!("testdata/blosc_zlib_byteshuffle.blosc"),
            include_bytes!("testdata/blosc_zlib_byteshuffle.raw"),
        );
    }
}
