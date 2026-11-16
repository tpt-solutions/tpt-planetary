//! `tptplan array convert`: conversion between local Zarr directories and
//! NetCDF-3 files.
//!
//! The direction is inferred from the input. A `.nc` file is read as NetCDF-3
//! and written as a Zarr group with one array per variable. A Zarr directory
//! (an array, or a group of arrays) is written as a NetCDF-3 file, which is
//! chosen by a `.nc` output path.
//!
//! Only numeric variables with at least one dimension are converted, and the
//! CF time decoding is out of scope, so time variables stay as plain numbers.
//! Supported element types are int8, int16, int32, float32 and float64 (NetCDF
//! byte, short, int, float and double). Unsigned and 64-bit integers, booleans,
//! text and NetCDF-4 files are reported as errors. Variables are read and
//! written whole, not streamed.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Number, Value};
use tpt_planetary_array::dtype::{Endian, Kind};
use tpt_planetary_array::meta::{ArrayMetadata, GroupMetadata, NodeMetadata, ZarrFormat};
use tpt_planetary_array::netcdf3::{Attribute, Format, NcType, NcValues, Reader, Writer};
use tpt_planetary_array::store::join;
use tpt_planetary_array::{Array, DType, FillValue, FsStore, Store, hierarchy};

use crate::array_cmd::{default_chunks, reject_remote, split_location};
use crate::{EXIT_RUNTIME, EXIT_USAGE, Failure};

/// Attribute that holds dimension names in Zarr v2 `.zattrs`.
const ARRAY_DIMENSIONS: &str = "_ARRAY_DIMENSIONS";
/// NetCDF attribute that marks missing values.
const FILL_VALUE: &str = "_FillValue";
/// Largest NetCDF-3 classic data section (variables plus records) in bytes.
const CLASSIC_DATA_LIMIT: u64 = i32::MAX as u64;

/// Direction of a conversion, inferred from the input location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    /// NetCDF-3 file to Zarr group.
    NetcdfToZarr,
    /// Zarr array or group to NetCDF-3 file.
    ZarrToNetcdf,
}

/// True when `path` names a NetCDF file by its `.nc` extension.
pub(crate) fn is_netcdf_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("nc"))
}

/// Maps a `--zarr-format` value to a Zarr format.
pub(crate) fn zarr_format(version: u8) -> ZarrFormat {
    match version {
        3 => ZarrFormat::V3,
        _ => ZarrFormat::V2,
    }
}

fn runtime(message: String) -> Failure {
    Failure::new(EXIT_RUNTIME, message)
}

fn usage(message: String) -> Failure {
    Failure::new(EXIT_USAGE, message)
}

/// Works out the conversion direction from the input and output paths.
fn direction(input: &str, output: &str) -> Result<Direction, Failure> {
    let meta = fs::metadata(input).map_err(|e| runtime(format!("cannot access '{input}': {e}")))?;
    let out_is_nc = is_netcdf_path(output);
    if meta.is_dir() {
        if out_is_nc {
            Ok(Direction::ZarrToNetcdf)
        } else {
            Err(usage(format!(
                "'{input}' is a Zarr directory; the output must be a NetCDF-3 file ending in .nc \
                 (Zarr to Zarr is not a conversion)"
            )))
        }
    } else if is_netcdf_path(input) {
        if out_is_nc {
            Err(usage(format!(
                "both '{input}' and '{output}' are NetCDF-3 files; the output must be a Zarr \
                 directory"
            )))
        } else {
            Ok(Direction::NetcdfToZarr)
        }
    } else {
        Err(usage(format!(
            "unrecognised input '{input}': expected a .nc file or a Zarr directory"
        )))
    }
}

