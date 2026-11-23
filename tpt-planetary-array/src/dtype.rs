//! Element data types: integers, floats, booleans, complex numbers and
//! fixed-length byte or unicode strings, with byte order.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::{Error, Result};

/// Byte order of multi-byte elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Endian {
    /// Least-significant byte first.
    Little,
    /// Most-significant byte first.
    Big,
}

/// The kind of an element, independent of byte order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// One-byte boolean (`0` or `1`).
    Bool,
    /// Signed integer of 8, 16, 32 or 64 bits.
    Int(u8),
    /// Unsigned integer of 8, 16, 32 or 64 bits.
    UInt(u8),
    /// IEEE-754 float of 16, 32 or 64 bits.
    Float(u8),
    /// Complex number with 64 bits (two `f32`) or 128 bits (two `f64`) in total.
    Complex(u8),
    /// Fixed-length byte string of the given number of bytes.
    FixedBytes(usize),
    /// Fixed-length unicode string of the given number of code points (UTF-32).
    FixedUnicode(usize),
}

/// An element data type: a [`Kind`] together with its byte order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DType {
    kind: Kind,
    endian: Endian,
}

impl DType {
    /// Creates a data type from a kind and byte order. Single-byte kinds
    /// ignore the byte order.
    pub fn new(kind: Kind, endian: Endian) -> Result<Self> {
        match kind {
            Kind::Int(b) | Kind::UInt(b) if !matches!(b, 8 | 16 | 32 | 64) => {
                Err(Error::Unsupported(format!("integer width {b}")))
            }
            Kind::Float(b) if !matches!(b, 16 | 32 | 64) => {
                Err(Error::Unsupported(format!("float width {b}")))
            }
            Kind::Complex(b) if !matches!(b, 64 | 128) => {
                Err(Error::Unsupported(format!("complex width {b}")))
            }
            _ => Ok(Self { kind, endian }),
        }
    }

    /// Little-endian `f8`.
    pub const FLOAT64: DType = DType {
        kind: Kind::Float(64),
        endian: Endian::Little,
    };
    /// Little-endian `f4`.
    pub const FLOAT32: DType = DType {
        kind: Kind::Float(32),
        endian: Endian::Little,
    };
    /// Little-endian `i8`.
    pub const INT64: DType = DType {
        kind: Kind::Int(64),
        endian: Endian::Little,
    };
    /// Little-endian `i4`.
    pub const INT32: DType = DType {
        kind: Kind::Int(32),
        endian: Endian::Little,
    };
    /// Little-endian `u8`.
    pub const UINT64: DType = DType {
        kind: Kind::UInt(64),
        endian: Endian::Little,
    };
    /// Boolean (`b1`).
    pub const BOOL: DType = DType {
        kind: Kind::Bool,
        endian: Endian::Little,
    };

    /// The kind of element.
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The byte order (meaningful only for multi-byte kinds).
    pub fn endian(&self) -> Endian {
        self.endian
    }

    /// Returns a copy with a different byte order.
    pub fn with_endian(self, endian: Endian) -> Self {
        Self {
            kind: self.kind,
            endian,
        }
    }

    /// Size of one element in bytes.
    pub fn item_size(&self) -> usize {
        match self.kind {
            Kind::Bool => 1,
            Kind::Int(b) | Kind::UInt(b) | Kind::Float(b) | Kind::Complex(b) => (b / 8) as usize,
            Kind::FixedBytes(n) => n,
            Kind::FixedUnicode(n) => n * 4,
        }
    }

    /// True for kinds that can be converted to `f64` by [`DType::decode_f64`].
    pub fn is_real_numeric(&self) -> bool {
        matches!(
            self.kind,
            Kind::Bool | Kind::Int(_) | Kind::UInt(_) | Kind::Float(_)
        )
    }

    /// True for floating-point kinds (real or complex).
    pub fn is_float(&self) -> bool {
        matches!(self.kind, Kind::Float(_) | Kind::Complex(_))
    }

