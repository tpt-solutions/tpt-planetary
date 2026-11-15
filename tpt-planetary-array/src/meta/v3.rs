//! Zarr v3 metadata: `zarr.json` documents for arrays and groups.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::{Map, Value, json};

use super::{
    ArrayMetadata, CodecSpec, GroupMetadata, NodeMetadata, ZarrFormat, dimension_names_from,
    dimension_names_to_json, field, parse_object, u64_vec,
};
use crate::dtype::{DType, Endian, Kind};
use crate::error::{Error, Result};
use crate::fill::FillValue;
use crate::grid::ChunkKeyEncoding;

/// File name of v3 node metadata.
pub const ZARR_JSON: &str = "zarr.json";

/// Parses a `zarr.json` document into an array or group.
pub fn parse_node(bytes: &[u8]) -> Result<NodeMetadata> {
    let obj = parse_object(bytes, ZARR_JSON)?;
    if field(&obj, "zarr_format")?.as_u64() != Some(3) {
        return Err(Error::InvalidMetadata(
            "zarr.json zarr_format must be 3".into(),
        ));
    }
    let attributes = match obj.get("attributes") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => {
            return Err(Error::InvalidMetadata(
                "attributes must be an object".into(),
            ));
        }
    };
    match field(&obj, "node_type")?.as_str() {
        Some("group") => Ok(NodeMetadata::Group(GroupMetadata {
            format: Some(ZarrFormat::V3),
            attributes,
        })),
        Some("array") => parse_array_obj(&obj, attributes).map(NodeMetadata::Array),
        _ => Err(Error::InvalidMetadata(
            "node_type must be 'array' or 'group'".into(),
        )),
    }
}

fn parse_array_obj(
    obj: &Map<String, Value>,
    attributes: Map<String, Value>,
) -> Result<ArrayMetadata> {
    let shape = u64_vec(field(obj, "shape")?, "shape")?;

    let grid = field(obj, "chunk_grid")?;
    if grid.get("name").and_then(Value::as_str) != Some("regular") {
        return Err(Error::Unsupported(
            "only the regular chunk grid is supported".into(),
        ));
    }
    let chunk_shape = u64_vec(
        grid.get("configuration")
            .and_then(|c| c.get("chunk_shape"))
            .ok_or_else(|| Error::InvalidMetadata("regular grid needs chunk_shape".into()))?,
        "chunk_shape",
    )?;

    let (codecs, endian) = parse_codecs(field(obj, "codecs")?)?;
    let data_type = field(obj, "data_type")?
        .as_str()
        .ok_or_else(|| Error::InvalidMetadata("data_type must be a string".into()))?;
    let dtype = DType::from_v3(data_type, endian)?;

    let fill_value = parse_fill(field(obj, "fill_value")?, &dtype)?;
    let chunk_key = parse_chunk_key(obj.get("chunk_key_encoding"))?;

    let dimension_names = match obj.get("dimension_names") {
        None | Some(Value::Null) => None,
        Some(v) => Some(dimension_names_from(v)?),
    };

    let mut meta = ArrayMetadata::new(ZarrFormat::V3, shape, chunk_shape, dtype)?
        .with_fill_value(fill_value)
        .with_codecs(codecs)
        .with_chunk_key(chunk_key)
        .with_attributes(attributes);
    meta.dimension_names = dimension_names;
    Ok(meta)
}

/// Splits the v3 codec list into byte-to-byte codecs and the byte order given
/// by the `bytes` codec (default little-endian).
fn parse_codecs(v: &Value) -> Result<(Vec<CodecSpec>, Endian)> {
    let arr = v
        .as_array()
        .ok_or_else(|| Error::InvalidMetadata("codecs must be an array".into()))?;
    let mut endian = Endian::Little;
    let mut specs = Vec::new();
    for c in arr {
        let name = c
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidMetadata("codec needs a name".into()))?;
        let config = c.get("configuration").cloned().unwrap_or_else(|| json!({}));
        if name == "bytes" {
            endian = match config.get("endian").and_then(Value::as_str) {
                None | Some("little") => Endian::Little,
                Some("big") => Endian::Big,
                Some(other) => return Err(Error::InvalidMetadata(format!("endian '{other}'"))),
            };
        } else {
            specs.push(CodecSpec {
                name: name.to_string(),
                config,
            });
        }
    }
    Ok((specs, endian))
}

