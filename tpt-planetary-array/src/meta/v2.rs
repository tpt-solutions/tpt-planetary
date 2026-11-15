//! Zarr v2 metadata: `.zarray`, `.zgroup`, `.zattrs` and consolidated
//! `.zmetadata`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::{Map, Value, json};

use super::{
    ARRAY_DIMENSIONS_ATTR, ArrayMetadata, CodecSpec, GroupMetadata, ZarrFormat,
    dimension_names_from, dimension_names_to_json, field, parse_object, u64_vec,
};
use crate::dtype::DType;
use crate::error::{Error, Result};
use crate::fill::FillValue;
use crate::grid::ChunkKeyEncoding;

/// Key of the array metadata document.
pub const ZARRAY: &str = ".zarray";
/// Key of the group metadata document.
pub const ZGROUP: &str = ".zgroup";
/// Key of the attributes document.
pub const ZATTRS: &str = ".zattrs";
/// Key of the consolidated metadata document.
pub const ZMETADATA: &str = ".zmetadata";

/// Codec identifiers that act as the (last) compressor rather than a filter.
const COMPRESSOR_IDS: &[&str] = &["gzip", "zlib", "lz4", "zstd", "blosc", "bz2", "lzma"];

/// Parses a `.zarray` document and optional `.zattrs` document.
pub fn parse_array(zarray: &[u8], zattrs: Option<&[u8]>) -> Result<ArrayMetadata> {
    let obj = parse_object(zarray, ZARRAY)?;
    match field(&obj, "zarr_format")?.as_u64() {
        Some(2) => {}
        _ => {
            return Err(Error::InvalidMetadata(
                "'.zarray' zarr_format must be 2".into(),
            ));
        }
    }
    let shape = u64_vec(field(&obj, "shape")?, "shape")?;
    let chunk_shape = u64_vec(field(&obj, "chunks")?, "chunks")?;
    let dtype_str = field(&obj, "dtype")?
        .as_str()
        .ok_or_else(|| Error::Unsupported("structured dtypes are not supported".into()))?;
    let dtype = DType::from_v2(dtype_str)?;

    if let Some(order) = obj.get("order").and_then(Value::as_str)
        && order != "C"
    {
        return Err(Error::Unsupported(format!("array order '{order}'")));
    }

    let fill_value = FillValue::from_json(field(&obj, "fill_value")?)?;

    let mut codecs = Vec::new();
    if let Some(filters) = obj.get("filters").filter(|v| !v.is_null()) {
        let arr = filters
            .as_array()
            .ok_or_else(|| Error::InvalidMetadata("'filters' must be an array or null".into()))?;
        for f in arr {
            codecs.push(spec_from_v2(f)?);
        }
    }
    if let Some(comp) = obj.get("compressor").filter(|v| !v.is_null()) {
        codecs.push(spec_from_v2(comp)?);
    }

    let separator = match obj.get("dimension_separator").and_then(Value::as_str) {
        None | Some(".") => '.',
        Some("/") => '/',
        Some(other) => {
            return Err(Error::InvalidMetadata(format!(
                "dimension_separator '{other}'"
            )));
        }
    };

    let attributes = match zattrs {
        Some(bytes) => parse_object(bytes, ZATTRS)?,
        None => Map::new(),
    };
    let dimension_names = match attributes.get(ARRAY_DIMENSIONS_ATTR) {
        Some(v) => Some(dimension_names_from(v)?),
        None => None,
    };

    let mut meta = ArrayMetadata::new(ZarrFormat::V2, shape, chunk_shape, dtype)?
        .with_fill_value(fill_value)
        .with_codecs(codecs)
        .with_chunk_key(ChunkKeyEncoding::v2(separator))
        .with_attributes(attributes);
    meta.dimension_names = dimension_names;
    Ok(meta)
}

fn spec_from_v2(v: &Value) -> Result<CodecSpec> {
    let name = v
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidMetadata("codec object needs an 'id'".into()))?;
    Ok(CodecSpec {
        name: name.to_string(),
        config: v.clone(),
    })
}

/// Serialises array metadata to a `.zarray` document.
pub fn array_json(meta: &ArrayMetadata) -> Result<String> {
    if meta.format != ZarrFormat::V2 {
        return Err(Error::InvalidMetadata("array is not in v2 format".into()));
    }
    let (compressor, filters): (Value, Vec<Value>) = match meta.codecs.split_last() {
        Some((last, rest)) if COMPRESSOR_IDS.contains(&last.name.as_str()) => (
            last.config.clone(),
            rest.iter().map(|c| c.config.clone()).collect(),
        ),
        _ => (
            Value::Null,
            meta.codecs.iter().map(|c| c.config.clone()).collect(),
        ),
    };
    let filters = if filters.is_empty() {
        Value::Null
    } else {
        Value::Array(filters)
    };
    let separator = meta.chunk_key.separator().to_string();
    let doc = json!({
        "chunks": meta.chunk_shape,
        "compressor": compressor,
        "dtype": meta.dtype.to_v2(),
        "fill_value": meta.fill_value.to_json(),
        "filters": filters,
        "order": "C",
        "shape": meta.shape,
        "zarr_format": 2,
        "dimension_separator": separator,
    });
    to_pretty(&doc)
}

/// Serialises attributes to a `.zattrs` document, writing dimension names
/// under the `_ARRAY_DIMENSIONS` key when they are set.
pub fn attrs_json(
    meta_attrs: &Map<String, Value>,
    dimension_names: Option<&[Option<String>]>,
) -> Result<String> {
    let mut attrs = meta_attrs.clone();
    if let Some(names) = dimension_names {
        attrs.insert(
            ARRAY_DIMENSIONS_ATTR.to_string(),
            dimension_names_to_json(names),
        );
    }
    to_pretty(&Value::Object(attrs))
}

