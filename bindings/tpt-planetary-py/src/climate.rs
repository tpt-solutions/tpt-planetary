//! Climate indices on a Zarr field of shape `[time, lat, lon]`, computed with the
//! lazy pipelines of `tpt_planetary_array::ops::climate`. The field is streamed
//! from the store with the GIL released; only the per-month values are returned.

use numpy::IntoPyArray;
use pyo3::prelude::*;

use tpt_planetary_array::ops::climate;

use crate::array::ZarrArray;
use crate::errors::array_error;

/// The Niño-3.4 SST anomaly index, one value per month.
///
/// ``field`` is a ZarrArray of shape ``(time, lat, lon)`` with monthly steps.
/// ``lat_deg`` and ``lon_deg`` give its coordinates in degrees: latitudes
/// increasing, longitudes in degrees east and increasing. The anomaly is taken
/// against the monthly climatology of the record; the index averages 5°S–5°N,
/// 190°E–240°E. Raises ``TptPlanetaryError`` for a shape or coordinate mismatch.
#[pyfunction]
pub fn nino34<'py>(
    py: Python<'py>,
    field: PyRef<'py, ZarrArray>,
    lat_deg: Vec<f64>,
    lon_deg: Vec<f64>,
) -> PyResult<Bound<'py, numpy::PyArray1<f64>>> {
    let expr = &field.expr;
    let values = py
        .detach(|| climate::nino34(expr, &lat_deg, &lon_deg))
        .map_err(array_error)?;
    Ok(values.into_pyarray(py))
}

/// The relative Niño-3.4 index, one value per month: the Niño-3.4 anomaly minus
/// the 20°S–20°N mean anomaly of the same month. Arguments as for ``nino34``.
#[pyfunction]
pub fn nino34_relative<'py>(
    py: Python<'py>,
    field: PyRef<'py, ZarrArray>,
    lat_deg: Vec<f64>,
    lon_deg: Vec<f64>,
) -> PyResult<Bound<'py, numpy::PyArray1<f64>>> {
    let expr = &field.expr;
    let values = py
        .detach(|| climate::nino34_relative(expr, &lat_deg, &lon_deg))
        .map_err(array_error)?;
    Ok(values.into_pyarray(py))
}