fn parse_chunk_key(v: Option<&Value>) -> Result<ChunkKeyEncoding> {
    let Some(v) = v else {
        return Ok(ChunkKeyEncoding::v3_default('/'));
    };
    let name = v.get("name").and_then(Value::as_str).unwrap_or("default");
    let sep = v
        .get("configuration")
        .and_then(|c| c.get("separator"))
        .and_then(Value::as_str);
    match name {
        "default" => Ok(ChunkKeyEncoding::v3_default(single_char(
            sep.unwrap_or("/"),
        )?)),
        "v2" => Ok(ChunkKeyEncoding::v2(single_char(sep.unwrap_or("."))?)),
        other => Err(Error::Unsupported(format!("chunk key encoding '{other}'"))),
    }
}

fn single_char(s: &str) -> Result<char> {
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => Ok(c),
        _ => Err(Error::InvalidMetadata(
            "chunk key separator must be one character".into(),
        )),
    }
}

/// Parses a v3 fill value. Floats may also be given as hexadecimal bit
/// patterns (`"0x7fc00000"`).
fn parse_fill(v: &Value, dtype: &DType) -> Result<FillValue> {
    if let (Value::String(s), true) = (v, dtype.is_float()) {
        if let Some(hex) = s.strip_prefix("0x") {
            let bits = u64::from_str_radix(hex, 16)
                .map_err(|_| Error::InvalidMetadata(format!("bad hex fill '{s}'")))?;
            return match dtype.kind() {
                Kind::Float(32) => Ok(FillValue::Float(f64::from(f32::from_bits(bits as u32)))),
                Kind::Float(_) => Ok(FillValue::Float(f64::from_bits(bits))),
                _ => Err(Error::Unsupported("hex fill for complex dtype".into())),
            };
        }
    }
    FillValue::from_json(v)
}

/// Serialises array metadata to a `zarr.json` document.
pub fn array_json(meta: &ArrayMetadata) -> Result<String> {
    if meta.format != ZarrFormat::V3 {
        return Err(Error::InvalidMetadata("array is not in v3 format".into()));
    }
    let endian = match meta.dtype.endian() {
        Endian::Little => "little",
        Endian::Big => "big",
    };
    let mut codecs = alloc::vec![json!({"name": "bytes", "configuration": {"endian": endian}})];
    codecs.extend(
        meta.codecs
            .iter()
            .map(|c| json!({"name": c.name, "configuration": c.config})),
    );

    let (key_name, key_sep) = if meta.chunk_key.has_prefix() {
        ("default", meta.chunk_key.separator())
    } else {
        ("v2", meta.chunk_key.separator())
    };

    let mut doc = Map::new();
    doc.insert("zarr_format".into(), json!(3));
    doc.insert("node_type".into(), json!("array"));
    doc.insert("shape".into(), json!(meta.shape));
    doc.insert("data_type".into(), json!(meta.dtype.v3_name()?));
    doc.insert(
        "chunk_grid".into(),
        json!({"name": "regular", "configuration": {"chunk_shape": meta.chunk_shape}}),
    );
    doc.insert(
        "chunk_key_encoding".into(),
        json!({"name": key_name, "configuration": {"separator": key_sep.to_string()}}),
    );
    doc.insert("fill_value".into(), meta.fill_value.to_json());
    doc.insert("codecs".into(), Value::Array(codecs));
    if let Some(names) = &meta.dimension_names {
        doc.insert("dimension_names".into(), dimension_names_to_json(names));
    }
    if !meta.attributes.is_empty() {
        doc.insert("attributes".into(), Value::Object(meta.attributes.clone()));
    }
    serde_json::to_string_pretty(&Value::Object(doc))
        .map_err(|e| Error::InvalidMetadata(e.to_string()))
}