    /// Parses a Zarr v2 `dtype` string such as `<f8`, `|b1`, `>i4` or `<U5`.
    pub fn from_v2(s: &str) -> Result<Self> {
        let bytes = s.as_bytes();
        if bytes.len() < 3 {
            return Err(Error::InvalidMetadata(format!("dtype '{s}' is too short")));
        }
        let endian = match bytes[0] {
            b'<' | b'=' | b'|' => Endian::Little,
            b'>' => Endian::Big,
            _ => {
                return Err(Error::InvalidMetadata(format!(
                    "bad byte order in dtype '{s}'"
                )));
            }
        };
        let size: usize = s[2..]
            .parse()
            .map_err(|_| Error::InvalidMetadata(format!("bad size in dtype '{s}'")))?;
        let bits = (size * 8) as u8;
        let kind = match bytes[1] {
            b'b' if size == 1 => Kind::Bool,
            b'i' => Kind::Int(bits),
            b'u' => Kind::UInt(bits),
            b'f' => Kind::Float(bits),
            b'c' => Kind::Complex(bits),
            b'S' => Kind::FixedBytes(size),
            b'U' => Kind::FixedUnicode(size),
            _ => return Err(Error::Unsupported(format!("dtype '{s}'"))),
        };
        Self::new(kind, endian)
    }

    /// Formats the dtype as a Zarr v2 `dtype` string.
    pub fn to_v2(&self) -> String {
        let order = match (self.kind, self.endian) {
            (Kind::Bool | Kind::FixedBytes(_), _) => '|',
            (Kind::Int(8) | Kind::UInt(8), _) => '|',
            (_, Endian::Little) => '<',
            (_, Endian::Big) => '>',
        };
        let code = match self.kind {
            Kind::Bool => 'b',
            Kind::Int(_) => 'i',
            Kind::UInt(_) => 'u',
            Kind::Float(_) => 'f',
            Kind::Complex(_) => 'c',
            Kind::FixedBytes(_) => 'S',
            Kind::FixedUnicode(_) => 'U',
        };
        let size = match self.kind {
            Kind::FixedBytes(n) | Kind::FixedUnicode(n) => n,
            _ => self.item_size(),
        };
        format!("{order}{code}{size}")
    }

    /// Parses a Zarr v3 data type name such as `float64` or `uint16`.
    /// Byte order comes from the `bytes` codec and is supplied separately.
    pub fn from_v3(name: &str, endian: Endian) -> Result<Self> {
        let kind = match name {
            "bool" => Kind::Bool,
            "int8" => Kind::Int(8),
            "int16" => Kind::Int(16),
            "int32" => Kind::Int(32),
            "int64" => Kind::Int(64),
            "uint8" => Kind::UInt(8),
            "uint16" => Kind::UInt(16),
            "uint32" => Kind::UInt(32),
            "uint64" => Kind::UInt(64),
            "float16" => Kind::Float(16),
            "float32" => Kind::Float(32),
            "float64" => Kind::Float(64),
            "complex64" => Kind::Complex(64),
            "complex128" => Kind::Complex(128),
            other => return Err(Error::Unsupported(format!("v3 data type '{other}'"))),
        };
        Self::new(kind, endian)
    }

    /// The Zarr v3 data type name, if the kind has one.
    pub fn v3_name(&self) -> Result<String> {
        let name = match self.kind {
            Kind::Bool => "bool".to_string(),
            Kind::Int(b) => format!("int{b}"),
            Kind::UInt(b) => format!("uint{b}"),
            Kind::Float(b) => format!("float{b}"),
            Kind::Complex(b) => format!("complex{b}"),
            _ => {
                return Err(Error::Unsupported(
                    "fixed-length strings have no v3 name in this crate".to_string(),
                ));
            }
        };
        Ok(name)
    }

    /// Decodes `bytes` (a whole number of elements in this dtype) into `f64`
    /// values. Booleans map to 0.0 and 1.0; complex and string types fail.
    pub fn decode_f64(&self, bytes: &[u8]) -> Result<Vec<f64>> {
        if !self.is_real_numeric() {
            return Err(Error::Unsupported(format!(
                "{:?} cannot be read as f64",
                self.kind
            )));
        }
        let size = self.item_size();
        if !bytes.len().is_multiple_of(size) {
            return Err(Error::SizeMismatch {
                expected: bytes.len() - bytes.len() % size,
                actual: bytes.len(),
            });
        }
        let big = self.endian == Endian::Big;
        let mut out = Vec::with_capacity(bytes.len() / size);
        for chunk in bytes.chunks_exact(size) {
            let raw = read_uint(chunk, big);
            let v = match self.kind {
                Kind::Bool => f64::from(chunk[0] != 0),
                Kind::Int(b) => sign_extend(raw, b) as f64,
                Kind::UInt(_) => raw as f64,
                Kind::Float(16) => f16_to_f64(raw as u16),
                Kind::Float(32) => f32::from_bits(raw as u32) as f64,
                Kind::Float(_) => f64::from_bits(raw),
                _ => unreachable!("checked by is_real_numeric"),
            };
            out.push(v);
        }
        Ok(out)
    }
}

