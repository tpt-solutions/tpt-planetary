//! Byte-level filters: byte shuffle and delta, both implemented by hand.
//!
//! Both follow the numcodecs definitions used by Zarr v2 so that chunks
//! written by other implementations decode bit-for-bit.

use alloc::boxed::Box;
use alloc::format;
use alloc::vec::Vec;

use serde_json::Value;

use super::{Codec, config_i64};
use crate::dtype::{DType, Endian, Kind, read_uint};
use crate::error::{Error, Result};

/// Byte shuffle: groups the first byte of every element, then the second
/// byte, and so on. Trailing bytes that do not form a whole element are
/// copied unchanged.
#[derive(Debug, Clone, Copy)]
pub struct ShuffleCodec {
    elementsize: usize,
}

impl ShuffleCodec {
    /// Creates a shuffle filter for elements of `elementsize` bytes.
    pub fn new(elementsize: usize) -> Result<Self> {
        if elementsize == 0 {
            return Err(Error::InvalidMetadata(
                "shuffle elementsize must be positive".into(),
            ));
        }
        Ok(Self { elementsize })
    }
}

impl Codec for ShuffleCodec {
    fn name(&self) -> &str {
        "shuffle"
    }

    fn encode(&self, input: &[u8]) -> Result<Vec<u8>> {
        let es = self.elementsize;
        let n = input.len() / es;
        let whole = n * es;
        let mut out = alloc::vec![0u8; input.len()];
        for i in 0..n {
            for b in 0..es {
                out[b * n + i] = input[i * es + b];
            }
        }
        out[whole..].copy_from_slice(&input[whole..]);
        Ok(out)
    }

    fn decode(&self, input: &[u8]) -> Result<Vec<u8>> {
        let es = self.elementsize;
        let n = input.len() / es;
        let whole = n * es;
        let mut out = alloc::vec![0u8; input.len()];
        for i in 0..n {
            for b in 0..es {
                out[i * es + b] = input[b * n + i];
            }
        }
        out[whole..].copy_from_slice(&input[whole..]);
        Ok(out)
    }
}

/// Delta filter: stores the first element, then the difference to the
/// previous element, computed in the element's own type.
///
/// Integer deltas wrap and are exact. Floating-point deltas are computed with
/// floating-point subtraction and are therefore only approximately invertible
/// (as in numcodecs).
#[derive(Debug, Clone, Copy)]
pub struct DeltaCodec {
    dtype: DType,
}

impl DeltaCodec {
    /// Creates a delta filter for elements of `dtype`. Only integer and
    /// floating-point kinds are supported.
    pub fn new(dtype: DType) -> Result<Self> {
        match dtype.kind() {
            Kind::Int(_) | Kind::UInt(_) | Kind::Float(_) => Ok(Self { dtype }),
            other => Err(Error::Unsupported(format!("delta filter on {other:?}"))),
        }
    }
}

impl Codec for DeltaCodec {
    fn name(&self) -> &str {
        "delta"
    }

    fn encode(&self, input: &[u8]) -> Result<Vec<u8>> {
        self.run(input, true)
    }

    fn decode(&self, input: &[u8]) -> Result<Vec<u8>> {
        self.run(input, false)
    }
}

impl DeltaCodec {
    fn run(&self, input: &[u8], encode: bool) -> Result<Vec<u8>> {
        let size = self.dtype.item_size();
        if !input.len().is_multiple_of(size) {
            return Err(Error::SizeMismatch {
                expected: input.len() - input.len() % size,
                actual: input.len(),
            });
        }
        let big = self.dtype.endian() == Endian::Big;
        let mut out = Vec::with_capacity(input.len());
        let mut prev: Option<u64> = None;
        for chunk in input.chunks_exact(size) {
            let x = read_uint(chunk, big);
            let y = match (prev, encode) {
                (None, _) => x,
                (Some(p), true) => self.combine_sub(x, p),
                (Some(p), false) => self.combine_add(x, p),
            };
            // `prev` tracks the reconstructed value when decoding, and the
            // original value when encoding.
            prev = Some(if encode { x } else { y });
            push_bits(&mut out, y, size, big);
        }
        Ok(out)
    }

