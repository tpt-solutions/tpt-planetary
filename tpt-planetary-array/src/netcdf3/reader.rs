//! Header parsing and variable data access for NetCDF-3 files.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{
    Attribute, Dimension, Format, MAGIC, NcType, NcValues, STREAMING_NUMRECS, TAG_ATTRIBUTE,
    TAG_DIMENSION, TAG_VARIABLE, Variable, check_unique, find_attribute, pad4, product, to_usize,
};
use crate::error::{Error, Result};

/// A parsed NetCDF-3 file that borrows its bytes.
///
/// [`Reader::new`] validates the whole header, so the accessors never see
/// dangling dimension ids or impossible offsets. Variable data is checked
/// against the byte slice when it is read, so truncated files produce an
/// error from the read call rather than a panic.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    format: Format,
    numrecs: usize,
    dimensions: Vec<Dimension>,
    attributes: Vec<Attribute>,
    variables: Vec<Variable>,
    recsize: u64,
}

impl<'a> Reader<'a> {
    /// Parses the header of a CDF-1 or CDF-2 file held in `data`.
    ///
    /// Fails for malformed headers, CDF-5 and streaming files, and for
    /// variables whose declared size or offset is inconsistent with the header.
    pub fn new(data: &'a [u8]) -> Result<Self> {
        let mut cur = Cursor::new(data);
        let magic: [u8; 3] = cur.array()?;
        if magic != *MAGIC {
            return Err(Error::InvalidMetadata(String::from(
                "not a NetCDF-3 file (missing CDF magic)",
            )));
        }
        let format = match cur.array::<1>()?[0] {
            1 => Format::Classic,
            2 => Format::Offset64,
            5 => {
                return Err(Error::Unsupported(String::from(
                    "CDF-5 (64-bit data) files",
                )));
            }
            other => {
                return Err(Error::InvalidMetadata(format!(
                    "unknown NetCDF-3 version byte {other}"
                )));
            }
        };
        let numrecs_raw = cur.u32()?;
        if numrecs_raw == STREAMING_NUMRECS {
            return Err(Error::Unsupported(String::from(
                "streaming NetCDF-3 files (numrecs = 0xFFFFFFFF)",
            )));
        }
        let numrecs = to_usize(numrecs_raw)?;

        let dim_count = read_list_header(&mut cur, TAG_DIMENSION)?;
        let mut dimensions: Vec<Dimension> = Vec::new();
        let mut unlimited_seen = false;
        for _ in 0..dim_count {
            let name = cur.name()?;
            let declared = cur.u32()?;
            let unlimited = declared == 0;
            if unlimited {
                if unlimited_seen {
                    return Err(Error::InvalidMetadata(String::from(
                        "more than one unlimited dimension",
                    )));
                }
                unlimited_seen = true;
            }
            let len = if unlimited {
                numrecs
            } else {
                to_usize(declared)?
            };
            dimensions.push(Dimension {
                name,
                len,
                unlimited,
            });
        }
        check_unique(dimensions.iter().map(|d| d.name.as_str()), "dimension")?;

        let attributes = read_attributes(&mut cur)?;

        let var_count = read_list_header(&mut cur, TAG_VARIABLE)?;
        let mut variables: Vec<Variable> = Vec::new();
        let mut recsize: u64 = 0;
        for _ in 0..var_count {
            let name = cur.name()?;
            let ndims = cur.u32()?;
            let mut dim_ids = Vec::new();
            for _ in 0..ndims {
                let id = to_usize(cur.u32()?)?;
                if id >= dimensions.len() {
                    return Err(Error::InvalidMetadata(format!(
                        "variable '{name}' uses undefined dimension {id}"
                    )));
                }
                dim_ids.push(id);
            }
            let var_attributes = read_attributes(&mut cur)?;
            let nc_type = read_type(&mut cur)?;
            let vsize = u64::from(cur.u32()?);
            let begin = match format {
                Format::Classic => u64::from(cur.u32()?),
                Format::Offset64 => u64::from_be_bytes(cur.array()?),
            };
            for (position, &id) in dim_ids.iter().enumerate() {
                if dimensions[id].unlimited && position != 0 {
                    return Err(Error::InvalidMetadata(format!(
                        "variable '{name}' uses the unlimited dimension but not as its first"
                    )));
                }
            }
            let is_record = dim_ids.first().is_some_and(|&id| dimensions[id].unlimited);
            let skip = usize::from(is_record);
            let elements = product(dim_ids.iter().skip(skip).map(|&id| dimensions[id].len))
                .ok_or_else(|| overflow(&name))?;
            let needed = elements
                .checked_mul(nc_type.size())
                .ok_or_else(|| overflow(&name))?;
            let needed = u64::try_from(needed).map_err(|_| overflow(&name))?;
            if vsize < needed {
                return Err(Error::InvalidMetadata(format!(
                    "variable '{name}' has vsize {vsize} but needs {needed} bytes"
                )));
            }
            if is_record {
                recsize = recsize.checked_add(vsize).ok_or_else(|| overflow(&name))?;
            }
            variables.push(Variable {
                name,
                dim_ids,
                nc_type,
                attributes: var_attributes,
                vsize,
                begin,
                is_record,
            });
        }

        let header_end = u64::try_from(cur.pos())
            .map_err(|_| Error::Unsupported(String::from("header larger than 2^64 bytes")))?;
        for var in &variables {
            if var.begin < header_end {
                return Err(Error::InvalidMetadata(format!(
                    "variable '{}' data begins at {} inside the header",
                    var.name, var.begin
                )));
            }
        }
        check_unique(variables.iter().map(|v| v.name.as_str()), "variable")?;

        Ok(Self {
            data,
            format,
            numrecs,
            dimensions,
            attributes,
            variables,
            recsize,
        })
    }

