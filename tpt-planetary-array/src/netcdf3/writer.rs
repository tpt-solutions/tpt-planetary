//! Serialization of NetCDF-3 files into memory.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{
    Attribute, Dimension, Format, MAGIC, NcType, NcValues, TAG_ATTRIBUTE, TAG_DIMENSION,
    TAG_VARIABLE, check_unique, pad4, product, validate_name,
};
use crate::error::{Error, Result};

/// Builds a NetCDF-3 file in memory.
///
/// Add dimensions first, then global attributes and variables, and call
/// [`Writer::finish`] to obtain the bytes. Variables that use the unlimited
/// dimension must list it first; all record variables must hold the same
/// number of records. The file layout is fixed variables first, then the
/// record section with records interleaved, as netCDF-C writes it.
#[derive(Debug, Clone)]
pub struct Writer {
    format: Format,
    dimensions: Vec<Dimension>,
    attributes: Vec<Attribute>,
    variables: Vec<PendingVariable>,
}

#[derive(Debug, Clone)]
struct PendingVariable {
    name: String,
    dim_ids: Vec<usize>,
    attributes: Vec<Attribute>,
    values: NcValues,
    is_record: bool,
    /// Elements per record for record variables, total elements otherwise.
    elements: usize,
}

impl Writer {
    /// Creates an empty writer for the given format.
    pub fn new(format: Format) -> Self {
        Self {
            format,
            dimensions: Vec::new(),
            attributes: Vec::new(),
            variables: Vec::new(),
        }
    }

    /// Adds a dimension and returns its id.
    ///
    /// `Some(len)` declares a fixed dimension (`len >= 1`). `None` declares
    /// the unlimited dimension; a file may have at most one.
    pub fn add_dimension(&mut self, name: &str, len: Option<usize>) -> Result<usize> {
        validate_name(name)?;
        if self.dimensions.iter().any(|d| d.name == name) {
            return Err(Error::InvalidMetadata(format!(
                "duplicate dimension '{name}'"
            )));
        }
        match len {
            Some(0) => {
                return Err(Error::Unsupported(String::from(
                    "fixed dimension length must be at least 1 (zero means unlimited)",
                )));
            }
            Some(n) => {
                u32_of(n, "dimension length")?;
            }
            None => {
                if self.dimensions.iter().any(|d| d.unlimited) {
                    return Err(Error::Unsupported(String::from(
                        "a NetCDF-3 file has at most one unlimited dimension",
                    )));
                }
            }
        }
        self.dimensions.push(Dimension {
            name: String::from(name),
            len: len.unwrap_or(0),
            unlimited: len.is_none(),
        });
        Ok(self.dimensions.len() - 1)
    }

    /// Adds a global attribute.
    pub fn add_attribute(&mut self, attribute: Attribute) {
        self.attributes.push(attribute);
    }

    /// Adds a variable with the given dimension ids (outermost first), its
    /// attributes and its values, and returns the variable's index.
    ///
    /// For a fixed variable `values` must hold the product of the dimension
    /// lengths. For a record variable (first dimension unlimited) it must hold
    /// a whole number of records.
    pub fn add_variable(
        &mut self,
        name: &str,
        dim_ids: &[usize],
        attributes: Vec<Attribute>,
        values: NcValues,
    ) -> Result<usize> {
        validate_name(name)?;
        if self.variables.iter().any(|v| v.name == name) {
            return Err(Error::InvalidMetadata(format!(
                "duplicate variable '{name}'"
            )));
        }
        for (position, &id) in dim_ids.iter().enumerate() {
            let dim = self
                .dimensions
                .get(id)
                .ok_or_else(|| Error::InvalidMetadata(format!("undefined dimension id {id}")))?;
            if dim.unlimited && position != 0 {
                return Err(Error::Unsupported(format!(
                    "variable '{name}': the unlimited dimension must be first"
                )));
            }
        }
        let is_record = dim_ids
            .first()
            .is_some_and(|&id| self.dimensions[id].unlimited);
        let skip = usize::from(is_record);
        let elements = product(dim_ids.iter().skip(skip).map(|&id| self.dimensions[id].len))
            .ok_or_else(|| Error::Unsupported(format!("variable '{name}' is too large")))?;
        let size = values.nc_type().size();
        if is_record {
            if elements == 0 || !values.len().is_multiple_of(elements) {
                return Err(Error::InvalidMetadata(format!(
                    "variable '{name}' has {} values, which is not a whole number of records of {elements}",
                    values.len()
                )));
            }
        } else {
            let expected = product(dim_ids.iter().map(|&id| self.dimensions[id].len))
                .ok_or_else(|| Error::Unsupported(format!("variable '{name}' is too large")))?;
            if values.len() != expected {
                return Err(Error::SizeMismatch {
                    expected: expected * size,
                    actual: values.len() * size,
                });
            }
        }
        self.variables.push(PendingVariable {
            name: String::from(name),
            dim_ids: dim_ids.to_vec(),
            attributes,
            values,
            is_record,
            elements,
        });
        Ok(self.variables.len() - 1)
    }

