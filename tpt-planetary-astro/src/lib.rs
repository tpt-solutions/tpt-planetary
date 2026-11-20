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
//! - [`precession`]: IAU 2006 precession angles and matrix (no frame bias).
//!   The angles need no `std`; the matrix needs `std`.
//! - [`nutation`]: IAU 2000A_R06 nutation (full series), the nutation matrix,
//!   and the classical equation of the equinoxes. The fundamental arguments
//!   and table data need no `std`; the series and matrix need `std`.
//! - [`cio`]: CIP coordinates X and Y, the CIO and TIO locators, frame bias,
//!   and the CIO-based ICRS to ITRS matrix (IAU 2006/2000A). Needs `std`.
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
pub mod fits;
#[cfg(all(test, not(feature = "std")))]
extern crate alloc;

pub mod ephemeris;
pub mod nutation;
pub mod precession;

#[cfg(feature = "std")]
mod nutation_data;

#[cfg(feature = "std")]
pub mod cio;
#[cfg(feature = "std")]
mod cio_data;

#[cfg(feature = "std")]
pub mod frames;
#[cfg(feature = "std")]
pub mod orbit;
#[cfg(feature = "std")]
pub mod time;
#[cfg(feature = "std")]
pub mod wcs;

/// Version of this crate, taken from the package manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// Checked at compile time: the package version must never be empty.
const _: () = assert!(!VERSION.is_empty());