/// Serialises group metadata to a `zarr.json` document.
pub fn group_json(group: &GroupMetadata) -> Result<String> {
    let mut doc = Map::new();
    doc.insert("zarr_format".into(), json!(3));
    doc.insert("node_type".into(), json!("group"));
    if !group.attributes.is_empty() {
        doc.insert("attributes".into(), Value::Object(group.attributes.clone()));
    }
    serde_json::to_string_pretty(&Value::Object(doc))
        .map_err(|e| Error::InvalidMetadata(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-written `zarr.json` for a float32 array with a big-endian bytes
    /// codec, a gzip codec and the `/`-separated default chunk key encoding.
    const ARRAY_FIXTURE: &str = r#"{
        "zarr_format": 3,
        "node_type": "array",
        "shape": [10, 20],
        "data_type": "float32",
        "chunk_grid": {"name": "regular", "configuration": {"chunk_shape": [5, 10]}},
        "chunk_key_encoding": {"name": "default", "configuration": {"separator": "/"}},
        "fill_value": "NaN",
        "codecs": [
            {"name": "bytes", "configuration": {"endian": "big"}},
            {"name": "gzip", "configuration": {"level": 3}}
        ],
        "dimension_names": ["x", "y"],
        "attributes": {"units": "m"}
    }"#;

    #[test]
    fn parses_array_fixture() {
        let NodeMetadata::Array(m) = parse_node(ARRAY_FIXTURE.as_bytes()).unwrap() else {
            panic!("expected array");
        };
        assert_eq!(m.shape, vec![10, 20]);
        assert_eq!(m.chunk_shape, vec![5, 10]);
        assert_eq!(m.dtype, DType::from_v3("float32", Endian::Big).unwrap());
        assert_eq!(m.codecs.len(), 1);
        assert_eq!(m.codecs[0].name, "gzip");
        assert_eq!(m.chunk_key, ChunkKeyEncoding::v3_default('/'));
        assert_eq!(m.dimension_names.as_ref().unwrap()[1].as_deref(), Some("y"));
    }

    #[test]
    fn array_round_trip() {
        let NodeMetadata::Array(m) = parse_node(ARRAY_FIXTURE.as_bytes()).unwrap() else {
            panic!("expected array");
        };
        let text = array_json(&m).unwrap();
        let NodeMetadata::Array(again) = parse_node(text.as_bytes()).unwrap() else {
            panic!("expected array");
        };
        assert_eq!(again, m);
    }

    #[test]
    fn parses_group_and_rejects_bad_documents() {
        let g = parse_node(br#"{"zarr_format": 3, "node_type": "group", "attributes": {"a": 1}}"#)
            .unwrap();
        assert!(matches!(g, NodeMetadata::Group(ref gm) if gm.attributes.len() == 1));
        assert!(parse_node(br#"{"zarr_format": 2, "node_type": "group"}"#).is_err());
        assert!(parse_node(br#"{"zarr_format": 3, "node_type": "table"}"#).is_err());
    }

    #[test]
    fn hex_float_fill_is_decoded() {
        let doc = ARRAY_FIXTURE.replace("\"NaN\"", "\"0x3f800000\"");
        let NodeMetadata::Array(m) = parse_node(doc.as_bytes()).unwrap() else {
            panic!("expected array");
        };
        assert_eq!(m.fill_value, FillValue::Float(1.0));
    }

    #[test]
    fn rejects_unsupported_grid() {
        let doc = ARRAY_FIXTURE.replace("\"regular\"", "\"rectilinear\"");
        assert!(parse_node(doc.as_bytes()).is_err());
    }
}
