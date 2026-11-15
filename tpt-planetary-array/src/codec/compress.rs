//! Pure-Rust compressors: gzip (RFC 1952), zlib (RFC 1950) and LZ4 block
//! format with a 4-byte size prefix (the layout used by Zarr v2 `lz4`).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;

use serde_json::Value;

use super::{Codec, config_i64};
use crate::dtype::DType;
use crate::error::{Error, Result};

/// Default DEFLATE level used when a configuration omits one.
const DEFAULT_LEVEL: u8 = 1;

fn level_from(config: &Value) -> Result<u8> {
    let level = config_i64(config, "level")?.unwrap_or(i64::from(DEFAULT_LEVEL));
    if !(0..=10).contains(&level) {
        return Err(Error::InvalidMetadata(format!(
            "compression level {level} outside 0..=10"
        )));
    }
    Ok(level as u8)
}

/// CRC-32 (IEEE 802.3, reflected) lookup table, built at compile time.
const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
};

/// Computes the CRC-32 checksum used by the gzip trailer.
pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = CRC_TABLE[((crc ^ u32::from(b)) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

/// zlib (RFC 1950) compression via `miniz_oxide`.
#[derive(Debug, Clone, Copy)]
pub struct ZlibCodec {
    level: u8,
}

impl ZlibCodec {
    /// Creates a zlib codec with the given level (0 to 10).
    pub fn new(level: u8) -> Self {
        Self { level }
    }
}

impl Codec for ZlibCodec {
    fn name(&self) -> &str {
        "zlib"
    }

    fn encode(&self, input: &[u8]) -> Result<Vec<u8>> {
        Ok(miniz_oxide::deflate::compress_to_vec_zlib(
            input, self.level,
        ))
    }

    fn decode(&self, input: &[u8]) -> Result<Vec<u8>> {
        miniz_oxide::inflate::decompress_to_vec_zlib(input)
            .map_err(|e| Error::Codec(format!("zlib: {e:?}")))
    }
}

/// gzip (RFC 1952) compression. Writes a minimal header and CRC-32/ISIZE
/// trailer around a raw DEFLATE stream; reads single-member gzip streams.
#[derive(Debug, Clone, Copy)]
pub struct GzipCodec {
    level: u8,
}

impl GzipCodec {
    /// Creates a gzip codec with the given level (0 to 10).
    pub fn new(level: u8) -> Self {
        Self { level }
    }
}

impl Codec for GzipCodec {
    fn name(&self) -> &str {
        "gzip"
    }

    fn encode(&self, input: &[u8]) -> Result<Vec<u8>> {
        let body = miniz_oxide::deflate::compress_to_vec(input, self.level);
        let mut out = Vec::with_capacity(body.len() + 18);
        // ID1, ID2, CM=deflate, FLG=0, MTIME=0, XFL=0, OS=unknown(255)
        out.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0x00, 0xff]);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(input).to_le_bytes());
        out.extend_from_slice(&(input.len() as u32).to_le_bytes());
        Ok(out)
    }

    fn decode(&self, input: &[u8]) -> Result<Vec<u8>> {
        let err = |m: &str| Error::Codec(format!("gzip: {m}"));
        if input.len() < 18 || input[0] != 0x1f || input[1] != 0x8b || input[2] != 0x08 {
            return Err(err("bad header"));
        }
        let flags = input[3];
        let mut pos = 10usize;
        if flags & 0x04 != 0 {
            // FEXTRA: 2-byte little-endian length followed by data.
            let xlen = usize::from(u16::from_le_bytes([input[pos], input[pos + 1]]));
            pos += 2 + xlen;
        }
        for flag in [0x08u8, 0x10] {
            // FNAME and FCOMMENT are zero-terminated strings.
            if flags & flag != 0 {
                while *input.get(pos).ok_or_else(|| err("truncated header"))? != 0 {
                    pos += 1;
                }
                pos += 1;
            }
        }
        if flags & 0x02 != 0 {
            pos += 2; // FHCRC
        }
        if input.len() < pos + 8 {
            return Err(err("truncated stream"));
        }
        let trailer = input.len() - 8;
        let crc = u32::from_le_bytes([
            input[trailer],
            input[trailer + 1],
            input[trailer + 2],
            input[trailer + 3],
        ]);
        let isize = u32::from_le_bytes([
            input[trailer + 4],
            input[trailer + 5],
            input[trailer + 6],
            input[trailer + 7],
        ]);
        let out = miniz_oxide::inflate::decompress_to_vec(&input[pos..trailer])
            .map_err(|e| err(&format!("inflate failed: {e:?}")))?;
        if crc32(&out) != crc {
            return Err(err("CRC-32 mismatch"));
        }
        if out.len() as u32 != isize {
            return Err(err("size mismatch"));
        }
        Ok(out)
    }
}