    /// Whether the file is CDF-1 or CDF-2.
    pub fn format(&self) -> Format {
        self.format
    }

    /// Number of records along the unlimited dimension (zero if none).
    pub fn numrecs(&self) -> usize {
        self.numrecs
    }

    /// All dimensions, in header order.
    pub fn dimensions(&self) -> &[Dimension] {
        &self.dimensions
    }

    /// The dimension called `name`, if any.
    pub fn dimension(&self, name: &str) -> Option<&Dimension> {
        self.dimensions.iter().find(|d| d.name == name)
    }

    /// Global attributes, in header order.
    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    /// The global attribute called `name`, if any.
    pub fn attribute(&self, name: &str) -> Option<&Attribute> {
        find_attribute(&self.attributes, name)
    }

    /// All variables, in header order.
    pub fn variables(&self) -> &[Variable] {
        &self.variables
    }

    /// The variable called `name`, if any.
    pub fn variable(&self, name: &str) -> Option<&Variable> {
        self.variables.iter().find(|v| v.name == name)
    }

    /// The shape of variable `name`. The unlimited dimension reports the
    /// number of records.
    pub fn shape(&self, name: &str) -> Result<Vec<usize>> {
        let var = self.require(name)?;
        Ok(var
            .dim_ids
            .iter()
            .map(|&id| self.dimensions[id].len)
            .collect())
    }

    /// Reads all values of variable `name` in its native type, in C order
    /// (for record variables, record by record).
    pub fn read_values(&self, name: &str) -> Result<NcValues> {
        let var = self.require(name)?;
        let raw = self.read_raw(var)?;
        NcValues::decode_be(var.nc_type, &raw)
    }

    /// Reads all values of variable `name` converted to `f64`.
    pub fn read_f64(&self, name: &str) -> Result<Vec<f64>> {
        self.read_values(name)?.to_f64_vec()
    }

    /// Reads an `NC_INT` variable. Fails if the variable has another type.
    pub fn read_i32(&self, name: &str) -> Result<Vec<i32>> {
        match self.read_values(name)? {
            NcValues::Int(values) => Ok(values),
            other => Err(type_mismatch(name, other.nc_type(), NcType::Int)),
        }
    }

    /// Reads an `NC_FLOAT` variable. Fails if the variable has another type.
    pub fn read_f32(&self, name: &str) -> Result<Vec<f32>> {
        match self.read_values(name)? {
            NcValues::Float(values) => Ok(values),
            other => Err(type_mismatch(name, other.nc_type(), NcType::Float)),
        }
    }

    fn require(&self, name: &str) -> Result<&Variable> {
        self.variable(name)
            .ok_or_else(|| Error::NotFound(format!("variable '{name}'")))
    }

    /// Element count of `var` excluding the unlimited dimension when
    /// `skip_record` is true.
    fn elements(&self, var: &Variable, skip_record: bool) -> Result<usize> {
        let skip = usize::from(skip_record && var.is_record);
        product(
            var.dim_ids
                .iter()
                .skip(skip)
                .map(|&id| self.dimensions[id].len),
        )
        .ok_or_else(|| overflow(&var.name))
    }

