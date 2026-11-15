//! Foundation crate for tpt-planetary.
//!
//! The crate is `no_std` unless the `std` feature is enabled. It provides:
//!
//! - [`constants`]: physical and astronomical constants, each with its source.
//! - [`jd`] and [`calendar`]: two-part Julian Dates and Gregorian/Julian
//!   calendar conversion (Meeus, chapter 7).
//! - [`leap`] and [`scale`]: the embedded leap-second table with expiry, and
//!   typed UTC, TAI and TT epochs with conversions between them.
//! - [`angle`] and [`sphere`]: angle newtypes, angular separation, position
//!   angle and great-circle operations.
//! - [`ellipsoid`] and [`rotation`]: WGS84 geodetic/ECEF conversion and
//!   quaternion rotations (via `glam`).
//!
//! TDB is not implemented yet.
//!
//! # Feature flags
//!
//! - `std` (default): enables the standard library. Implies `alloc`.
//! - `alloc`: enables items that require heap allocation. None are currently
//!   provided, so the feature has no effect on the public API.
//!
//! # Floating-point functions
//!
//! `core` does not provide `sin`, `sqrt` and `floor` for `f64` without `std`.
//! This crate routes trigonometry through `glam` (built with its `libm`
//! feature), so it builds for embedded targets without another dependency.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod angle;
pub mod calendar;
pub mod constants;
pub mod ellipsoid;
pub mod error;
pub mod jd;
pub mod leap;
mod math;
pub mod rotation;
pub mod scale;
pub mod sphere;

pub use error::Error;

/// Version of this crate, taken from the package manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// Checked at compile time: the package version must never be empty.
const _: () = assert!(!VERSION.is_empty());
