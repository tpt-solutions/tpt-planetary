//! Mapping from the Rust crates' error types to Python exceptions.
//!
//! - `ValueError`: bad input (invalid dates, selections, headers, array lengths).
//! - `OSError` (`FileNotFoundError` for a missing node): I/O failures.
//! - `TptPlanetaryError`: everything else, including library data problems
//!   such as an expired leap-second table.

use pyo3::create_exception;
use pyo3::exceptions::{PyFileNotFoundError, PyOSError, PyValueError};
use pyo3::prelude::*;

create_exception!(
    tpt_planetary,
    TptPlanetaryError,
    pyo3::exceptions::PyException,
    "Base class for tpt-planetary errors that are neither bad input nor I/O failures."
);

/// Maps a `tpt-planetary-core` error.
pub(crate) fn core_error(e: tpt_planetary_core::Error) -> PyErr {
    use tpt_planetary_core::Error as E;
    let msg = e.to_string();
    match e {
        E::InvalidDate
        | E::InvalidTime
        | E::LeapSecondNotInTable
        | E::BeforeLeapSecondTable
        | E::NonConvergence
        | E::ZeroVector
        | E::Antipodal
        | E::InvalidRotation => PyValueError::new_err(msg),
        _ => TptPlanetaryError::new_err(msg),
    }
}

/// Maps a `tpt-planetary-array` error.
pub(crate) fn array_error(e: tpt_planetary_array::Error) -> PyErr {
    use tpt_planetary_array::Error as E;
    let msg = e.to_string();
    match e {
        E::Io(_) => PyOSError::new_err(msg),
        E::NotFound(_) => PyFileNotFoundError::new_err(msg),
        E::InvalidSelection(_) | E::SizeMismatch { .. } => PyValueError::new_err(msg),
        _ => TptPlanetaryError::new_err(msg),
    }
}

/// Maps a `tpt-planetary-astro` WCS error (all are bad header input).
pub(crate) fn wcs_error(e: tpt_planetary_astro::wcs::WcsError) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// Builds a `ValueError` with the given message.
pub(crate) fn value_error(msg: impl Into<String>) -> PyErr {
    PyValueError::new_err(msg.into())
}
