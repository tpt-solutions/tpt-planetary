//! CRC-32C (Castagnoli) checksum and the Zarr v3 `crc32c` codec.
//!
//! Parameters (CRC-32C catalogue entry): polynomial 0x1EDC6F41 (reflected
//! 0x82F63B78), initial value 0xFFFFFFFF, reflected input and output, final
//! XOR 0xFFFFFFFF. The check value for the ASCII string `"123456789"` is
//! 0xE3069283. The table is generated at compile time with the bit-by-bit
//! algorithm; the checksum is computed one byte per table lookup.

use alloc::boxed::Box;
use alloc::vec::Vec;

use serde_json::Value;

use super::Codec;
use crate::dtype::DType;
use crate::error::{Error, Result};

/// Reflected CRC-32C polynomial.
const POLY_REFLECTED: u32 = 0x82F6_3B78;

/// Byte-indexed lookup table for [`crc32c`], built at compile time.
const TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                POLY_REFLECTED ^ (c >> 1)
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

/// Computes the CRC-32C checksum of `data`.
///
/// ```
/// assert_eq!(tpt_planetary_array::codec::crc32c::crc32c(b"123456789"), 0xE306_9283);
/// ```
pub fn crc32c(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = TABLE[((crc ^ u32::from(b)) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

/// The Zarr v3 `crc32c` bytes-to-bytes codec.
///
/// Encoding appends the CRC-32C of the input as a 4-byte little-endian
/// trailer. Decoding checks and removes the trailer, and fails on mismatch.
#[derive(Debug, Clone, Copy, Default)]
pub struct Crc32cCodec;

impl Codec for Crc32cCodec {
    fn name(&self) -> &str {
        "crc32c"
    }

    fn encode(&self, input: &[u8]) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(input.len() + 4);
        out.extend_from_slice(input);
        out.extend_from_slice(&crc32c(input).to_le_bytes());
        Ok(out)
    }

    fn decode(&self, input: &[u8]) -> Result<Vec<u8>> {
        let Some(split) = input.len().checked_sub(4) else {
            return Err(Error::Codec("crc32c: input shorter than checksum".into()));
        };
        let (body, trailer) = input.split_at(split);
        let stored = u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
        let computed = crc32c(body);
        if stored != computed {
            return Err(Error::Codec(alloc::format!(
                "crc32c: checksum mismatch (stored {stored:#010x}, computed {computed:#010x})"
            )));
        }
        Ok(body.to_vec())
    }
}

pub(crate) fn crc32c_factory(_config: &Value, _dtype: &DType) -> Result<Box<dyn Codec>> {
    Ok(Box::new(Crc32cCodec))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_value_for_123456789() {
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn empty_input_and_single_byte() {
        assert_eq!(crc32c(b""), 0);
        // CRC-32C of a single 0x00 byte is 0x527D5351 (init/xorout 0xFFFFFFFF).
        assert_eq!(crc32c(&[0u8]), 0x527D_5351);
    }

    #[test]
    fn codec_round_trip_and_detects_corruption() {
        let c = Crc32cCodec;
        let data = b"zarr v3 payload";
        let enc = c.encode(data).unwrap();
        assert_eq!(enc.len(), data.len() + 4);
        assert_eq!(c.decode(&enc).unwrap(), data.to_vec());
        let mut bad = enc.clone();
        bad[0] ^= 1;
        assert!(c.decode(&bad).is_err());
        assert!(c.decode(&[1, 2, 3]).is_err());
    }
}
