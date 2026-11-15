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

/// Zstandard level that the encoder accepts (Zarr `level` 1, the numcodecs default).
/// ruzstd only implements its "Fastest" mode, which it describes as roughly level 1.
const ZSTD_ENCODE_LEVEL: i64 = 1;

/// Zstandard via `ruzstd` (MIT, pure Rust, `no_std` with `alloc`).
///
/// Decoding accepts one or more concatenated frames, as produced by the zstd
/// CLI and numcodecs. Content checksums are verified (ruzstd `hash` feature,
/// which adds `twox-hash`, MIT), so a corrupted frame is an error.
///
/// Encoding works only at level 1. Any other level returns
/// [`Error::Unsupported`]: ruzstd panics for its unimplemented levels, so they
/// are rejected before reaching the encoder.
#[derive(Debug, Clone, Copy)]
pub struct ZstdCodec {
    level: i64,
}

impl ZstdCodec {
    /// Creates a zstd codec. `level` is recorded for metadata; only level 1
    /// can be used to encode.
    pub fn new(level: i64) -> Self {
        Self { level }
    }

    /// The configured compression level.
    pub fn level(&self) -> i64 {
        self.level
    }
}

impl Codec for ZstdCodec {
    fn name(&self) -> &str {
        "zstd"
    }

    fn encode(&self, input: &[u8]) -> Result<Vec<u8>> {
        if self.level != ZSTD_ENCODE_LEVEL {
            return Err(Error::Unsupported(format!(
                "zstd encode at level {}: only level {ZSTD_ENCODE_LEVEL} is supported",
                self.level
            )));
        }
        let mut out = Vec::new();
        ruzstd::encoding::compress(input, &mut out, ruzstd::encoding::CompressionLevel::Fastest);
        Ok(out)
    }

    fn decode(&self, input: &[u8]) -> Result<Vec<u8>> {
        use ruzstd::io::Read;

        let mut out = Vec::new();
        let mut rest = input;
        loop {
            let mut decoder = ruzstd::decoding::StreamingDecoder::new(rest)
                .map_err(|e| Error::Codec(format!("zstd: {e}")))?;
            decoder
                .read_to_end(&mut out)
                .map_err(|e| Error::Codec(format!("zstd: {e}")))?;
            let (remaining, frame) = decoder.into_parts();
            // ruzstd reports the stored and recomputed checksums but does not
            // compare them, so a mismatch is checked here.
            if let (Some(stored), Some(calculated)) = (
                frame.get_checksum_from_data(),
                frame.get_calculated_checksum(),
            ) && stored != calculated
            {
                return Err(Error::Codec(format!(
                    "zstd: content checksum mismatch (stored {stored:#010x}, computed {calculated:#010x})"
                )));
            }
            rest = remaining;
            if rest.is_empty() {
                return Ok(out);
            }
        }
    }
}

