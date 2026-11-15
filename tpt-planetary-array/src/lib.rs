//! Chunked N-dimensional arrays for tpt-planetary.
//!
//! This crate provides:
//!
//! - a data model for N-dimensional shapes, C-order strides, element types
//!   ([`DType`]), fill values ([`FillValue`]), regular chunk grids and chunk
//!   key encodings;
//! - slicing ([`Selection`], [`Slice`]) and chunk-aligned region planning
//!   ([`plan::RegionPlan`]);
//! - Zarr v2 and v3 metadata (see [`meta`]);
//! - pure-Rust codecs (gzip, zlib, LZ4, shuffle, delta) and a registry
//!   ([`codec`]);
//! - a synchronous [`Store`] trait with in-memory and local-filesystem
//!   implementations (`std` feature);
//! - an [`Array`] handle with chunked read and write paths; and
//! - a lazy expression graph with bounded-memory reductions ([`lazy`]).
//!
//! The crate is `no_std` with `alloc` when the default `std` feature is
//! disabled. Asynchronous and network-backed stores are not implemented.
//!
//! # Example
//!
//! ```
//! use tpt_planetary_array::{Array, MemoryStore, Selection, Slice};
//! use tpt_planetary_array::meta::{ArrayMetadata, ZarrFormat};
//! use tpt_planetary_array::{DType, FillValue};
//! use std::sync::Arc;
//!
//! let store = Arc::new(MemoryStore::new());
//! let meta = ArrayMetadata::new(ZarrFormat::V2, vec![4, 6], vec![2, 3], DType::FLOAT64)
//!     .unwrap()
//!     .with_fill_value(FillValue::Float(f64::NAN));
//! let array = Array::create(store.clone(), "temperature", meta).unwrap();
//!
//! let values: Vec<f64> = (0..24).map(f64::from).collect();
//! array.write_f64(&[Selection::ALL, Selection::ALL], &values).unwrap();
//!
//! let row = array
//!     .read_f64(&[Selection::Index(1), Selection::Range(Slice::range(0, 3))])
//!     .unwrap();
//! assert_eq!(row, vec![6.0, 7.0, 8.0]);
//! ```

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod array;
pub mod codec;
pub mod dtype;
pub mod error;
pub mod fill;
pub mod grid;
pub mod hierarchy;
pub mod lazy;
pub mod meta;
pub mod netcdf3;
pub mod ops;
pub mod plan;
pub mod selection;
pub mod store;

pub use array::Array;
pub use dtype::{DType, Endian, Kind};
pub use error::{Error, Result};
pub use fill::FillValue;
pub use lazy::Expr;
pub use meta::{ArrayMetadata, ZarrFormat};
pub use selection::{Selection, Slice};
pub use store::Store;
#[cfg(feature = "std")]
pub use store::{FsStore, MemoryStore};

/// Version of this crate, taken from the package manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