    fn combine_sub(&self, x: u64, p: u64) -> u64 {
        match self.dtype.kind() {
            Kind::Float(32) => f32_bits(f32::from_bits(x as u32) - f32::from_bits(p as u32)),
            Kind::Float(_) => (f64::from_bits(x) - f64::from_bits(p)).to_bits(),
            _ => self.mask(x.wrapping_sub(p)),
        }
    }

    fn combine_add(&self, d: u64, p: u64) -> u64 {
        match self.dtype.kind() {
            Kind::Float(32) => f32_bits(f32::from_bits(d as u32) + f32::from_bits(p as u32)),
            Kind::Float(_) => (f64::from_bits(d) + f64::from_bits(p)).to_bits(),
            _ => self.mask(d.wrapping_add(p)),
        }
    }

    fn mask(&self, v: u64) -> u64 {
        let bits = self.dtype.item_size() * 8;
        if bits >= 64 {
            v
        } else {
            v & ((1u64 << bits) - 1)
        }
    }
}

fn f32_bits(v: f32) -> u64 {
    u64::from(v.to_bits())
}

fn push_bits(out: &mut Vec<u8>, value: u64, width: usize, big: bool) {
    let bytes = value.to_le_bytes();
    if big {
        out.extend(bytes[..width].iter().rev());
    } else {
        out.extend_from_slice(&bytes[..width]);
    }
}

pub(crate) fn shuffle_factory(config: &Value, dtype: &DType) -> Result<Box<dyn Codec>> {
    let es = config_i64(config, "elementsize")?.map_or(dtype.item_size(), |v| v as usize);
    Ok(Box::new(ShuffleCodec::new(es)?))
}

pub(crate) fn delta_factory(config: &Value, dtype: &DType) -> Result<Box<dyn Codec>> {
    let d = match config.get("dtype").and_then(Value::as_str) {
        Some(s) => DType::from_v2(s)?,
        None => *dtype,
    };
    if let Some(astype) = config.get("astype").and_then(Value::as_str)
        && DType::from_v2(astype)? != d
    {
        return Err(Error::Unsupported("delta filter with astype".into()));
    }
    Ok(Box::new(DeltaCodec::new(d)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shuffle_round_trip_with_remainder() {
        let s = ShuffleCodec::new(4).unwrap();
        let data: Vec<u8> = (0..23u8).collect();
        let enc = s.encode(&data).unwrap();
        assert_ne!(enc, data);
        assert_eq!(&enc[20..], &data[20..]);
        assert_eq!(s.decode(&enc).unwrap(), data);
    }

    #[test]
    fn shuffle_known_layout() {
        let s = ShuffleCodec::new(2).unwrap();
        // elements [0x0102, 0x0304] -> bytes grouped by position: 02 04 01 03
        assert_eq!(
            s.encode(&[0x02, 0x01, 0x04, 0x03]).unwrap(),
            vec![0x02, 0x04, 0x01, 0x03]
        );
    }

    #[test]
    fn integer_delta_is_exact_and_wraps() {
        let d = DeltaCodec::new(DType::from_v2("<u1").unwrap()).unwrap();
        let data = [10u8, 13, 250, 3];
        let enc = d.encode(&data).unwrap();
        assert_eq!(enc, vec![10, 3, 237, 9]);
        assert_eq!(d.decode(&enc).unwrap(), data.to_vec());
    }

    #[test]
    fn signed_big_endian_delta_round_trip() {
        let dt = DType::from_v2(">i4").unwrap();
        let d = DeltaCodec::new(dt).unwrap();
        let values: [i32; 5] = [-5, 100, -1_000_000, 7, i32::MAX];
        let raw: Vec<u8> = values.iter().flat_map(|v| v.to_be_bytes()).collect();
        let enc = d.encode(&raw).unwrap();
        assert_eq!(d.decode(&enc).unwrap(), raw);
    }

    #[test]
    fn float_delta_round_trip_approximate() {
        let d = DeltaCodec::new(DType::FLOAT64).unwrap();
        let values = [1.0f64, 1.5, 2.25, -3.0];
        let raw: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let back = d.decode(&d.encode(&raw).unwrap()).unwrap();
        for (a, b) in back.chunks(8).zip(values) {
            let x = f64::from_le_bytes(a.try_into().unwrap());
            assert!((x - b).abs() < 1e-12);
        }
    }

    #[test]
    fn delta_rejects_bool() {
        assert!(DeltaCodec::new(DType::BOOL).is_err());
    }
}
