//! Python bindings for tpt-planetary.
//!
//! The extension module is `tpt_planetary._core`; the public package
//! `tpt_planetary` re-exports its names from `python/tpt_planetary/__init__.py`.
//! `publish = false` keeps this crate off crates.io; maturin builds it for PyPI.
//!
//! Safety: this crate is `#![forbid(unsafe_code)]`. The PyO3 and rust-numpy
//! macros and APIs contain their own `unsafe` internally; no `unsafe` is written
//! here.

#![forbid(unsafe_code)]

mod array;
mod astro;
mod core_api;
mod errors;

use pyo3::prelude::*;

/// Version of the core crate the bindings are built against.
pub const CORE_VERSION: &str = tpt_planetary_core::VERSION;

/// Native extension module `tpt_planetary._core`.
#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("CORE_VERSION", CORE_VERSION)?;
    m.add(
        "TptPlanetaryError",
        m.py().get_type::<errors::TptPlanetaryError>(),
    )?;

    // Core: time, leap seconds, angular separation.
    m.add_class::<core_api::JulianDate>()?;
    m.add_function(wrap_pyfunction!(core_api::utc_to_tai, m)?)?;
    m.add_function(wrap_pyfunction!(core_api::tai_to_utc, m)?)?;
    m.add_function(wrap_pyfunction!(core_api::tai_to_tt, m)?)?;
    m.add_function(wrap_pyfunction!(core_api::tt_to_tai, m)?)?;
    m.add_function(wrap_pyfunction!(core_api::utc_to_tt, m)?)?;
    m.add_function(wrap_pyfunction!(core_api::tt_to_utc, m)?)?;
    m.add_function(wrap_pyfunction!(core_api::leap_second_table_info, m)?)?;
    m.add_function(wrap_pyfunction!(core_api::leap_second_table_expired, m)?)?;
    m.add_function(wrap_pyfunction!(core_api::angular_separation, m)?)?;

    // Astro: ICRS <-> Galactic, ERA/GMST, WCS.
    m.add_function(wrap_pyfunction!(astro::icrs_to_galactic, m)?)?;
    m.add_function(wrap_pyfunction!(astro::galactic_to_icrs, m)?)?;
    m.add_function(wrap_pyfunction!(astro::earth_rotation_angle, m)?)?;
    m.add_function(wrap_pyfunction!(astro::gmst_iau2006, m)?)?;
    m.add_class::<astro::Wcs>()?;

    // Array: local Zarr v2/v3 directories and lazy reductions.
    m.add_class::<array::ZarrArray>()?;
    m.add_function(wrap_pyfunction!(array::open_zarr, m)?)?;
    Ok(())
}