    /// Adds a variable of type `nc_type` whose values are converted from `f64`.
    /// See [`NcValues::from_f64`] for the conversion rules.
    pub fn add_variable_f64(
        &mut self,
        name: &str,
        dim_ids: &[usize],
        attributes: Vec<Attribute>,
        nc_type: NcType,
        values: &[f64],
    ) -> Result<usize> {
        let values = NcValues::from_f64(nc_type, values)?;
        self.add_variable(name, dim_ids, attributes, values)
    }

    /// Serializes the file.
    pub fn finish(self) -> Result<Vec<u8>> {
        check_unique(self.dimensions.iter().map(|d| d.name.as_str()), "dimension")?;
        check_unique(self.attributes.iter().map(|a| a.name.as_str()), "attribute")?;
        check_unique(self.variables.iter().map(|v| v.name.as_str()), "variable")?;
        for attr in &self.attributes {
            validate_name(&attr.name)?;
        }
        for var in &self.variables {
            check_unique(var.attributes.iter().map(|a| a.name.as_str()), "attribute")?;
            for attr in &var.attributes {
                validate_name(&attr.name)?;
            }
        }

        let mut numrecs: Option<usize> = None;
        for var in self.variables.iter().filter(|v| v.is_record) {
            let records = var.values.len() / var.elements;
            match numrecs {
                Some(previous) if previous != records => {
                    return Err(Error::InvalidMetadata(format!(
                        "record variable '{}' has {records} records, others have {previous}",
                        var.name
                    )));
                }
                _ => numrecs = Some(records),
            }
        }
        let numrecs = numrecs.unwrap_or(0);
        let numrecs_u32 = u32_of(numrecs, "record count")?;
        if numrecs_u32 == u32::MAX {
            return Err(Error::Unsupported(String::from(
                "record count collides with the streaming marker",
            )));
        }

        // Variable sizes: padded to four bytes, except the lone record variable
        // of a 1- or 2-byte type, whose record is stored unpadded.
        let record_vars = self.variables.iter().filter(|v| v.is_record).count();
        let mut vsizes: Vec<u32> = Vec::with_capacity(self.variables.len());
        for var in &self.variables {
            let size = var.values.nc_type().size();
            let bytes = var.elements.checked_mul(size).ok_or_else(|| {
                Error::Unsupported(format!("variable '{}' is too large", var.name))
            })?;
            let vsize = if var.is_record && record_vars == 1 && size < 4 {
                bytes
            } else {
                pad4(bytes)
            };
            vsizes.push(u32_of(vsize, "variable size")?);
        }

        // Header length does not depend on the offset values, so measure it
        // with placeholders, then write the real offsets.
        let placeholder = alloc::vec![0u64; self.variables.len()];
        let mut header = Vec::new();
        self.write_header(&mut header, numrecs_u32, &placeholder, &vsizes)?;
        let header_len = header.len() as u64;

        let mut begins = alloc::vec![0u64; self.variables.len()];
        let mut cursor = header_len;
        for (i, var) in self.variables.iter().enumerate() {
            if !var.is_record {
                begins[i] = cursor;
                cursor += u64::from(vsizes[i]);
            }
        }
        let record_base = cursor;
        let mut in_record = 0u64;
        for (i, var) in self.variables.iter().enumerate() {
            if var.is_record {
                begins[i] = record_base + in_record;
                in_record += u64::from(vsizes[i]);
            }
        }

        header.clear();
        self.write_header(&mut header, numrecs_u32, &begins, &vsizes)?;

        let encoded: Vec<Vec<u8>> = self
            .variables
            .iter()
            .map(|var| {
                let mut bytes = Vec::new();
                var.values.encode_be(&mut bytes);
                bytes
            })
            .collect();

        let mut out = header;
        for (i, var) in self.variables.iter().enumerate() {
            if !var.is_record {
                out.extend_from_slice(&encoded[i]);
                pad_to(&mut out, encoded[i].len(), vsizes[i] as usize);
            }
        }
        for record in 0..numrecs {
            for (i, var) in self.variables.iter().enumerate() {
                if var.is_record {
                    let per_record = var.elements * var.values.nc_type().size();
                    let start = record * per_record;
                    out.extend_from_slice(&encoded[i][start..start + per_record]);
                    pad_to(&mut out, per_record, vsizes[i] as usize);
                }
            }
        }
        Ok(out)
    }