impl DType {
    /// Encodes `values` as elements of this dtype in its byte order.
    ///
    /// Integer targets truncate toward zero and saturate at the type bounds
    /// (NaN becomes zero). Booleans are `value != 0.0`. Complex and string
    /// dtypes are not supported.
    pub fn encode_f64(&self, values: &[f64]) -> Result<Vec<u8>> {
        if !self.is_real_numeric() {
            return Err(Error::Unsupported(format!(
                "{:?} cannot be written from f64",
                self.kind
            )));
        }
        let size = self.item_size();
        let big = self.endian == Endian::Big;
        let mut out = Vec::with_capacity(values.len() * size);
        for &v in values {
            let bits: u64 = match self.kind {
                Kind::Bool => u64::from(v != 0.0),
                Kind::Int(_) => v as i64 as u64,
                Kind::UInt(_) => v as u64,
                Kind::Float(16) => u64::from(f64_to_f16(v)),
                Kind::Float(32) => u64::from((v as f32).to_bits()),
                Kind::Float(_) => v.to_bits(),
                _ => unreachable!("checked by is_real_numeric"),
            };
            let le = bits.to_le_bytes();
            if big {
                out.extend(le[..size].iter().rev());
            } else {
                out.extend_from_slice(&le[..size]);
            }
        }
        Ok(out)
    }
}

/// Reads an unsigned integer from `chunk` (1, 2, 4 or 8 bytes) using the
/// given byte order.
pub(crate) fn read_uint(chunk: &[u8], big: bool) -> u64 {
    let mut v: u64 = 0;
    if big {
        for &b in chunk {
            v = (v << 8) | u64::from(b);
        }
    } else {
        for &b in chunk.iter().rev() {
            v = (v << 8) | u64::from(b);
        }
    }
    v
}

/// Interprets the low `bits` bits of `raw` as a two's-complement integer.
pub(crate) fn sign_extend(raw: u64, bits: u8) -> i64 {
    let shift = 64 - u32::from(bits);
    ((raw << shift) as i64) >> shift
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v2_strings() {
        assert_eq!(DType::from_v2("<f8").unwrap(), DType::FLOAT64);
        assert_eq!(DType::from_v2("|b1").unwrap(), DType::BOOL);
        assert_eq!(DType::from_v2(">i4").unwrap().endian(), Endian::Big);
        assert_eq!(DType::from_v2("|S7").unwrap().item_size(), 7);
        assert_eq!(DType::from_v2("<U5").unwrap().item_size(), 20);
        assert_eq!(DType::from_v2("<c16").unwrap().item_size(), 16);
        assert_eq!(DType::from_v2("<f2").unwrap().item_size(), 2);
        assert!(DType::from_v2("<M8").is_err());
    }

    #[test]
    fn v2_round_trip() {
        for s in ["<f8", "<f4", ">i2", "|u1", "|b1", "<c8", "|S3", "<U2"] {
            assert_eq!(DType::from_v2(s).unwrap().to_v2(), s);
        }
    }

    #[test]
    fn decodes_big_endian_and_signed() {
        let d = DType::from_v2(">u2").unwrap();
        assert_eq!(
            d.decode_f64(&[0x01, 0x02, 0xff, 0xff]).unwrap(),
            vec![258.0, 65535.0]
        );
        assert_eq!(
            DType::BOOL.decode_f64(&[0, 1, 7]).unwrap(),
            vec![0.0, 1.0, 1.0]
        );
        let i = DType::from_v2("<i2").unwrap();
        assert_eq!(i.decode_f64(&[0xff, 0xff]).unwrap(), vec![-1.0]);
        let i1 = DType::from_v2("|i1").unwrap();
        assert_eq!(i1.decode_f64(&[0x80]).unwrap(), vec![-128.0]);
    }

    #[test]
    fn v3_names() {
        let d = DType::from_v3("float32", Endian::Big).unwrap();
        assert_eq!(d.v3_name().unwrap(), "float32");
        assert_eq!(d.endian(), Endian::Big);
        assert!(DType::from_v3("string", Endian::Little).is_err());
    }
}

/// `2^k` for `k` in the normal `f64` exponent range, built from its bits.
fn pow2(k: i32) -> f64 {
    f64::from_bits(((k + 1023) as u64) << 52)
}

/// The value of an IEEE binary16 bit pattern, exactly (every half is a double).
pub(crate) fn f16_to_f64(bits: u16) -> f64 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((bits >> 10) & 0x1f);
    let mant = f64::from(bits & 0x3ff);
    let magnitude = if exp == 0x1f {
        if mant == 0.0 { f64::INFINITY } else { f64::NAN }
    } else if exp == 0 {
        // Subnormal: mant * 2^-24.
        mant * pow2(-24)
    } else {
        // Normal: (1024 + mant) * 2^(exp - 25).
        (1024.0 + mant) * pow2(exp - 25)
    };
    sign * magnitude
}