/// Converts `input` to `output` and returns the summary line for stdout.
pub fn convert(input: &str, output: &str, version: u8, verbose: u8) -> Result<String, Failure> {
    reject_remote(input)?;
    reject_remote(output)?;
    let dir = direction(input, output)?;
    if Path::new(output).exists() {
        return Err(runtime(format!(
            "output '{output}' already exists; remove it or choose another path"
        )));
    }
    let format = zarr_format(version);
    let (count, what) = match dir {
        Direction::NetcdfToZarr => {
            if verbose > 0 {
                eprintln!("tptplan: reading NetCDF-3 file {input}");
            }
            let n = netcdf_to_zarr(input, output, format, verbose)?;
            (n, "Zarr group")
        }
        Direction::ZarrToNetcdf => {
            if verbose > 0 {
                eprintln!("tptplan: reading Zarr from {input}");
            }
            let n = zarr_to_netcdf(input, output, verbose)?;
            (n, "NetCDF-3 file")
        }
    };
    Ok(format!(
        "converted {count} variable{} from {input} to {what} {output}",
        if count == 1 { "" } else { "s" }
    ))
}

/// Element type of a NetCDF variable, or an error for types Zarr cannot hold
/// in this build.
fn nc_to_dtype(name: &str, nc_type: NcType) -> Result<DType, Failure> {
    let kind = match nc_type {
        NcType::Byte => Kind::Int(8),
        NcType::Short => Kind::Int(16),
        NcType::Int => Kind::Int(32),
        NcType::Float => Kind::Float(32),
        NcType::Double => Kind::Float(64),
        NcType::Char => {
            return Err(runtime(format!(
                "variable '{name}' is NC_CHAR (text); only numeric variables can be converted"
            )));
        }
    };
    DType::new(kind, Endian::Little).map_err(|e| runtime(format!("variable '{name}': {e}")))
}

/// NetCDF type for a Zarr element type, or an error naming the supported set.
fn dtype_to_nc(name: &str, dtype: &DType) -> Result<NcType, Failure> {
    match dtype.kind() {
        Kind::Int(8) => Ok(NcType::Byte),
        Kind::Int(16) => Ok(NcType::Short),
        Kind::Int(32) => Ok(NcType::Int),
        Kind::Float(32) => Ok(NcType::Float),
        Kind::Float(64) => Ok(NcType::Double),
        other => {
            let hint = match other {
                Kind::Int(64) => " (NetCDF-3 has no 64-bit integers)",
                Kind::UInt(_) => " (NetCDF-3 has no unsigned integers)",
                _ => "",
            };
            Err(runtime(format!(
                "array '{name}' has dtype {}, which NetCDF-3 cannot store{hint}; supported: \
                 int8, int16, int32, float32, float64",
                dtype.to_v2()
            )))
        }
    }
}

/// Dimension names for a variable, with generated names for unnamed axes.
/// Generated names are `dim{axis}_{length}`, which are unique per axis and length.
fn dimension_names(shape: &[u64], names: Option<&[Option<String>]>) -> Vec<String> {
    shape
        .iter()
        .enumerate()
        .map(|(axis, &len)| match names.and_then(|n| n.get(axis)) {
            Some(Some(name)) => name.clone(),
            _ => format!("dim{axis}_{len}"),
        })
        .collect()
}

/// Converts a NetCDF-3 attribute to Zarr JSON. Text becomes a string, numbers
/// become JSON numbers (or an array of numbers). Other values are skipped.
fn attribute_to_json(attr: &Attribute) -> Option<Value> {
    match &attr.values {
        NcValues::Char(bytes) => Some(Value::String(String::from_utf8_lossy(bytes).into_owned())),
        values => {
            let numbers = values.to_f64_vec().ok()?;
            let json: Vec<Value> = numbers
                .iter()
                .map(|&x| Number::from_f64(x).map_or(Value::Null, Value::Number))
                .collect();
            if json.len() == 1 {
                json.into_iter().next()
            } else {
                Some(Value::Array(json))
            }
        }
    }
}

/// Converts a Zarr JSON attribute to NetCDF-3 when it is a string or a number
/// (or an array of numbers). Returns `None` for anything else.
fn json_to_attribute(name: &str, value: &Value) -> Option<Attribute> {
    match value {
        Value::String(s) => Some(Attribute::text(name, s)),
        Value::Number(n) => Some(Attribute::new(name, NcValues::Double(vec![n.as_f64()?]))),
        Value::Array(items) => {
            let numbers: Option<Vec<f64>> = items.iter().map(Value::as_f64).collect();
            let numbers = numbers?;
            (!numbers.is_empty()).then(|| Attribute::new(name, NcValues::Double(numbers)))
        }
        _ => None,
    }
}

