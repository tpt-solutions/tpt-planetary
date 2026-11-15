//! Codec trait, codec pipelines and the codec registry.
//!
//! A [`Pipeline`] is an ordered list of byte-to-byte codecs. Encoding applies
//! them in order; decoding applies their inverses in reverse. Zarr v2 filters
//! and compressors, and Zarr v3 `bytes->bytes` codecs, are both expressed this
//! way. The registry turns codec names and JSON configuration into codecs.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use serde_json::Value;

use crate::dtype::DType;
use crate::error::{Error, Result};

pub mod compress;
pub mod filters;

/// A reversible transformation of a chunk's bytes.
pub trait Codec: Send + Sync + fmt::Debug {
    /// Codec name as it appears in metadata (for example `gzip`).
    fn name(&self) -> &str;

    /// Encodes `input` into a new buffer.
    fn encode(&self, input: &[u8]) -> Result<Vec<u8>>;

    /// Decodes `input` into a new buffer.
    fn decode(&self, input: &[u8]) -> Result<Vec<u8>>;
}

/// An ordered list of codecs, stored in encoding order.
#[derive(Debug, Default)]
pub struct Pipeline {
    codecs: Vec<Box<dyn Codec>>,
}

impl Pipeline {
    /// Creates a pipeline from codecs listed in encoding order.
    pub fn new(codecs: Vec<Box<dyn Codec>>) -> Self {
        Self { codecs }
    }

    /// An empty pipeline (identity transform).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Number of codecs in the pipeline.
    pub fn len(&self) -> usize {
        self.codecs.len()
    }

    /// True if the pipeline has no codecs.
    pub fn is_empty(&self) -> bool {
        self.codecs.is_empty()
    }

    /// Names of the codecs in encoding order.
    pub fn names(&self) -> Vec<String> {
        self.codecs.iter().map(|c| c.name().to_string()).collect()
    }

    /// Encodes `input` through every codec in order.
    pub fn encode(&self, input: &[u8]) -> Result<Vec<u8>> {
        let mut buf: Option<Vec<u8>> = None;
        for codec in &self.codecs {
            let next = codec.encode(buf.as_deref().unwrap_or(input))?;
            buf = Some(next);
        }
        Ok(buf.unwrap_or_else(|| input.to_vec()))
    }

    /// Decodes `input` through every codec in reverse order.
    pub fn decode(&self, input: &[u8]) -> Result<Vec<u8>> {
        let mut buf: Option<Vec<u8>> = None;
        for codec in self.codecs.iter().rev() {
            let next = codec.decode(buf.as_deref().unwrap_or(input))?;
            buf = Some(next);
        }
        Ok(buf.unwrap_or_else(|| input.to_vec()))
    }
}

/// Builds a codec from its JSON configuration object and the array dtype.
pub type Factory = fn(config: &Value, dtype: &DType) -> Result<Box<dyn Codec>>;

/// Maps codec names to factories.
#[derive(Debug, Clone)]
pub struct Registry {
    entries: Vec<(&'static str, Factory)>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl Registry {
    /// A registry with no codecs.
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// A registry with every codec implemented by this crate.
    pub fn builtin() -> Self {
        let mut r = Self::empty();
        r.register("gzip", compress::gzip_factory);
        r.register("zlib", compress::zlib_factory);
        r.register("lz4", compress::lz4_factory);
        r.register("shuffle", filters::shuffle_factory);
        r.register("delta", filters::delta_factory);
        r
    }

    /// Adds or replaces a factory for `name`.
    pub fn register(&mut self, name: &'static str, factory: Factory) {
        self.entries.retain(|(n, _)| *n != name);
        self.entries.push((name, factory));
    }

    /// Returns true if a codec named `name` is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.entries.iter().any(|(n, _)| *n == name)
    }

    /// Builds the codec `name` with configuration `config` for an array of `dtype`.
    pub fn build(&self, name: &str, config: &Value, dtype: &DType) -> Result<Box<dyn Codec>> {
        match self.entries.iter().find(|(n, _)| *n == name) {
            Some((_, f)) => f(config, dtype),
            None => Err(Error::Unsupported(alloc::format!("codec '{name}'"))),
        }
    }
}

/// Reads an optional integer from a codec configuration object.
pub(crate) fn config_i64(config: &Value, key: &str) -> Result<Option<i64>> {
    match config.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_i64().map(Some).ok_or_else(|| {
            Error::InvalidMetadata(alloc::format!("codec option '{key}' must be an integer"))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug)]
    struct AddOne;
    impl Codec for AddOne {
        fn name(&self) -> &str {
            "add1"
        }
        fn encode(&self, input: &[u8]) -> Result<Vec<u8>> {
            Ok(input.iter().map(|b| b.wrapping_add(1)).collect())
        }
        fn decode(&self, input: &[u8]) -> Result<Vec<u8>> {
            Ok(input.iter().map(|b| b.wrapping_sub(1)).collect())
        }
    }

    #[test]
    fn pipeline_order_is_reversed_on_decode() {
        let p = Pipeline::new(vec![
            Box::new(AddOne),
            Box::new(compress::ZlibCodec::new(1)),
        ]);
        let data = b"hello hello hello".to_vec();
        let enc = p.encode(&data).unwrap();
        assert_eq!(p.decode(&enc).unwrap(), data);
    }

    #[test]
    fn registry_builds_known_and_rejects_unknown() {
        let r = Registry::builtin();
        assert!(
            r.build("gzip", &json!({"level": 1}), &DType::FLOAT64)
                .is_ok()
        );
        assert!(r.build("zstd", &json!({}), &DType::FLOAT64).is_err());
        assert!(r.contains("shuffle"));
    }
}
