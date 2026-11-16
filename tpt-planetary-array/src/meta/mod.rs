//! Zarr metadata: the format-independent model ([`ArrayMetadata`],
//! [`GroupMetadata`]) plus the Zarr v2 ([`v2`]) and v3 ([`v3`]) readers and
//! writers.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::{Map, Value};

use alloc::boxed::Box;

use crate::codec::sharding::ShardingCodec;
use crate::codec::transpose::TransposeCodec;
use crate::codec::{Codec, Pipeline, Registry};
use crate::dtype::DType;
use crate::error::{Error, Result};
use crate::fill::FillValue;
use crate::grid::{ChunkGrid, ChunkKeyEncoding};

pub mod v2;
pub mod v3;

/// Attribute key used by xarray and NetCDF-style Zarr stores to record
/// dimension names in Zarr v2 `.zattrs`.
pub const ARRAY_DIMENSIONS_ATTR: &str = "_ARRAY_DIMENSIONS";

/// Zarr specification version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZarrFormat {
    /// Zarr v2 (`.zarray`, `.zgroup`, `.zattrs`).
    V2,
    /// Zarr v3 (`zarr.json`).
    V3,
}

/// A codec as named in metadata, with its JSON configuration.
///
/// For v2 the configuration is the full compressor or filter object (which
/// includes its `id`). For v3 it is the `configuration` member.
#[derive(Debug, Clone, PartialEq)]
pub struct CodecSpec {
    /// Codec name (`id` in v2, `name` in v3).
    pub name: String,
    /// Codec configuration.
    pub config: Value,
}

/// Format-independent description of an array.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrayMetadata {
    /// Zarr format this metadata is written in.
    pub format: ZarrFormat,
    /// Array shape.
    pub shape: Vec<u64>,
    /// Chunk shape.
    pub chunk_shape: Vec<u64>,
    /// Element type, including byte order.
    pub dtype: DType,
    /// Fill value for unwritten chunks.
    pub fill_value: FillValue,
    /// Chunk key encoding.
    pub chunk_key: ChunkKeyEncoding,
    /// Byte-to-byte codecs in encoding order (v2 filters followed by the
    /// compressor; v3 `bytes->bytes` codecs). The v3 `bytes` codec is folded
    /// into [`DType::endian`] and does not appear here.
    pub codecs: Vec<CodecSpec>,
    /// Optional dimension names (`None` entries are unnamed dimensions).
    pub dimension_names: Option<Vec<Option<String>>>,
    /// User attributes.
    pub attributes: Map<String, Value>,
}

impl ArrayMetadata {
    /// Creates metadata with a `null` fill value, the default chunk key
    /// encoding for `format`, no codecs and no attributes.
    pub fn new(
        format: ZarrFormat,
        shape: Vec<u64>,
        chunk_shape: Vec<u64>,
        dtype: DType,
    ) -> Result<Self> {
        ChunkGrid::new(shape.clone(), chunk_shape.clone())?;
        let chunk_key = match format {
            ZarrFormat::V2 => ChunkKeyEncoding::v2('.'),
            ZarrFormat::V3 => ChunkKeyEncoding::v3_default('/'),
        };
        Ok(Self {
            format,
            shape,
            chunk_shape,
            dtype,
            fill_value: FillValue::Null,
            chunk_key,
            codecs: Vec::new(),
            dimension_names: None,
            attributes: Map::new(),
        })
    }

    /// Sets the fill value.
    pub fn with_fill_value(mut self, fill_value: FillValue) -> Self {
        self.fill_value = fill_value;
        self
    }

    /// Sets the byte-to-byte codec list (encoding order).
    pub fn with_codecs(mut self, codecs: Vec<CodecSpec>) -> Self {
        self.codecs = codecs;
        self
    }

    /// Sets the chunk key encoding.
    pub fn with_chunk_key(mut self, chunk_key: ChunkKeyEncoding) -> Self {
        self.chunk_key = chunk_key;
        self
    }

    /// Sets dimension names.
    pub fn with_dimension_names(mut self, names: Vec<Option<String>>) -> Self {
        self.dimension_names = Some(names);
        self
    }

    /// Sets user attributes.
    pub fn with_attributes(mut self, attributes: Map<String, Value>) -> Self {
        self.attributes = attributes;
        self
    }

    /// The chunk grid implied by shape and chunk shape.
    pub fn grid(&self) -> Result<ChunkGrid> {
        ChunkGrid::new(self.shape.clone(), self.chunk_shape.clone())
    }