fn numeric_fill(attr: &Attribute) -> Option<f64> {
    match &attr.values {
        NcValues::Char(_) => None,
        values => values.to_f64_vec().ok()?.first().copied(),
    }
}

fn warn_skipped(variable: &str, attribute: &str) {
    eprintln!(
        "tptplan: warning: attribute '{attribute}' of '{variable}' skipped (unsupported type)"
    );
}

/// Writes the NetCDF-3 variables of `reader` as arrays of a Zarr group at
/// `output`. Returns the number of variables written.
fn netcdf_to_zarr(
    input: &str,
    output: &str,
    format: ZarrFormat,
    verbose: u8,
) -> Result<usize, Failure> {
    let bytes = fs::read(input).map_err(|e| runtime(format!("cannot read '{input}': {e}")))?;
    let reader = Reader::new(&bytes)
        .map_err(|e| runtime(format!("cannot read NetCDF-3 file '{input}': {e}")))?;

    let (parent, name) = split_location(output)?;
    let store: Arc<dyn Store> =
        Arc::new(FsStore::open(&parent).map_err(|e| runtime(format!("cannot open store: {e}")))?);
    hierarchy::write_group(
        store.as_ref(),
        &name,
        format,
        &GroupMetadata {
            format: Some(format),
            attributes: Map::new(),
        },
    )
    .map_err(|e| runtime(format!("cannot create Zarr group '{output}': {e}")))?;

    for var in reader.variables() {
        let dtype = nc_to_dtype(&var.name, var.nc_type)?;
        let shape: Vec<u64> = reader
            .shape(&var.name)
            .map_err(|e| runtime(format!("variable '{}': {e}", var.name)))?
            .iter()
            .map(|&n| n as u64)
            .collect();
        if shape.is_empty() {
            return Err(runtime(format!(
                "variable '{}' is a scalar; Zarr arrays need at least one dimension",
                var.name
            )));
        }
        if shape.contains(&0) {
            return Err(runtime(format!(
                "variable '{}' has a zero-length dimension, which cannot be converted",
                var.name
            )));
        }
        if verbose > 0 {
            eprintln!("tptplan: writing array '{}' with shape {shape:?}", var.name);
        }

        let names: Vec<String> = var
            .dim_ids
            .iter()
            .map(|&id| reader.dimensions()[id].name.clone())
            .collect();
        let values = reader
            .read_values(&var.name)
            .and_then(|v| v.to_f64_vec())
            .map_err(|e| runtime(format!("variable '{}': {e}", var.name)))?;

        let mut attrs = Map::new();
        let mut fill = FillValue::Null;
        for attr in &var.attributes {
            if attr.name == FILL_VALUE
                && let Some(x) = numeric_fill(attr)
            {
                fill = match dtype.kind() {
                    Kind::Int(_) if x.fract() == 0.0 => FillValue::Int(x as i64),
                    _ => FillValue::Float(x),
                };
            }
            match attribute_to_json(attr) {
                Some(json) => {
                    attrs.insert(attr.name.clone(), json);
                }
                None => warn_skipped(&var.name, &attr.name),
            }
        }

        let chunk = default_chunks(&shape);
        let meta = ArrayMetadata::new(format, shape.clone(), chunk, dtype)
            .map_err(|e| runtime(format!("variable '{}': {e}", var.name)))?
            .with_fill_value(fill)
            .with_dimension_names(names.into_iter().map(Some).collect())
            .with_attributes(attrs);
        let path = join(&name, &var.name);
        let array = Array::create(store.clone(), &path, meta)
            .map_err(|e| runtime(format!("cannot create array '{}': {e}", var.name)))?;
        let selection = vec![tpt_planetary_array::Selection::ALL; shape.len()];
        array
            .write_f64(&selection, &values)
            .map_err(|e| runtime(format!("cannot write array '{}': {e}", var.name)))?;
    }
    Ok(reader.variables().len())
}

