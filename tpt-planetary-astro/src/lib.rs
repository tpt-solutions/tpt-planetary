//! Astronomy layer for tpt-planetary: time scales, coordinate frames,
//! JPL-style ephemerides and WCS.
//!
//! Modules and their feature requirements:
//! - [`ephemeris`]: DAF/SPK parser and Chebyshev evaluator. Works without
//!   `std` and without allocation when given a byte slice. The light-time
//!   helper and file reader need `std`.
//! - [`time`]: TDB <-> TT (leading series terms only), ERA, GMST (IAU 2006),
//!   GAST. Needs `std` for the trigonometric functions.
//! - [`frames`]: typed sky coordinates, ICRS/Galactic/ecliptic transforms, and
//!   Alt/Az. Needs `std`.
//! - [`wcs`]: FITS WCS header parsing and TAN/SIN/AIT projections. Needs `std`.
//!
//! Feature flags:
//! - `std` (default): enables the trigonometric modules and file I/O.
//!
//! Status: see the crate README and the Phase 2 and 3 checklist in the
//! workspace `todo.md`. Numerical claims are limited to what the unit tests
//! check. Comparisons against ERFA, Astropy and wcslib are in the Python
//! validation suite; see `docs/validation.md` for results and open gaps.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

pub mod eop;
pub mod ephemeris;

#[cfg(feature = "std")]
pub mod frames;
#[cfg(feature = "std")]
pub mod time;
#[cfg(feature = "std")]
pub mod wcs;

/// Version of this crate, taken from the package manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// Checked at compile time: the package version must never be empty.
const _: () = assert!(!VERSION.is_empty());