/// The binary16 bit pattern nearest to `v`, rounding ties to even. Values beyond
/// the largest half become infinity; NaN becomes a quiet NaN.
pub(crate) fn f64_to_f16(v: f64) -> u16 {
    if v.is_nan() {
        return 0x7e00;
    }
    let sign: u16 = if v.is_sign_negative() { 0x8000 } else { 0 };
    let a = v.abs();
    if a.is_infinite() || a >= 65520.0 {
        // 65520 is the midpoint between the largest half (65504) and 2^16.
        return sign | 0x7c00;
    }
    if a == 0.0 {
        return sign;
    }
    // Round `x` to an integer, ties to even, for `0 <= x < 2^20`.
    let round_even = |x: f64| -> u32 {
        let t = x as u32;
        let frac = x - f64::from(t);
        if frac > 0.5 || (frac == 0.5 && t % 2 == 1) {
            t + 1
        } else {
            t
        }
    };
    if a < pow2(-14) {
        // Subnormal: the mantissa counts units of 2^-24; a carry into 1024 is the
        // smallest normal, which the bit layout represents directly.
        let m = round_even(a / pow2(-24));
        return sign | m as u16;
    }
    // Normal: exponent e with 2^e <= a < 2^(e+1), mantissa rounded at 2^(e-10).
    let e = (((a.to_bits() >> 52) & 0x7ff) as i32) - 1023;
    let q = round_even(a / pow2(e - 10));
    let (e, q) = if q == 2048 { (e + 1, 1024) } else { (e, q) };
    if e > 15 {
        return sign | 0x7c00;
    }
    sign | (((e + 15) as u16) << 10) | (q - 1024) as u16
}

#[cfg(all(test, feature = "std"))]
mod float16_tests {
    use super::*;

    #[test]
    fn every_finite_half_round_trips_exactly() {
        for bits in 0..=u16::MAX {
            let v = f16_to_f64(bits);
            if v.is_nan() {
                assert!(f16_to_f64(f64_to_f16(v)).is_nan());
                continue;
            }
            assert_eq!(f64_to_f16(v), bits, "pattern {bits:#06x} value {v}");
        }
    }

    #[test]
    fn known_values_and_rounding_boundaries() {
        assert_eq!(f64_to_f16(1.0), 0x3c00);
        assert_eq!(f64_to_f16(-2.5), 0xc100);
        assert_eq!(f64_to_f16(65504.0), 0x7bff);
        assert_eq!(f64_to_f16(65519.0), 0x7bff);
        assert_eq!(f64_to_f16(65520.0), 0x7c00);
        assert_eq!(f64_to_f16(f64::INFINITY), 0x7c00);
        assert_eq!(f64_to_f16(f64::NEG_INFINITY), 0xfc00);
        assert_eq!(f64_to_f16(2f64.powi(-24)), 0x0001);
        // Exactly half the smallest subnormal ties to even (zero); a hair more rounds up.
        assert_eq!(f64_to_f16(2f64.powi(-25)), 0x0000);
        assert_eq!(f64_to_f16(2f64.powi(-25) * 1.000001), 0x0001);
        assert_eq!(f64_to_f16(0.1), 0x2e66);
        assert_eq!(f64_to_f16(-0.0), 0x8000);
        assert_eq!(f64_to_f16(f64::NAN) & 0x7c00, 0x7c00);
    }

    #[test]
    fn matches_numpy_float16_rounding() {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/float16_rounding.tsv"),
        )
        .unwrap();
        let mut n = 0;
        for line in text.lines() {
            let (v, bits) = line.split_once('\t').unwrap();
            let v: f64 = v.parse().unwrap();
            let bits: u16 = bits.parse().unwrap();
            assert_eq!(f64_to_f16(v), bits, "value {v}");
            n += 1;
        }
        assert!(n > 4000);
    }

    #[test]
    fn float16_arrays_decode_and_encode_through_dtype() {
        let dt = DType::from_v3("float16", Endian::Little).unwrap();
        assert_eq!(dt.item_size(), 2);
        assert_eq!(dt.v3_name().unwrap(), "float16");
        let values = [1.0, -0.5, 65504.0, 0.25];
        let bytes = dt.encode_f64(&values).unwrap();
        assert_eq!(bytes.len(), 8);
        assert_eq!(dt.decode_f64(&bytes).unwrap(), values.to_vec());
    }
}