/// A Zarr array to be written as a NetCDF-3 variable.
struct Planned {
    name: String,
    array: Array,
    nc_type: NcType,
    dims: Vec<(String, usize)>,
    attributes: Vec<Attribute>,
}

/// The (variable name, store path) of each array to convert.
type ArrayList = Vec<(String, String)>;

/// Opens the Zarr node at `input`: a single array, or the arrays directly
/// inside a group. Returns the store and the arrays.
fn zarr_arrays(input: &str) -> Result<(Arc<dyn Store>, ArrayList), Failure> {
    let (parent, name) = split_location(input)?;
    let store: Arc<dyn Store> =
        Arc::new(FsStore::open(&parent).map_err(|e| runtime(format!("cannot open store: {e}")))?);
    let node = hierarchy::read_node(store.as_ref(), &name)
        .map_err(|e| runtime(format!("cannot read metadata of '{input}': {e}")))?;
    let arrays = match node {
        Some(NodeMetadata::Array(_)) => {
            let var = name.strip_suffix(".zarr").unwrap_or(&name).to_string();
            vec![(var, name.clone())]
        }
        Some(NodeMetadata::Group(_)) => {
            let prefix = format!("{name}/");
            let mut out = Vec::new();
            for entry in hierarchy::list_nodes(store.as_ref(), &name)
                .map_err(|e| runtime(format!("cannot list '{input}': {e}")))?
            {
                let Some(child) = entry.path.strip_prefix(&prefix) else {
                    continue;
                };
                if child.contains('/') {
                    continue;
                }
                if entry.is_array {
                    out.push((child.to_string(), entry.path.clone()));
                } else {
                    return Err(usage(format!(
                        "'{input}' contains the nested group '{child}'; only a group of arrays \
                         can be converted"
                    )));
                }
            }
            if out.is_empty() {
                return Err(runtime(format!("group '{input}' contains no arrays")));
            }
            out
        }
        None => {
            return Err(runtime(format!(
                "'{input}' has no Zarr metadata (zarr.json, .zarray or .zgroup)"
            )));
        }
    };
    Ok((store, arrays))
}