    /// Concatenated big-endian bytes of every value of `var`.
    fn read_raw(&self, var: &Variable) -> Result<Vec<u8>> {
        let size = var.nc_type.size();
        let begin = to_usize_u64(var.begin)?;
        if !var.is_record {
            let bytes = self
                .elements(var, false)?
                .checked_mul(size)
                .ok_or_else(|| overflow(&var.name))?;
            return Ok(self.slice(begin, bytes)?.to_vec());
        }

        let per_record = self
            .elements(var, true)?
            .checked_mul(size)
            .ok_or_else(|| overflow(&var.name))?;
        if self.numrecs == 0 {
            return Ok(Vec::new());
        }
        let recsize = to_usize_u64(self.recsize)?;
        let record_start = |r: usize| -> Result<usize> {
            r.checked_mul(recsize)
                .and_then(|offset| offset.checked_add(begin))
                .ok_or_else(|| overflow(&var.name))
        };
        // Check the span of the last record before allocating the output.
        self.slice(record_start(self.numrecs - 1)?, per_record)?;
        let total = self
            .numrecs
            .checked_mul(per_record)
            .ok_or_else(|| overflow(&var.name))?;
        let mut out = Vec::with_capacity(total);
        for r in 0..self.numrecs {
            out.extend_from_slice(self.slice(record_start(r)?, per_record)?);
        }
        Ok(out)
    }

    /// Bounds-checked sub-slice of the file.
    fn slice(&self, start: usize, len: usize) -> Result<&'a [u8]> {
        let end = start
            .checked_add(len)
            .ok_or_else(|| truncated_data(start, len))?;
        self.data
            .get(start..end)
            .ok_or_else(|| truncated_data(start, len))
    }
}

/// Converts a header offset to `usize`.
fn to_usize_u64(value: u64) -> Result<usize> {
    usize::try_from(value).map_err(|_| {
        Error::Unsupported(format!("offset {value} exceeds the platform address width"))
    })
}

fn overflow(name: &str) -> Error {
    Error::InvalidMetadata(format!("size of variable '{name}' overflows"))
}

fn truncated_data(start: usize, len: usize) -> Error {
    Error::InvalidMetadata(format!(
        "data of {len} bytes at offset {start} lies beyond the end of the file"
    ))
}

fn type_mismatch(name: &str, actual: NcType, wanted: NcType) -> Error {
    Error::Unsupported(format!(
        "variable '{name}' has type {actual:?}, not {wanted:?}"
    ))
}

/// Reads `ABSENT` (`0 0`) or `tag count` and returns the element count.
fn read_list_header(cur: &mut Cursor<'_>, tag: u32) -> Result<u32> {
    let found = cur.u32()?;
    let count = cur.u32()?;
    if found == 0 {
        if count != 0 {
            return Err(Error::InvalidMetadata(String::from(
                "ABSENT list with nonzero count",
            )));
        }
        return Ok(0);
    }
    if found != tag {
        return Err(Error::InvalidMetadata(format!(
            "expected list tag {tag:#x}, found {found:#x}"
        )));
    }
    Ok(count)
}

fn read_type(cur: &mut Cursor<'_>) -> Result<NcType> {
    let tag = cur.u32()?;
    NcType::from_tag(tag)
        .ok_or_else(|| Error::InvalidMetadata(format!("unknown NetCDF type code {tag}")))
}

fn read_attributes(cur: &mut Cursor<'_>) -> Result<Vec<Attribute>> {
    let count = read_list_header(cur, TAG_ATTRIBUTE)?;
    let mut out: Vec<Attribute> = Vec::new();
    for _ in 0..count {
        let name = cur.name()?;
        let nc_type = read_type(cur)?;
        let nelems = to_usize(cur.u32()?)?;
        let bytes = nelems
            .checked_mul(nc_type.size())
            .ok_or_else(|| overflow(&name))?;
        let raw = cur.take(bytes)?;
        cur.take(pad4(bytes) - bytes)?;
        let values = NcValues::decode_be(nc_type, raw)?;
        out.push(Attribute { name, values });
    }
    check_unique(out.iter().map(|a| a.name.as_str()), "attribute")?;
    Ok(out)
}

/// Bounds-checked reader over the header bytes.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn pos(&self) -> usize {
        self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let truncated = || {
            Error::InvalidMetadata(format!(
                "unexpected end of NetCDF-3 header at byte {}",
                self.pos
            ))
        };
        let end = self.pos.checked_add(n).ok_or_else(truncated)?;
        let bytes = self.data.get(self.pos..end).ok_or_else(truncated)?;
        self.pos = end;
        Ok(bytes)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let bytes = self.take(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(bytes);
        Ok(out)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    /// Reads a padded name: a `u32` length, the UTF-8 bytes, then zero padding.
    fn name(&mut self) -> Result<String> {
        let len = to_usize(self.u32()?)?;
        let bytes = self.take(len)?;
        self.take(pad4(len) - len)?;
        let name = core::str::from_utf8(bytes)
            .map_err(|_| Error::InvalidMetadata(String::from("name is not valid UTF-8")))?;
        if name.is_empty() {
            return Err(Error::InvalidMetadata(String::from("empty name")));
        }
        Ok(String::from(name))
    }
}
