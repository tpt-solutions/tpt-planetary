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
//! | 2      | 1    | `flags`: 0x01 byte shuffle, 0x02 memcpyed, 0x04 bit shuffle, 0x08 don't split, bits 5-7 compressor (0 blosclz, 1 lz4, 2 snappy, 3 zlib, 4 zstd) |
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
//! `miniz_oxide`), with byte and bit shuffle. Rejected with
//! [`Error::Unsupported`]: blosclz, snappy and zstd streams. Encoding is not
//! implemented.
//!
//! The split rule and the tail-bytes placement are taken from the c-blosc1
//! sources as remembered; no reference frames were available to check them, so
//! the test frames are built from the layout above.

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
const FLAG_DONT_SPLIT: u8 = 0x08;

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
        _ => Err(Error::Unsupported(format!(
            "blosc compressor code {compressor} (only lz4 and zlib are decoded)"
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
        COMPRESSOR_LZ4 | COMPRESSOR_ZLIB => {}
        COMPRESSOR_BLOSCLZ => {
            return Err(Error::Unsupported("blosc blosclz compressor".into()));
        }
        COMPRESSOR_SNAPPY => return Err(Error::Unsupported("blosc snappy compressor".into())),
        COMPRESSOR_ZSTD => return Err(Error::Unsupported("blosc zstd compressor".into())),
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

/// The Zarr v2 `blosc` compressor. Decoding only.
#[derive(Debug, Clone, Copy, Default)]
pub struct BloscCodec;

impl Codec for BloscCodec {
    fn name(&self) -> &str {
        "blosc"
    }

    fn encode(&self, _input: &[u8]) -> Result<Vec<u8>> {
        Err(Error::Unsupported(
            "blosc encode is not implemented (decode only)".into(),
        ))
    }

    fn decode(&self, input: &[u8]) -> Result<Vec<u8>> {
        decode_frame(input)
    }
}

pub(crate) fn blosc_factory(_config: &Value, _dtype: &DType) -> Result<Box<dyn Codec>> {
    Ok(Box::new(BloscCodec))
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
    fn unsupported_compressors_are_rejected() {
        for code in [COMPRESSOR_BLOSCLZ, COMPRESSOR_SNAPPY, COMPRESSOR_ZSTD] {
            let f = frame(code << 5, 1, 4, 4, &[stream(&[1, 2, 3, 4])]);
            assert!(matches!(decode_frame(&f), Err(Error::Unsupported(_))));
        }
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

    #[test]
    fn encode_is_unsupported() {
        assert!(matches!(
            BloscCodec.encode(&[1]),
            Err(Error::Unsupported(_))
        ));
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