/// Parses a `.zgroup` document and optional `.zattrs` document.
pub fn parse_group(zgroup: &[u8], zattrs: Option<&[u8]>) -> Result<GroupMetadata> {
    let obj = parse_object(zgroup, ZGROUP)?;
    if field(&obj, "zarr_format")?.as_u64() != Some(2) {
        return Err(Error::InvalidMetadata(
            "'.zgroup' zarr_format must be 2".into(),
        ));
    }
    let attributes = match zattrs {
        Some(bytes) => parse_object(bytes, ZATTRS)?,
        None => Map::new(),
    };
    Ok(GroupMetadata {
        format: Some(ZarrFormat::V2),
        attributes,
    })
}

/// The `.zgroup` document.
pub fn group_json() -> String {
    to_pretty(&json!({ "zarr_format": 2 })).unwrap_or_default()
}

/// Builds the consolidated `.zmetadata` document from `(key, document)`
/// pairs, where each key is a path such as `group/.zarray`.
pub fn consolidated_json(entries: Vec<(String, Value)>) -> Result<String> {
    let mut metadata = Map::new();
    for (k, v) in entries {
        metadata.insert(k, v);
    }
    to_pretty(&json!({ "metadata": metadata, "zarr_consolidated_format": 1 }))
}

/// Parses a `.zmetadata` document into `(key, document)` pairs.
pub fn parse_consolidated(bytes: &[u8]) -> Result<Vec<(String, Value)>> {
    let obj = parse_object(bytes, ZMETADATA)?;
    let meta = field(&obj, "metadata")?
        .as_object()
        .ok_or_else(|| Error::InvalidMetadata("'.zmetadata' metadata must be an object".into()))?;
    Ok(meta.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
}

fn to_pretty(v: &Value) -> Result<String> {
    serde_json::to_string_pretty(v).map_err(|e| Error::InvalidMetadata(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.zarray` document written by hand from the Zarr v2 specification
    /// example layout: little-endian float64, zlib compressor, C order.
    const ZARRAY_FIXTURE: &str = r#"{
        "chunks": [2, 3],
        "compressor": {"id": "zlib", "level": 1},
        "dtype": "<f8",
        "fill_value": "NaN",
        "filters": null,
        "order": "C",
        "shape": [4, 6],
        "zarr_format": 2
    }"#;

    #[test]
    fn parses_zarray_fixture() {
        let m = parse_array(ZARRAY_FIXTURE.as_bytes(), None).unwrap();
        assert_eq!(m.shape, vec![4, 6]);
        assert_eq!(m.chunk_shape, vec![2, 3]);
        assert_eq!(m.dtype, DType::FLOAT64);
        assert!(matches!(m.fill_value, FillValue::Float(f) if f.is_nan()));
        assert_eq!(m.codecs.len(), 1);
        assert_eq!(m.codecs[0].name, "zlib");
        assert_eq!(m.chunk_key, ChunkKeyEncoding::v2('.'));
    }

    #[test]
    fn zarray_round_trip_is_stable() {
        let m = parse_array(ZARRAY_FIXTURE.as_bytes(), None).unwrap();
        let text = array_json(&m).unwrap();
        let again = parse_array(text.as_bytes(), None).unwrap();
        assert_eq!(again, m);
    }

    #[test]
    fn filters_precede_compressor() {
        let doc = r#"{"chunks":[4],"compressor":{"id":"lz4"},"dtype":"<i4",
            "fill_value":0,"filters":[{"id":"delta","dtype":"<i4"}],
            "order":"C","shape":[8],"zarr_format":2,"dimension_separator":"/"}"#;
        let m = parse_array(doc.as_bytes(), None).unwrap();
        let names: Vec<&str> = m.codecs.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["delta", "lz4"]);
        assert_eq!(m.chunk_key, ChunkKeyEncoding::v2('/'));
    }

    #[test]
    fn rejects_fortran_order_and_wrong_format() {
        let f = ZARRAY_FIXTURE.replace("\"C\"", "\"F\"");
        assert!(parse_array(f.as_bytes(), None).is_err());
        let w = ZARRAY_FIXTURE.replace("\"zarr_format\": 2", "\"zarr_format\": 3");
        assert!(parse_array(w.as_bytes(), None).is_err());
    }

    #[test]
    fn dimension_names_come_from_attrs() {
        let attrs = br#"{"units": "K", "_ARRAY_DIMENSIONS": ["time", "lat"]}"#;
        let m = parse_array(ZARRAY_FIXTURE.as_bytes(), Some(attrs)).unwrap();
        assert_eq!(
            m.dimension_names,
            Some(vec![Some("time".to_string()), Some("lat".to_string())])
        );
        let text = attrs_json(&m.attributes, m.dimension_names.as_deref()).unwrap();
        assert!(text.contains("_ARRAY_DIMENSIONS"));
    }

    #[test]
    fn group_and_consolidated_documents() {
        let g = parse_group(br#"{"zarr_format": 2}"#, Some(br#"{"title": "x"}"#)).unwrap();
        assert_eq!(g.attributes.get("title"), Some(&Value::String("x".into())));
        let c = consolidated_json(vec![(".zgroup".into(), json!({"zarr_format": 2}))]).unwrap();
        let entries = parse_consolidated(c.as_bytes()).unwrap();
        assert_eq!(entries[0].0, ".zgroup");
    }
}
