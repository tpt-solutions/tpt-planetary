//! Bindings for SGP4 / SDP4 propagation of two-line element sets.
//!
//! A `Satellite` is initialised from the two TLE lines (checksums are checked)
//! and propagated to a time in minutes from the TLE epoch. States are TEME, in km
//! and km/s. Near-Earth and deep-space (period of 225 minutes or more) orbits are
//! both handled.

use pyo3::prelude::*;
use pyo3::types::PyTuple;

use tpt_planetary_astro::orbit::{OrbitError, Sgp4};

use crate::errors::{TptPlanetaryError, value_error};

fn orbit_error(e: OrbitError) -> PyErr {
    match e {
        OrbitError::InvalidTle { .. }
        | OrbitError::TleChecksum { .. }
        | OrbitError::TleSatelliteMismatch => value_error(e.to_string()),
        _ => TptPlanetaryError::new_err(e.to_string()),
    }
}

/// An SGP4/SDP4 satellite initialised from a two-line element set.
#[pyclass(name = "Satellite", module = "tpt_planetary", frozen)]
pub struct Satellite {
    sgp4: Sgp4,
}

#[pymethods]
impl Satellite {
    /// Initialises the propagator from the two TLE lines. Raises ``ValueError`` for
    /// a malformed or mismatched element set (including a bad checksum), and
    /// ``TptPlanetaryError`` for elements the model cannot use (for example a zero
    /// mean motion).
    #[staticmethod]
    fn from_tle(line1: &str, line2: &str) -> PyResult<Self> {
        Ok(Self {
            sgp4: Sgp4::from_tle_lines(line1, line2).map_err(orbit_error)?,
        })
    }

    /// Epoch of the element set as a two-part Julian date ``(day, fraction)``.
    #[getter]
    fn epoch<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let jd = self.sgp4.epoch();
        PyTuple::new(py, [jd.day(), jd.fraction()])
    }

    /// TEME position (km) and velocity (km/s) at ``minutes`` after the epoch, as a
    /// tuple ``(position, velocity)``. Raises ``TptPlanetaryError`` when the model
    /// reports the orbit as decayed or the elements invalid at that time.
    fn propagate<'py>(
        &self,
        py: Python<'py>,
        minutes: f64,
    ) -> PyResult<(Bound<'py, PyTuple>, Bound<'py, PyTuple>)> {
        let s = self.sgp4.propagate(minutes).map_err(orbit_error)?;
        Ok((
            PyTuple::new(py, s.position_km)?,
            PyTuple::new(py, s.velocity_km_s)?,
        ))
    }
}