    fn write_header(
        &self,
        out: &mut Vec<u8>,
        numrecs: u32,
        begins: &[u64],
        vsizes: &[u32],
    ) -> Result<()> {
        out.extend_from_slice(MAGIC);
        out.push(self.format.version());
        push_u32(out, numrecs);

        if self.dimensions.is_empty() {
            push_u32(out, 0);
            push_u32(out, 0);
        } else {
            push_u32(out, TAG_DIMENSION);
            push_u32(out, u32_of(self.dimensions.len(), "dimension count")?);
            for dim in &self.dimensions {
                push_name(out, &dim.name)?;
                let declared = if dim.unlimited {
                    0
                } else {
                    u32_of(dim.len, "dimension length")?
                };
                push_u32(out, declared);
            }
        }

        push_attributes(out, &self.attributes)?;

        if self.variables.is_empty() {
            push_u32(out, 0);
            push_u32(out, 0);
        } else {
            push_u32(out, TAG_VARIABLE);
            push_u32(out, u32_of(self.variables.len(), "variable count")?);
            for (i, var) in self.variables.iter().enumerate() {
                push_name(out, &var.name)?;
                push_u32(out, u32_of(var.dim_ids.len(), "dimension count")?);
                for &id in &var.dim_ids {
                    push_u32(out, u32_of(id, "dimension id")?);
                }
                push_attributes(out, &var.attributes)?;
                push_u32(out, var.values.nc_type().tag());
                push_u32(out, vsizes[i]);
                match self.format {
                    Format::Classic => {
                        let begin = u32::try_from(begins[i]).map_err(|_| {
                            Error::Unsupported(String::from(
                                "data offset exceeds 2^32 - 1; use Format::Offset64",
                            ))
                        })?;
                        push_u32(out, begin);
                    }
                    Format::Offset64 => out.extend_from_slice(&begins[i].to_be_bytes()),
                }
            }
        }
        Ok(())
    }
}

/// Converts `value` to `u32`, naming `what` in the error.
fn u32_of(value: usize, what: &str) -> Result<u32> {
    u32::try_from(value).map_err(|_| {
        Error::Unsupported(format!("{what} {value} exceeds the 32-bit NetCDF-3 limit"))
    })
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

/// Appends zero bytes so that `written` bytes occupy `total` bytes.
fn pad_to(out: &mut Vec<u8>, written: usize, total: usize) {
    out.resize(out.len() + total.saturating_sub(written), 0);
}

/// Writes a padded name: a `u32` length, the bytes, then zero padding.
fn push_name(out: &mut Vec<u8>, name: &str) -> Result<()> {
    let len = u32_of(name.len(), "name length")?;
    push_u32(out, len);
    out.extend_from_slice(name.as_bytes());
    pad_to(out, name.len(), pad4(name.len()));
    Ok(())
}

/// Writes an attribute list, or `ABSENT` when it is empty.
fn push_attributes(out: &mut Vec<u8>, attributes: &[Attribute]) -> Result<()> {
    if attributes.is_empty() {
        push_u32(out, 0);
        push_u32(out, 0);
        return Ok(());
    }
    push_u32(out, TAG_ATTRIBUTE);
    push_u32(out, u32_of(attributes.len(), "attribute count")?);
    for attr in attributes {
        push_name(out, &attr.name)?;
        push_u32(out, attr.values.nc_type().tag());
        push_u32(out, u32_of(attr.values.len(), "attribute length")?);
        let start = out.len();
        attr.values.encode_be(out);
        let written = out.len() - start;
        pad_to(out, written, pad4(written));
    }
    Ok(())
}
