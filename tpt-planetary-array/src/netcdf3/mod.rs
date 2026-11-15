//! NetCDF-3 Classic (`CDF-1`) and 64-bit offset (`CDF-2`) files in pure Rust.
//!
//! The layout follows the Unidata NetCDF-3 file format specification: a
//! big-endian header holding the dimension list, the global attribute list
//! and the variable list, followed by the variable data. Names, attribute
//! payloads and every variable's data are zero-padded to a multiple of four
//! bytes.
//!
//! - [`Reader`] parses a complete file held in a byte slice and decodes
//!   variables. Record variables, which share one unlimited dimension and are
//!   stored interleaved record by record, are supported.
//! - [`Writer`] builds a complete file in a `Vec<u8>`.
//! - [`cf`] parses CF `"<unit> since <date>"` time units and converts numeric
//!   offsets to calendar dates.
//!
//! Not supported: CDF-5 (64-bit data) files, NetCDF-4 (HDF5-based) files,
//! streaming files whose header holds `numrecs = 0xFFFFFFFF`, and the
//! NetCDF-4 enhanced data model (groups, user-defined types).
//!
//! # Example
//!
//! ```
//! use tpt_planetary_array::netcdf3::{Format, NcValues, Reader, Writer};
//!
//! let mut writer = Writer::new(Format::Classic);
//! let x = writer.add_dimension("x", Some(3)).unwrap();
//! writer
//!     .add_variable("v", &[x], Vec::new(), NcValues::Float(vec![1.5, 2.5, 3.5]))
//!     .unwrap();
//! let bytes = writer.finish().unwrap();
//!
//! let reader = Reader::new(&bytes).unwrap();
//! assert_eq!(reader.read_f64("v").unwrap(), vec![1.5, 2.5, 3.5]);
//! ```

pub mod cf;
mod reader;
mod writer;

#[cfg(test)]
mod tests;

pub use cf::{Calendar, CfDateTime, TimeUnit, TimeUnits};
pub use reader::Reader;
pub use writer::Writer;

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{Error, Result};

/// The three-byte magic that starts every NetCDF-3 file.
pub(crate) const MAGIC: &[u8; 3] = b"CDF";
/// Header tag introducing a dimension list (`NC_DIMENSION`).
pub(crate) const TAG_DIMENSION: u32 = 0x0A;
/// Header tag introducing a variable list (`NC_VARIABLE`).
pub(crate) const TAG_VARIABLE: u32 = 0x0B;
/// Header tag introducing an attribute list (`NC_ATTRIBUTE`).
pub(crate) const TAG_ATTRIBUTE: u32 = 0x0C;
/// `numrecs` value marking a streaming file whose length is not in the header.
pub(crate) const STREAMING_NUMRECS: u32 = 0xFFFF_FFFF;

/// The two NetCDF-3 variants this module reads and writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// CDF-1 (`CDF\x01`): variable `begin` offsets are 32-bit.
    Classic,
    /// CDF-2 (`CDF\x02`): variable `begin` offsets are 64-bit.
    Offset64,
}

impl Format {
    /// The version byte stored after the `CDF` magic.
    pub const fn version(self) -> u8 {
        match self {
            Format::Classic => 1,
            Format::Offset64 => 2,
        }
    }

    /// Width in bytes of a variable `begin` offset in the header.
    pub const fn offset_size(self) -> usize {
        match self {
            Format::Classic => 4,
            Format::Offset64 => 8,
        }
    }
}

/// The external element types of NetCDF-3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NcType {
    /// `NC_BYTE`: signed 8-bit integer.
    Byte,
    /// `NC_CHAR`: 8-bit character.
    Char,
    /// `NC_SHORT`: signed 16-bit integer.
    Short,
    /// `NC_INT`: signed 32-bit integer.
    Int,
    /// `NC_FLOAT`: IEEE 32-bit float.
    Float,
    /// `NC_DOUBLE`: IEEE 64-bit float.
    Double,
}

impl NcType {
    /// The numeric type code used in the header.
    pub const fn tag(self) -> u32 {
        match self {
            NcType::Byte => 1,
            NcType::Char => 2,
            NcType::Short => 3,
            NcType::Int => 4,
            NcType::Float => 5,
            NcType::Double => 6,
        }
    }

    /// Size of one element in bytes.
    pub const fn size(self) -> usize {
        match self {
            NcType::Byte | NcType::Char => 1,
            NcType::Short => 2,
            NcType::Int | NcType::Float => 4,
            NcType::Double => 8,
        }
    }

