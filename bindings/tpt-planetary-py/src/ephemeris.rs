//! Bindings for JPL SPK kernels: geometric states, and astrometric and apparent
//! directions (light-time corrected; apparent also has the Sun's light deflection
//! and the annual aberration applied).
//!
//! Times are TDB as seconds past J2000 (`et`); `et_from_tdb` converts a TDB
//! Julian date. A kernel is read into memory when opened.

use std::path::PathBuf;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

use tpt_planetary_astro::ephemeris::{
    SpkFile, apparent_direction, astrometric, et_from_tdb_jd, read_file,
};

use crate::errors::{TptPlanetaryError, value_error};

fn ephemeris_error(e: impl std::fmt::Display) -> PyErr {
    TptPlanetaryError::new_err(e.to_string())
}

/// A DE440-style SPK kernel held in memory.
#[pyclass(name = "SpkKernel", module = "tpt_planetary", frozen)]
pub struct SpkKernel {
    bytes: Vec<u8>,
}

impl SpkKernel {
    fn file(&self) -> PyResult<SpkFile<'_>> {
        SpkFile::new(&self.bytes).map_err(ephemeris_error)
    }
}

/// Opens the SPK kernel at `path` and reads it into memory.
///
/// Raises `OSError` if the file cannot be read and `TptPlanetaryError` if it is
/// not an SPK file. Kernels are not bundled with the package: obtain one from
/// JPL (for example de440s.bsp).
#[pyfunction]
pub fn open_spk(path: PathBuf) -> PyResult<SpkKernel> {
    let bytes = read_file(&path).map_err(|e| {
        pyo3::exceptions::PyOSError::new_err(format!("cannot read '{}': {e}", path.display()))
    })?;
    let kernel = SpkKernel { bytes };
    kernel.file()?;
    Ok(kernel)
}

/// Seconds past J2000 TDB for a TDB Julian date given in two parts.
#[pyfunction]
pub fn et_from_tdb(jd_hi: f64, jd_lo: f64) -> PyResult<f64> {
    if !jd_hi.is_finite() || !jd_lo.is_finite() {
        return Err(value_error("Julian date parts must be finite"));
    }
    Ok(et_from_tdb_jd(jd_hi, jd_lo))
}

#[pymethods]
impl SpkKernel {
    /// Position (km) and velocity (km/s) of `target` relative to `center`, in the
    /// kernel's ICRF axes, as a tuple `(position, velocity)`.
    fn state<'py>(
        &self,
        py: Python<'py>,
        target: i32,
        center: i32,
        et: f64,
    ) -> PyResult<(
        Bound<'py, pyo3::types::PyTuple>,
        Bound<'py, pyo3::types::PyTuple>,
    )> {
        let s = self
            .file()?
            .state(target, center, et)
            .map_err(ephemeris_error)?;
        Ok((
            pyo3::types::PyTuple::new(py, s.position)?,
            pyo3::types::PyTuple::new(py, s.velocity)?,
        ))
    }

    /// Astrometric direction of `target` from `observer` at `et`: the
    /// light-time corrected direction without stellar aberration, as JPL Horizons
    /// gives astrometric positions. Returns a dict with `ra_deg`, `dec_deg`,
    /// `distance_km` and `light_time_s`.
    fn astrometric<'py>(
        &self,
        py: Python<'py>,
        target: i32,
        observer: i32,
        et: f64,
    ) -> PyResult<Bound<'py, PyDict>> {
        let a = astrometric(&self.file()?, target, observer, et).map_err(ephemeris_error)?;
        let d = PyDict::new(py);
        d.set_item("ra_deg", a.ra_rad.to_degrees())?;
        d.set_item("dec_deg", a.dec_rad.to_degrees())?;
        d.set_item("distance_km", a.distance_km)?;
        d.set_item("light_time_s", a.light_time_s)?;
        Ok(d)
    }

    /// Rotation from J2000 axes to the body-fixed frame `body_frame` (3000 for
    /// ITRF93) at `et`, from a binary PCK: a row-major 3 x 3 tuple of tuples, as
    /// SPICE's pxform gives it.
    fn orientation<'py>(
        &self,
        py: Python<'py>,
        body_frame: i32,
        et: f64,
    ) -> PyResult<Bound<'py, PyTuple>> {
        let m = tpt_planetary_astro::pck::orientation_matrix(&self.file()?, body_frame, et)
            .map_err(ephemeris_error)?;
        let rows = m
            .iter()
            .map(|r| PyTuple::new(py, r.to_vec()))
            .collect::<PyResult<Vec<_>>>()?;
        PyTuple::new(py, rows)
    }

    /// Apparent direction of `target` from `observer` at `et`: the astrometric
    /// direction with the Sun's light deflection and the annual aberration applied,
    /// in ICRF axes. Returns a dict with `ra_deg`, `dec_deg` and `light_time_s`.
    fn apparent<'py>(
        &self,
        py: Python<'py>,
        target: i32,
        observer: i32,
        et: f64,
    ) -> PyResult<Bound<'py, PyDict>> {
        let (dir, light_time_s) =
            apparent_direction(&self.file()?, target, observer, et).map_err(ephemeris_error)?;
        let d = PyDict::new(py);
        d.set_item("ra_deg", dir.lon_deg())?;
        d.set_item("dec_deg", dir.lat_deg())?;
        d.set_item("light_time_s", light_time_s)?;
        Ok(d)
    }
}