pub(crate) fn zstd_factory(config: &Value, _dtype: &DType) -> Result<Box<dyn Codec>> {
    let level = config_i64(config, "level")?.unwrap_or(ZSTD_ENCODE_LEVEL);
    // Range accepted by the zstd library (negative levels are fast modes).
    if !(-131_072..=22).contains(&level) {
        return Err(Error::InvalidMetadata(format!(
            "zstd level {level} outside -131072..=22"
        )));
    }
    Ok(Box::new(ZstdCodec::new(level)))
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

    /// Plaintext of the zstd reference streams: 4000 bytes, `(i*i) % 251`.
    fn zstd_plain() -> Vec<u8> {
        (0..4000u32).map(|i| ((i * i) % 251) as u8).collect()
    }

    // Reference streams made with the zstd CLI v1.5.7 (zstd.exe, Poppler build).
    // Commands: `zstd -1` on bytes 0..2000 (first half), `zstd -19` on bytes
    // 2000..4000 (second half), `zstd -3 --no-check` on all 4000 bytes, and the
    // first and second frames concatenated.
    const ZSTD_L1_FIRST_HALF: &[u8] =
        include_bytes!("testdata/zstd_cli_l1_checksum_first_half.zst");
    const ZSTD_L19_SECOND_HALF: &[u8] =
        include_bytes!("testdata/zstd_cli_l19_checksum_second_half.zst");
    const ZSTD_L3_FULL_NO_CHECKSUM: &[u8] =
        include_bytes!("testdata/zstd_cli_l3_no_checksum_full.zst");
    const ZSTD_CONCATENATED: &[u8] = include_bytes!("testdata/zstd_cli_concatenated_frames.zst");

    #[test]
    fn zstd_round_trip_level_1() {
        let c = ZstdCodec::new(1);
        let data = sample();
        let enc = c.encode(&data).unwrap();
        assert_eq!(c.decode(&enc).unwrap(), data);
        let empty = c.encode(&[]).unwrap();
        assert_eq!(c.decode(&empty).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn zstd_decodes_zstd_cli_streams() {
        let c = ZstdCodec::new(1);
        let plain = zstd_plain();
        assert_eq!(c.decode(ZSTD_L1_FIRST_HALF).unwrap(), plain[..2000]);
        assert_eq!(c.decode(ZSTD_L19_SECOND_HALF).unwrap(), plain[2000..]);
        assert_eq!(c.decode(ZSTD_L3_FULL_NO_CHECKSUM).unwrap(), plain);
    }

    #[test]
    fn zstd_decodes_concatenated_frames() {
        let c = ZstdCodec::new(1);
        assert_eq!(c.decode(ZSTD_CONCATENATED).unwrap(), zstd_plain());
    }

    #[test]
    fn zstd_rejects_truncated_stream() {
        let c = ZstdCodec::new(1);
        let cut = &ZSTD_L3_FULL_NO_CHECKSUM[..ZSTD_L3_FULL_NO_CHECKSUM.len() - 20];
        assert!(c.decode(cut).is_err());
        assert!(c.decode(&[]).is_err());
        assert!(c.decode(b"not a zstd stream").is_err());
    }

    #[test]
    fn zstd_encode_is_level_1_only() {
        assert!(matches!(
            ZstdCodec::new(3).encode(b"abc"),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            ZstdCodec::new(19).encode(b"abc"),
            Err(Error::Unsupported(_))
        ));
        // Decoding does not depend on the recorded level.
        assert_eq!(
            ZstdCodec::new(19).decode(ZSTD_L19_SECOND_HALF).unwrap(),
            zstd_plain()[2000..]
        );
    }

    #[test]
    fn zstd_factory_validates_level() {
        let r = super::super::Registry::builtin();
        let dt = DType::FLOAT64;
        assert!(r.build("zstd", &serde_json::json!({}), &dt).is_ok());
        assert!(
            r.build("zstd", &serde_json::json!({"level": 22}), &dt)
                .is_ok()
        );
        assert!(
            r.build("zstd", &serde_json::json!({"level": 23}), &dt)
                .is_err()
        );
        assert!(
            r.build("zstd", &serde_json::json!({"level": "x"}), &dt)
                .is_err()
        );
    }
}

#[cfg(test)]
mod zstd_checksum_tests {
    use super::*;

    #[test]
    fn corrupted_content_checksum_is_rejected() {
        // Stream written by the zstd CLI with a content checksum. Flipping the
        // last byte changes only the 4-byte checksum trailer.
        let good = include_bytes!("testdata/zstd_cli_l1_checksum_first_half.zst");
        let codec = ZstdCodec::new(1);
        assert!(codec.decode(good).is_ok());
        let mut bad = good.to_vec();
        let last = bad.len() - 1;
        bad[last] ^= 0xFF;
        assert!(
            codec.decode(&bad).is_err(),
            "corrupted checksum was accepted"
        );
    }
}