    /// Builds the codec pipeline using `registry`.
    ///
    /// Codecs are checked against the Zarr v3 stage order: array-to-array
    /// (`transpose`) first, then at most one array-to-bytes codec
    /// (`sharding_indexed`), then bytes-to-bytes codecs. `transpose` and
    /// `sharding_indexed` are built here, not by the registry, because they
    /// need the chunk shape and fill value. Each returned codec maps the stored
    /// chunk back to its element bytes in C order.
    pub fn pipeline(&self, registry: &Registry) -> Result<Pipeline> {
        let mut stage = Stage::ArrayToArray;
        let mut shape = self.chunk_shape.clone();
        let item = self.dtype.item_size();
        let mut codecs: Vec<Box<dyn Codec>> = Vec::with_capacity(self.codecs.len());
        for spec in &self.codecs {
            match spec.name.as_str() {
                "transpose" => {
                    if stage != Stage::ArrayToArray {
                        return Err(Error::InvalidMetadata(
                            "transpose must come before sharding and bytes-to-bytes codecs".into(),
                        ));
                    }
                    let t = TransposeCodec::new(&spec.config, &shape, item)?;
                    shape = t.output_shape();
                    codecs.push(Box::new(t));
                }
                "sharding_indexed" => {
                    if stage != Stage::ArrayToArray {
                        return Err(Error::InvalidMetadata(
                            "sharding_indexed must follow only array-to-array codecs".into(),
                        ));
                    }
                    let fill = self.fill_value.to_element_bytes(&self.dtype)?;
                    let s = ShardingCodec::new(&spec.config, &shape, &self.dtype, &fill, registry)?;
                    stage = Stage::ArrayToBytes;
                    codecs.push(Box::new(s));
                }
                "bitshuffle" => {
                    if stage == Stage::ArrayToBytes {
                        return Err(Error::InvalidMetadata(
                            "bitshuffle after sharding_indexed is not supported".into(),
                        ));
                    }
                    let elements = shape
                        .iter()
                        .try_fold(1usize, |acc, &d| acc.checked_mul(usize::try_from(d).ok()?))
                        .ok_or_else(|| Error::InvalidMetadata("chunk shape is too large".into()))?;
                    let nbytes = elements
                        .checked_mul(item)
                        .ok_or_else(|| Error::InvalidMetadata("chunk size is too large".into()))?;
                    let c = crate::codec::bitshuffle::bitshuffle_for_size(
                        &spec.config,
                        &self.dtype,
                        Some(nbytes),
                    )?;
                    stage = Stage::BytesToBytes;
                    codecs.push(Box::new(c));
                }
                name => {
                    if stage == Stage::ArrayToArray {
                        stage = Stage::BytesToBytes;
                    }
                    codecs.push(registry.build(name, &spec.config, &self.dtype)?);
                }
            }
        }
        Ok(Pipeline::new(codecs))
    }
}

/// Position of a codec in the Zarr v3 chain, used to check stage order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Only array-to-array codecs so far.
    ArrayToArray,
    /// An array-to-bytes codec has been placed.
    ArrayToBytes,
    /// A bytes-to-bytes codec has been placed; no array codecs may follow.
    BytesToBytes,
}

/// Metadata of a Zarr group.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GroupMetadata {
    /// Zarr format this metadata is written in.
    pub format: Option<ZarrFormat>,
    /// User attributes.
    pub attributes: Map<String, Value>,
}

/// A node read from a store: either an array or a group.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeMetadata {
    /// An array node.
    Array(ArrayMetadata),
    /// A group node.
    Group(GroupMetadata),
}

/// Reads a required member of a JSON object.
pub(crate) fn field<'a>(obj: &'a Map<String, Value>, key: &str) -> Result<&'a Value> {
    obj.get(key)
        .ok_or_else(|| Error::InvalidMetadata(format!("missing field '{key}'")))
}

/// Reads an array of non-negative integers.
pub(crate) fn u64_vec(v: &Value, key: &str) -> Result<Vec<u64>> {
    let arr = v
        .as_array()
        .ok_or_else(|| Error::InvalidMetadata(format!("'{key}' must be an array")))?;
    arr.iter()
        .map(|x| {
            x.as_u64().ok_or_else(|| {
                Error::InvalidMetadata(format!("'{key}' must contain non-negative integers"))
            })
        })
        .collect()
}

/// Parses a JSON document that must be an object.
pub(crate) fn parse_object(bytes: &[u8], what: &str) -> Result<Map<String, Value>> {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(Error::InvalidMetadata(format!(
            "{what} must be a JSON object"
        ))),
        Err(e) => Err(Error::InvalidMetadata(format!("{what}: {e}"))),
    }
}

/// Parses dimension names from a JSON array of strings or nulls.
pub(crate) fn dimension_names_from(v: &Value) -> Result<Vec<Option<String>>> {
    let arr = v
        .as_array()
        .ok_or_else(|| Error::InvalidMetadata("dimension names must be an array".to_string()))?;
    arr.iter()
        .map(|x| match x {
            Value::Null => Ok(None),
            Value::String(s) => Ok(Some(s.clone())),
            _ => Err(Error::InvalidMetadata(
                "dimension names must be strings or null".to_string(),
            )),
        })
        .collect()
}

pub(crate) fn dimension_names_to_json(names: &[Option<String>]) -> Value {
    Value::Array(
        names
            .iter()
            .map(|n| n.as_ref().map_or(Value::Null, |s| Value::String(s.clone())))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_format() {
        let v2 = ArrayMetadata::new(ZarrFormat::V2, vec![2], vec![1], DType::FLOAT64).unwrap();
        assert_eq!(v2.chunk_key, ChunkKeyEncoding::v2('.'));
        let v3 = ArrayMetadata::new(ZarrFormat::V3, vec![2], vec![1], DType::FLOAT64).unwrap();
        assert_eq!(v3.chunk_key, ChunkKeyEncoding::v3_default('/'));
        assert!(ArrayMetadata::new(ZarrFormat::V3, vec![2], vec![0], DType::FLOAT64).is_err());
    }
}
