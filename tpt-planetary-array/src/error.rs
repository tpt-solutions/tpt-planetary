//! Error type shared by every module of the array crate.

use alloc::string::String;
use core::fmt;

/// Errors produced while parsing metadata, planning selections, encoding
/// or decoding chunks, and accessing a store.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Metadata document is malformed or internally inconsistent.
    InvalidMetadata(String),
    /// Metadata or data uses a feature this crate does not implement.
    Unsupported(String),
    /// A selection is out of bounds or otherwise invalid.
    InvalidSelection(String),
    /// A buffer has the wrong length for the requested shape or dtype.
    SizeMismatch {
        /// Number of bytes that were required.
        expected: usize,
        /// Number of bytes that were supplied.
        actual: usize,
    },
    /// A codec failed to encode or decode its input.
    Codec(String),
    /// A requested node or key does not exist in the store.
    NotFound(String),
    /// An I/O error from a store backend (only produced with the `std` feature).
    Io(String),
}

/// Convenience alias used throughout the crate.
pub type Result<T> = core::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidMetadata(m) => write!(f, "invalid metadata: {m}"),
            Error::Unsupported(m) => write!(f, "unsupported: {m}"),
            Error::InvalidSelection(m) => write!(f, "invalid selection: {m}"),
            Error::SizeMismatch { expected, actual } => {
                write!(
                    f,
                    "buffer size mismatch: expected {expected} bytes, got {actual}"
                )
            }
            Error::Codec(m) => write!(f, "codec error: {m}"),
            Error::NotFound(m) => write!(f, "not found: {m}"),
            Error::Io(m) => write!(f, "I/O error: {m}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}
