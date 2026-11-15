//! Fill values: the value reported for chunks that were never written, and
//! the sentinel used by NaN-aware reductions.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::{Number, Value};

use crate::dtype::{DType, Endian, Kind};
use crate::error::{Error, Result};

/// A fill value as stored in array metadata.
#[derive(Debug, Clone)]
pub enum FillValue {
    /// No fill value (`null`). Missing chunks read as zeros.
    Null,
    /// Boolean fill.
    Bool(bool),
    /// Signed integer fill.
    Int(i64),
    /// Unsigned integer fill.
    UInt(u64),
    /// Floating-point fill (may be NaN or infinite).
    Float(f64),
    /// Complex fill as `(real, imaginary)`.
    Complex(f64, f64),
    /// Text fill for fixed-length string dtypes.
    Text(String),
}

impl PartialEq for FillValue {
    /// Structural equality in which NaN equals NaN, so that a fill value
    /// survives a metadata round trip with `==`.
    fn eq(&self, other: &Self) -> bool {
        fn same(a: f64, b: f64) -> bool {
            a == b || (a.is_nan() && b.is_nan())
        }
        match (self, other) {
            (FillValue::Null, FillValue::Null) => true,
            (FillValue::Bool(a), FillValue::Bool(b)) => a == b,
            (FillValue::Int(a), FillValue::Int(b)) => a == b,
            (FillValue::UInt(a), FillValue::UInt(b)) => a == b,
            (FillValue::Float(a), FillValue::Float(b)) => same(*a, *b),
            (FillValue::Complex(ar, ai), FillValue::Complex(br, bi)) => {
                same(*ar, *br) && same(*ai, *bi)
            }
            (FillValue::Text(a), FillValue::Text(b)) => a == b,
            _ => false,
        }
    }
}

impl FillValue {
    /// Parses a fill value from JSON as written by Zarr v2 or v3.
    ///
    /// Non-finite floats are written as the strings `"NaN"`, `"Infinity"` and
    /// `"-Infinity"`. Complex values are two-element arrays.
    pub fn from_json(v: &Value) -> Result<Self> {
        Ok(match v {
            Value::Null => FillValue::Null,
            Value::Bool(b) => FillValue::Bool(*b),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    FillValue::Int(i)
                } else if let Some(u) = n.as_u64() {
                    FillValue::UInt(u)
                } else {
                    FillValue::Float(n.as_f64().unwrap_or(f64::NAN))
                }
            }
            Value::String(s) => match s.as_str() {
                "NaN" => FillValue::Float(f64::NAN),
                "Infinity" => FillValue::Float(f64::INFINITY),
                "-Infinity" => FillValue::Float(f64::NEG_INFINITY),
                other => FillValue::Text(other.to_string()),
            },
            Value::Array(items) if items.len() == 2 => {
                let re = Self::float_part(&items[0])?;
                let im = Self::float_part(&items[1])?;
                FillValue::Complex(re, im)
            }
            other => {
                return Err(Error::InvalidMetadata(format!(
                    "unsupported fill value {other}"
                )));
            }
        })
    }

    fn float_part(v: &Value) -> Result<f64> {
        match Self::from_json(v)? {
            FillValue::Int(i) => Ok(i as f64),
            FillValue::UInt(u) => Ok(u as f64),
            FillValue::Float(f) => Ok(f),
            _ => Err(Error::InvalidMetadata(
                "complex fill parts must be numbers".to_string(),
            )),
        }
    }

    /// Serialises the fill value to JSON using the Zarr convention for
    /// non-finite floats.
    pub fn to_json(&self) -> Value {
        match self {
            FillValue::Null => Value::Null,
            FillValue::Bool(b) => Value::Bool(*b),
            FillValue::Int(i) => Value::Number(Number::from(*i)),
            FillValue::UInt(u) => Value::Number(Number::from(*u)),
            FillValue::Float(f) => float_json(*f),
            FillValue::Complex(re, im) => {
                Value::Array(alloc::vec![float_json(*re), float_json(*im)])
            }
            FillValue::Text(s) => Value::String(s.clone()),
        }
    }

    /// The fill value as a single `f64`, if it is numeric.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            FillValue::Bool(b) => Some(f64::from(u8::from(*b))),
            FillValue::Int(i) => Some(*i as f64),
            FillValue::UInt(u) => Some(*u as f64),
            FillValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// True if decoded value `v` should be treated as the fill value.
    /// A NaN fill matches NaN values; `null` fill never matches.
    pub fn matches(&self, v: f64) -> bool {
        match self.as_f64() {
            Some(f) if f.is_nan() => v.is_nan(),
            Some(f) => v == f,
            None => false,
        }
    }

    /// Encodes the fill value as one element of `dtype` in its byte order.
    /// `Null` encodes as zero bytes.
    pub fn to_element_bytes(&self, dtype: &DType) -> Result<Vec<u8>> {
        let size = dtype.item_size();
        let big = dtype.endian() == Endian::Big;
        let mut out = Vec::with_capacity(size);
        match (self, dtype.kind()) {
            (FillValue::Null, _) => out.resize(size, 0),
            (FillValue::Text(s), Kind::FixedBytes(n)) => {
                let b = s.as_bytes();
                if b.len() > n {
                    return Err(Error::InvalidMetadata(
                        "fill string longer than dtype".to_string(),
                    ));
                }
                out.extend_from_slice(b);
                out.resize(n, 0);
            }
            (FillValue::Text(s), Kind::FixedUnicode(n)) => {
                let count = s.chars().count();
                if count > n {
                    return Err(Error::InvalidMetadata(
                        "fill string longer than dtype".to_string(),
                    ));
                }
                for c in s.chars() {
                    push_uint(&mut out, u64::from(u32::from(c)), 4, big);
                }
                out.resize(size, 0);
            }
            (_, Kind::Bool) => {
                let v = self.as_f64().unwrap_or(0.0) != 0.0;
                out.push(u8::from(v));
            }
            (_, Kind::Int(b) | Kind::UInt(b)) => {
                let raw = match self {
                    FillValue::Int(i) => *i as u64,
                    FillValue::UInt(u) => *u,
                    FillValue::Bool(v) => u64::from(*v),
                    FillValue::Float(f) => *f as i64 as u64,
                    _ => return Err(mismatch(self, dtype)),
                };
                push_uint(&mut out, raw, usize::from(b / 8), big);
            }
            (_, Kind::Float(32)) => {
                let f = self.as_f64().ok_or_else(|| mismatch(self, dtype))?;
                push_uint(&mut out, u64::from((f as f32).to_bits()), 4, big);
            }
            (_, Kind::Float(_)) => {
                let f = self.as_f64().ok_or_else(|| mismatch(self, dtype))?;
                push_uint(&mut out, f.to_bits(), 8, big);
            }
            (FillValue::Complex(re, im), Kind::Complex(b)) => {
                if b == 64 {
                    push_uint(&mut out, u64::from((*re as f32).to_bits()), 4, big);
                    push_uint(&mut out, u64::from((*im as f32).to_bits()), 4, big);
                } else {
                    push_uint(&mut out, re.to_bits(), 8, big);
                    push_uint(&mut out, im.to_bits(), 8, big);
                }
            }
            (_, Kind::Complex(_)) => {
                let f = self.as_f64().ok_or_else(|| mismatch(self, dtype))?;
                let zero_bytes = dtype.item_size() / 2;
                if zero_bytes == 4 {
                    push_uint(&mut out, u64::from((f as f32).to_bits()), 4, big);
                    out.resize(size, 0);
                } else {
                    push_uint(&mut out, f.to_bits(), 8, big);
                    out.resize(size, 0);
                }
            }
            _ => return Err(mismatch(self, dtype)),
        }
        Ok(out)
    }

    /// Returns `count` copies of the element encoding, concatenated.
    pub fn repeat_bytes(&self, dtype: &DType, count: usize) -> Result<Vec<u8>> {
        let one = self.to_element_bytes(dtype)?;
        let mut out = Vec::with_capacity(one.len() * count);
        for _ in 0..count {
            out.extend_from_slice(&one);
        }
        Ok(out)
    }
}