/// Writes the Zarr arrays of `input` as NetCDF-3 variables in `output`.
/// Returns the number of variables written.
fn zarr_to_netcdf(input: &str, output: &str, verbose: u8) -> Result<usize, Failure> {
    let (store, arrays) = zarr_arrays(input)?;

    // First pass: open arrays, check types and collect dimensions, so that the
    // format (classic or 64-bit offset) can be chosen from the total size.
    let mut planned: Vec<Planned> = Vec::new();
    let mut total_bytes: u64 = 0;
    for (var, path) in arrays {
        let array = Array::open(store.clone(), &path)
            .map_err(|e| runtime(format!("cannot open array '{var}': {e}")))?;
        let meta = array.metadata().clone();
        let nc_type = dtype_to_nc(&var, &meta.dtype)?;
        if meta.shape.is_empty() {
            return Err(runtime(format!(
                "array '{var}' is a scalar; NetCDF variables need at least one dimension"
            )));
        }
        if meta.shape.contains(&0) {
            return Err(runtime(format!(
                "array '{var}' has a zero-length dimension, which cannot be converted"
            )));
        }
        let names = dimension_names(&meta.shape, meta.dimension_names.as_deref());
        let dims = names
            .into_iter()
            .zip(meta.shape.iter())
            .map(|(n, &len)| (n, len as usize))
            .collect();
        let mut attributes = Vec::new();
        for (key, value) in &meta.attributes {
            if key == ARRAY_DIMENSIONS {
                continue;
            }
            match json_to_attribute(key, value) {
                Some(attr) => attributes.push(attr),
                None => warn_skipped(&var, key),
            }
        }
        if let Some(x) = meta.fill_value.as_f64()
            && !attributes.iter().any(|a| a.name == FILL_VALUE)
        {
            let values = NcValues::from_f64(nc_type, &[x])
                .map_err(|e| runtime(format!("array '{var}': {e}")))?;
            attributes.push(Attribute::new(FILL_VALUE, values));
        }
        let elements: u64 = meta.shape.iter().product();
        total_bytes += elements * nc_type.size() as u64;
        planned.push(Planned {
            name: var,
            array,
            nc_type,
            dims,
            attributes,
        });
    }

    let format = if total_bytes > CLASSIC_DATA_LIMIT {
        Format::Offset64
    } else {
        Format::Classic
    };
    if verbose > 0 && format == Format::Offset64 {
        eprintln!("tptplan: data exceeds 2 GiB, writing 64-bit offset NetCDF-3");
    }

    // Second pass: dimensions and variables. A name used for two different
    // lengths is an error, because NetCDF-3 dimensions are global.
    let mut writer = Writer::new(format);
    let mut dim_ids: Vec<(String, usize, usize)> = Vec::new();
    for item in &planned {
        let mut ids = Vec::with_capacity(item.dims.len());
        for (dim_name, len) in &item.dims {
            let existing = dim_ids.iter().find(|(n, _, _)| n == dim_name);
            let id = match existing {
                Some((_, known_len, id)) if known_len == len => *id,
                Some((_, known_len, _)) => {
                    return Err(runtime(format!(
                        "dimension '{dim_name}' has length {known_len} in one array and {len} \
                         in '{}'; NetCDF-3 dimensions must agree",
                        item.name
                    )));
                }
                None => {
                    let id = writer
                        .add_dimension(dim_name, Some(*len))
                        .map_err(|e| runtime(format!("dimension '{dim_name}': {e}")))?;
                    dim_ids.push((dim_name.clone(), *len, id));
                    id
                }
            };
            ids.push(id);
        }
        let selection = vec![tpt_planetary_array::Selection::ALL; item.dims.len()];
        let values = item
            .array
            .read_f64(&selection)
            .and_then(|v| NcValues::from_f64(item.nc_type, &v))
            .map_err(|e| runtime(format!("array '{}': {e}", item.name)))?;
        writer
            .add_variable(&item.name, &ids, item.attributes.clone(), values)
            .map_err(|e| runtime(format!("variable '{}': {e}", item.name)))?;
    }
    let bytes = writer
        .finish()
        .map_err(|e| runtime(format!("cannot serialise NetCDF-3 file: {e}")))?;
    fs::write(Path::new(output), &bytes)
        .map_err(|e| runtime(format!("cannot write '{output}': {e}")))?;
    Ok(planned.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_is_inferred_from_extensions() {
        assert!(is_netcdf_path("data/in.NC"));
        assert!(!is_netcdf_path("data/in.zarr"));
        assert!(!is_netcdf_path("data/in.nc4"));
    }

    #[test]
    fn unsupported_dtypes_are_named() {
        let err = dtype_to_nc("a", &DType::INT64).unwrap_err();
        assert_eq!(err.code, EXIT_RUNTIME);
        assert!(err.message.contains("no 64-bit integers"));
        let err = dtype_to_nc("a", &DType::UINT64).unwrap_err();
        assert!(err.message.contains("unsigned"));
        assert!(nc_to_dtype("t", NcType::Char).is_err());
    }

    #[test]
    fn generated_dimension_names_are_stable() {
        assert_eq!(
            dimension_names(&[2, 3], Some(&[Some("y".to_string()), None])),
            vec!["y".to_string(), "dim1_3".to_string()]
        );
    }

    #[test]
    fn attributes_round_trip_text_and_numbers() {
        let text = Attribute::text("units", "K");
        assert_eq!(attribute_to_json(&text), Some(Value::String("K".into())));
        let back = json_to_attribute("units", &Value::String("K".into())).unwrap();
        assert_eq!(back.values, NcValues::text("K"));
        assert!(json_to_attribute("flag", &Value::Bool(true)).is_none());
    }
}