    /// Maps a header type code to a type, or `None` for unknown codes.
    pub const fn from_tag(tag: u32) -> Option<Self> {
        match tag {
            1 => Some(NcType::Byte),
            2 => Some(NcType::Char),
            3 => Some(NcType::Short),
            4 => Some(NcType::Int),
            5 => Some(NcType::Float),
            6 => Some(NcType::Double),
            _ => None,
        }
    }
}

/// A dimension of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dimension {
    /// Dimension name.
    pub name: String,
    /// Length. For the unlimited dimension this is the number of records when
    /// read, and is ignored (set to zero) when written.
    pub len: usize,
    /// True for the single unlimited (record) dimension.
    pub unlimited: bool,
}

/// A named attribute, global or attached to a variable.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    /// Attribute name.
    pub name: String,
    /// Attribute values; the variant is the attribute's NetCDF type.
    pub values: NcValues,
}

impl Attribute {
    /// Creates an attribute from a name and typed values.
    pub fn new(name: &str, values: NcValues) -> Self {
        Self {
            name: String::from(name),
            values,
        }
    }

    /// Creates a text (`NC_CHAR`) attribute.
    pub fn text(name: &str, value: &str) -> Self {
        Self::new(name, NcValues::text(value))
    }
}

/// Typed values of a variable or attribute, in the file's native type.
///
/// Values are stored in big-endian order in the file and decoded into these
/// vectors on read.
#[derive(Debug, Clone, PartialEq)]
pub enum NcValues {
    /// `NC_BYTE` values.
    Byte(Vec<i8>),
    /// `NC_CHAR` bytes (usually text).
    Char(Vec<u8>),
    /// `NC_SHORT` values.
    Short(Vec<i16>),
    /// `NC_INT` values.
    Int(Vec<i32>),
    /// `NC_FLOAT` values.
    Float(Vec<f32>),
    /// `NC_DOUBLE` values.
    Double(Vec<f64>),
}

impl NcValues {
    /// Creates `NC_CHAR` values holding the bytes of `s`.
    pub fn text(s: &str) -> Self {
        NcValues::Char(s.as_bytes().to_vec())
    }

    /// The NetCDF type of these values.
    pub fn nc_type(&self) -> NcType {
        match self {
            NcValues::Byte(_) => NcType::Byte,
            NcValues::Char(_) => NcType::Char,
            NcValues::Short(_) => NcType::Short,
            NcValues::Int(_) => NcType::Int,
            NcValues::Float(_) => NcType::Float,
            NcValues::Double(_) => NcType::Double,
        }
    }

    /// The number of elements.
    pub fn len(&self) -> usize {
        match self {
            NcValues::Byte(v) => v.len(),
            NcValues::Char(v) => v.len(),
            NcValues::Short(v) => v.len(),
            NcValues::Int(v) => v.len(),
            NcValues::Float(v) => v.len(),
            NcValues::Double(v) => v.len(),
        }
    }

    /// True when there are no elements.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The text of `NC_CHAR` values, if they are valid UTF-8.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            NcValues::Char(bytes) => core::str::from_utf8(bytes).ok(),
            _ => None,
        }
    }

    /// Converts numeric values to `f64`. Fails for `NC_CHAR`.
    pub fn to_f64_vec(&self) -> Result<Vec<f64>> {
        Ok(match self {
            NcValues::Byte(v) => v.iter().map(|&x| f64::from(x)).collect(),
            NcValues::Short(v) => v.iter().map(|&x| f64::from(x)).collect(),
            NcValues::Int(v) => v.iter().map(|&x| f64::from(x)).collect(),
            NcValues::Float(v) => v.iter().map(|&x| f64::from(x)).collect(),
            NcValues::Double(v) => v.clone(),
            NcValues::Char(_) => {
                return Err(Error::Unsupported(String::from(
                    "NC_CHAR values cannot be converted to f64",
                )));
            }
        })
    }

    /// Converts `values` to the requested type.
    ///
    /// Integer targets truncate toward zero and fail if the result is out of
    /// range or the input is not finite. Float targets round to nearest
    /// (`f32` conversion) and never fail. `NC_CHAR` cannot be built from
    /// numbers.
    pub fn from_f64(nc_type: NcType, values: &[f64]) -> Result<Self> {
        Ok(match nc_type {
            NcType::Byte => NcValues::Byte(to_ints(values, "NC_BYTE")?),
            NcType::Short => NcValues::Short(to_ints(values, "NC_SHORT")?),
            NcType::Int => NcValues::Int(to_ints(values, "NC_INT")?),
            NcType::Float => NcValues::Float(values.iter().map(|&v| v as f32).collect()),
            NcType::Double => NcValues::Double(values.to_vec()),
            NcType::Char => {
                return Err(Error::Unsupported(String::from(
                    "NC_CHAR values cannot be written from f64",
                )));
            }
        })
    }

    /// Appends the big-endian encoding of the values to `out`.
    pub(crate) fn encode_be(&self, out: &mut Vec<u8>) {
        match self {
            NcValues::Byte(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_be_bytes())),
            NcValues::Char(v) => out.extend_from_slice(v),
            NcValues::Short(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_be_bytes())),
            NcValues::Int(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_be_bytes())),
            NcValues::Float(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_be_bytes())),
            NcValues::Double(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_be_bytes())),
        }
    }

    /// Decodes big-endian `bytes` of type `nc_type`.
    pub(crate) fn decode_be(nc_type: NcType, bytes: &[u8]) -> Result<Self> {
        let size = nc_type.size();
        if !bytes.len().is_multiple_of(size) {
            return Err(Error::SizeMismatch {
                expected: bytes.len() - bytes.len() % size,
                actual: bytes.len(),
            });
        }
        Ok(match nc_type {
            NcType::Byte => NcValues::Byte(collect_be(bytes, i8::from_be_bytes)),
            NcType::Char => NcValues::Char(bytes.to_vec()),
            NcType::Short => NcValues::Short(collect_be(bytes, i16::from_be_bytes)),
            NcType::Int => NcValues::Int(collect_be(bytes, i32::from_be_bytes)),
            NcType::Float => NcValues::Float(collect_be(bytes, f32::from_be_bytes)),
            NcType::Double => NcValues::Double(collect_be(bytes, f64::from_be_bytes)),
        })
    }
}