fn mismatch(v: &FillValue, dtype: &DType) -> Error {
    Error::InvalidMetadata(format!(
        "fill value {v:?} does not fit dtype {:?}",
        dtype.kind()
    ))
}

fn float_json(f: f64) -> Value {
    if f.is_nan() {
        Value::String("NaN".to_string())
    } else if f == f64::INFINITY {
        Value::String("Infinity".to_string())
    } else if f == f64::NEG_INFINITY {
        Value::String("-Infinity".to_string())
    } else {
        match Number::from_f64(f) {
            Some(n) => Value::Number(n),
            None => Value::Null,
        }
    }
}

fn push_uint(out: &mut Vec<u8>, value: u64, width: usize, big: bool) {
    let bytes = value.to_le_bytes();
    if big {
        out.extend(bytes[..width].iter().rev());
    } else {
        out.extend_from_slice(&bytes[..width]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nan_and_infinity_round_trip() {
        let v = FillValue::from_json(&Value::String("NaN".into())).unwrap();
        assert!(matches!(v, FillValue::Float(f) if f.is_nan()));
        assert_eq!(v.to_json(), Value::String("NaN".into()));
        let inf = FillValue::from_json(&Value::String("-Infinity".into())).unwrap();
        assert_eq!(inf.to_json(), Value::String("-Infinity".into()));
    }

    #[test]
    fn encodes_big_endian_float() {
        let d = DType::from_v2(">f4").unwrap();
        let b = FillValue::Float(1.0).to_element_bytes(&d).unwrap();
        assert_eq!(b, vec![0x3f, 0x80, 0x00, 0x00]);
    }

    #[test]
    fn nan_fill_matches_nan_only() {
        let v = FillValue::Float(f64::NAN);
        assert!(v.matches(f64::NAN));
        assert!(!v.matches(0.0));
        assert!(!FillValue::Null.matches(0.0));
        assert!(FillValue::Int(-9999).matches(-9999.0));
    }

    #[test]
    fn string_fill_is_padded() {
        let d = DType::from_v2("|S4").unwrap();
        let b = FillValue::Text("ab".into()).to_element_bytes(&d).unwrap();
        assert_eq!(b, vec![b'a', b'b', 0, 0]);
    }
}