/// LZ4 block compression with a little-endian 4-byte uncompressed-size prefix.
#[derive(Debug, Clone, Copy, Default)]
pub struct Lz4Codec;

impl Codec for Lz4Codec {
    fn name(&self) -> &str {
        "lz4"
    }

    fn encode(&self, input: &[u8]) -> Result<Vec<u8>> {
        if input.len() > u32::MAX as usize {
            return Err(Error::Codec("lz4: input larger than 4 GiB".to_string()));
        }
        Ok(lz4_flex::block::compress_prepend_size(input))
    }

    fn decode(&self, input: &[u8]) -> Result<Vec<u8>> {
        lz4_flex::block::decompress_size_prepended(input)
            .map_err(|e| Error::Codec(format!("lz4: {e}")))
    }
}

pub(crate) fn gzip_factory(config: &Value, _dtype: &DType) -> Result<Box<dyn Codec>> {
    Ok(Box::new(GzipCodec::new(level_from(config)?)))
}

pub(crate) fn zlib_factory(config: &Value, _dtype: &DType) -> Result<Box<dyn Codec>> {
    Ok(Box::new(ZlibCodec::new(level_from(config)?)))
}

pub(crate) fn lz4_factory(_config: &Value, _dtype: &DType) -> Result<Box<dyn Codec>> {
    Ok(Box::new(Lz4Codec))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        // Repetitive but not trivially constant, so compressors do real work.
        (0..10_000u32)
            .flat_map(|i| ((i * i) % 251).to_le_bytes())
            .collect()
    }

    #[test]
    fn zlib_round_trip() {
        let c = ZlibCodec::new(6);
        let data = sample();
        let enc = c.encode(&data).unwrap();
        assert!(enc.len() < data.len());
        assert_eq!(c.decode(&enc).unwrap(), data);
    }

    #[test]
    fn gzip_round_trip_and_header() {
        let c = GzipCodec::new(1);
        let data = sample();
        let enc = c.encode(&data).unwrap();
        assert_eq!(&enc[..2], &[0x1f, 0x8b]);
        assert_eq!(c.decode(&enc).unwrap(), data);
    }

    #[test]
    fn gzip_detects_corruption() {
        let c = GzipCodec::new(1);
        let mut enc = c.encode(b"abcdefgh").unwrap();
        let n = enc.len();
        enc[n - 8] ^= 0xff;
        assert!(c.decode(&enc).is_err());
    }

    #[test]
    fn gzip_matches_known_crc() {
        // CRC-32 check value for "123456789".
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn lz4_round_trip() {
        let c = Lz4Codec;
        let data = sample();
        let enc = c.encode(&data).unwrap();
        assert_eq!(&enc[..4], &(data.len() as u32).to_le_bytes());
        assert_eq!(c.decode(&enc).unwrap(), data);
        assert_eq!(c.decode(&c.encode(&[]).unwrap()).unwrap(), Vec::<u8>::new());
    }
}