/// A variable as described by the header. Data is read through [`Reader`].
#[derive(Debug, Clone, PartialEq)]
pub struct Variable {
    /// Variable name.
    pub name: String,
    /// Dimension ids into [`Reader::dimensions`], outermost first.
    pub dim_ids: Vec<usize>,
    /// Element type.
    pub nc_type: NcType,
    /// Variable attributes.
    pub attributes: Vec<Attribute>,
    /// Header `vsize`: bytes per variable (per record for record variables).
    pub vsize: u64,
    /// Byte offset of the variable's data (of record 0 for record variables).
    pub begin: u64,
    /// True when the first dimension is the unlimited dimension.
    pub is_record: bool,
}

impl Variable {
    /// Looks up a variable attribute by name.
    pub fn attribute(&self, name: &str) -> Option<&Attribute> {
        find_attribute(&self.attributes, name)
    }
}

/// Finds the attribute called `name` in `attrs`.
pub(crate) fn find_attribute<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a Attribute> {
    attrs.iter().find(|a| a.name == name)
}

/// Rounds `len` up to a multiple of four.
pub(crate) fn pad4(len: usize) -> usize {
    len.div_ceil(4) * 4
}

/// Product of `lens`, or `None` on overflow. The empty product is one.
pub(crate) fn product(mut lens: impl Iterator<Item = usize>) -> Option<usize> {
    lens.try_fold(1usize, usize::checked_mul)
}

/// Converts a header `u32` to `usize`.
pub(crate) fn to_usize(value: u32) -> Result<usize> {
    usize::try_from(value).map_err(|_| {
        Error::Unsupported(format!("value {value} exceeds the platform address width"))
    })
}

/// Fails if two entries share a name. `what` names the kind of entry.
pub(crate) fn check_unique<'a>(names: impl Iterator<Item = &'a str>, what: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name) {
            return Err(Error::InvalidMetadata(format!("duplicate {what} '{name}'")));
        }
    }
    Ok(())
}

/// Rejects names NetCDF cannot store: empty names and names containing `/`
/// or NUL.
pub(crate) fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains(['/', '\0']) {
        return Err(Error::InvalidMetadata(format!(
            "invalid NetCDF name {name:?}"
        )));
    }
    Ok(())
}

/// Decodes `bytes` as fixed-width big-endian values.
fn collect_be<T, const N: usize>(bytes: &[u8], convert: fn([u8; N]) -> T) -> Vec<T> {
    bytes
        .chunks_exact(N)
        .map(|chunk| {
            let mut array = [0u8; N];
            array.copy_from_slice(chunk);
            convert(array)
        })
        .collect()
}

/// Converts each finite `f64` to `T` by truncation, failing when out of range.
fn to_ints<T: TryFrom<i64>>(values: &[f64], label: &str) -> Result<Vec<T>> {
    values
        .iter()
        .map(|&v| {
            if !v.is_finite() || v <= -9.2e18 || v >= 9.2e18 {
                return Err(Error::InvalidMetadata(format!(
                    "value {v} cannot be stored as {label}"
                )));
            }
            T::try_from(v as i64).map_err(|_| {
                Error::InvalidMetadata(format!("value {v} is out of range for {label}"))
            })
        })
        .collect()
}
