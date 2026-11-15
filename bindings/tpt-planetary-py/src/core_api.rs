//! Bindings for `tpt-planetary-core`: Julian Dates, UTC/TAI/TT conversion,
//! the leap-second table and angular separation.

use numpy::{IntoPyArray, PyArray1, PyReadonlyArray1};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use tpt_planetary_core::angle::Degrees;
use tpt_planetary_core::calendar::{CalendarSystem, CivilDate};
use tpt_planetary_core::jd::JulianDate as CoreJd;
use tpt_planetary_core::leap::EMBEDDED;
use tpt_planetary_core::scale::{Epoch, ScaleConvert, Tai, TimeScale, Tt, Utc};
use tpt_planetary_core::sphere::{SphericalCoord, angular_separation as core_separation};

use crate::errors::{core_error, value_error};

/// Two-part Julian Date: an integer `day` and a `fraction` in `[0, 1)`.
///
/// Keeping the two parts separate preserves sub-microsecond precision that a
/// single float64 cannot hold near J2000. The scale (UTC, TAI or TT) is not
/// stored; the caller states it and the conversion functions use it.
#[pyclass(
    name = "JulianDate",
    module = "tpt_planetary",
    frozen,
    eq,
    from_py_object
)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JulianDate {
    pub(crate) inner: CoreJd,
}

/// Largest magnitude accepted for the integer part of a Julian Date. It is far
/// beyond the i32 year range (about 7.8e11 days). The core crate's calendar code
/// overflows i64 arithmetic (and panics in debug builds) for values near 1e19,
/// so the binding rejects anything larger before it reaches the core.
const MAX_ABS_JD_DAY: f64 = 1.0e12;

fn finite(name: &str, v: f64) -> PyResult<f64> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(value_error(format!("{name} must be finite, got {v}")))
    }
}

/// Checks a Julian Date's integer part against `MAX_ABS_JD_DAY`.
fn check_jd(jd: CoreJd) -> PyResult<JulianDate> {
    if jd.day().abs() <= MAX_ABS_JD_DAY {
        Ok(JulianDate { inner: jd })
    } else {
        Err(value_error(format!(
            "Julian Date day must satisfy |day| <= {MAX_ABS_JD_DAY:e}, got {}",
            jd.day()
        )))
    }
}

/// Parses the `calendar` argument: `"gregorian"` or `"julian"`.
fn parse_calendar(name: &str) -> PyResult<CalendarSystem> {
    match name.to_ascii_lowercase().as_str() {
        "gregorian" => Ok(CalendarSystem::Gregorian),
        "julian" => Ok(CalendarSystem::Julian),
        other => Err(value_error(format!(
            "calendar must be 'gregorian' or 'julian', got '{other}'"
        ))),
    }
}

/// Checks a calendar field against its range. Out-of-range values are
/// rejected here rather than wrapped by a narrowing cast.
fn checked_field(name: &str, v: i64, lo: i64, hi: i64) -> PyResult<i64> {
    if (lo..=hi).contains(&v) {
        Ok(v)
    } else {
        Err(value_error(format!(
            "{name} must be in {lo}..={hi}, got {v}"
        )))
    }
}

#[pymethods]
impl JulianDate {
    /// Creates a Julian Date from `day + frac`. The fraction may be negative or
    /// exceed 1; the value is normalised.
    #[new]
    #[pyo3(signature = (day, frac = 0.0))]
    fn new(day: f64, frac: f64) -> PyResult<Self> {
        check_jd(CoreJd::new(finite("day", day)?, finite("frac", frac)?))
    }

    /// Builds a Julian Date from a calendar date and seconds since midnight.
    ///
    /// `seconds_of_day` must be in `[0, 86400)`. A leap second (23:59:60)
    /// cannot be represented here; convert a UTC value with `utc_to_tai` instead.
    #[staticmethod]
    #[pyo3(signature = (year, month, day, seconds_of_day = 0.0, calendar = "gregorian"))]
    fn from_calendar(
        year: i64,
        month: i64,
        day: i64,
        seconds_of_day: f64,
        calendar: &str,
    ) -> PyResult<Self> {
        let cal = parse_calendar(calendar)?;
        let year = checked_field("year", year, i32::MIN as i64, i32::MAX as i64)?;
        let month = checked_field("month", month, 1, 12)?;
        let day = checked_field("day", day, 1, 31)?;
        let secs = finite("seconds_of_day", seconds_of_day)?;
        let date = CivilDate::new(year as i32, month as u8, day as u8);
        let jd = CoreJd::from_calendar(date, cal, secs).map_err(core_error)?;
        Ok(Self { inner: jd })
    }

    /// Returns `(year, month, day, seconds_of_day)` for this Julian Date.
    #[pyo3(signature = (calendar = "gregorian"))]
    #[allow(clippy::wrong_self_convention)] // Python method name; the Python API is fixed.
    fn to_calendar(&self, calendar: &str) -> PyResult<(i32, u8, u8, f64)> {
        let cal = parse_calendar(calendar)?;
        let (date, secs) = self.inner.to_calendar(cal).map_err(core_error)?;
        Ok((date.year, date.month, date.day, secs))
    }

    /// Integer part of the Julian Date (first component).
    #[getter]
    fn day(&self) -> f64 {
        self.inner.day()
    }

    /// Fraction of the day in `[0, 1)` (second component).
    #[getter]
    fn fraction(&self) -> f64 {
        self.inner.fraction()
    }

    /// Single float64 approximation `day + fraction`. This loses precision near
    /// J2000; use the two components for precise work.
    #[allow(clippy::wrong_self_convention)] // Python method name; the Python API is fixed.
    fn to_float(&self) -> f64 {
        self.inner.to_f64()
    }

    /// Modified Julian Date as a `(day, fraction)` tuple.
    #[allow(clippy::wrong_self_convention)] // Python method name; the Python API is fixed.
    fn to_mjd(&self) -> (f64, f64) {
        let m = self.inner.to_mjd();
        (m.day(), m.fraction())
    }

    /// Returns a new Julian Date `days` later.
    fn add_days(&self, days: f64) -> PyResult<Self> {
        check_jd(self.inner.add_days(finite("days", days)?))
    }

    /// Returns a new Julian Date `seconds` later.
    fn add_seconds(&self, seconds: f64) -> PyResult<Self> {
        check_jd(self.inner.add_seconds(finite("seconds", seconds)?))
    }

    /// Days from `other` to `self` (negative if `self` is earlier).
    fn days_since(&self, other: &JulianDate) -> f64 {
        self.inner.days_since(other.inner)
    }

    /// Seconds from `other` to `self`.
    fn seconds_since(&self, other: &JulianDate) -> f64 {
        self.inner.seconds_since(other.inner)
    }

    fn __repr__(&self) -> String {
        format!(
            "JulianDate(day={:?}, fraction={:?})",
            self.inner.day(),
            self.inner.fraction()
        )
    }
}

/// Converts between two time scales through the embedded leap-second table.
fn convert<S, T>(jd: &JulianDate) -> PyResult<JulianDate>
where
    S: TimeScale + ScaleConvert<T>,
    T: TimeScale,
{
    let src: Epoch<S> = Epoch::from_jd(jd.inner);
    let dst: Epoch<T> = src.to_scale(&EMBEDDED).map_err(core_error)?;
    Ok(JulianDate { inner: dst.jd() })
}

/// Converts a UTC Julian Date to TAI using the embedded leap-second table.
#[pyfunction]
pub fn utc_to_tai(jd: &JulianDate) -> PyResult<JulianDate> {
    convert::<Utc, Tai>(jd)
}

/// Converts a TAI Julian Date to UTC using the embedded leap-second table.
#[pyfunction]
pub fn tai_to_utc(jd: &JulianDate) -> PyResult<JulianDate> {
    convert::<Tai, Utc>(jd)
}

/// Converts a TAI Julian Date to TT (fixed offset of 32.184 s).
#[pyfunction]
pub fn tai_to_tt(jd: &JulianDate) -> PyResult<JulianDate> {
    convert::<Tai, Tt>(jd)
}

/// Converts a TT Julian Date to TAI (fixed offset of 32.184 s).
#[pyfunction]
pub fn tt_to_tai(jd: &JulianDate) -> PyResult<JulianDate> {
    convert::<Tt, Tai>(jd)
}

/// Converts a UTC Julian Date to TT.
#[pyfunction]
pub fn utc_to_tt(jd: &JulianDate) -> PyResult<JulianDate> {
    convert::<Utc, Tt>(jd)
}

/// Converts a TT Julian Date to UTC.
#[pyfunction]
pub fn tt_to_utc(jd: &JulianDate) -> PyResult<JulianDate> {
    convert::<Tt, Utc>(jd)
}

/// Describes the embedded leap-second table.
///
/// Returns a dict with `version` (int), `source` (str), `valid_until`
/// (`(year, month, day)`: last day the table is trusted for), `entries`
/// (number of leap seconds) and `last_tai_minus_utc` (seconds).
#[pyfunction]
pub fn leap_second_table_info(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let t = &EMBEDDED;
    let until = t.valid_until();
    let last = t.entries().last().map(|e| e.tai_minus_utc).unwrap_or(0);
    let d = PyDict::new(py);
    d.set_item("version", t.version())?;
    d.set_item("source", t.source())?;
    d.set_item("valid_until", (until.year, until.month, until.day))?;
    d.set_item("entries", t.entries().len())?;
    d.set_item("last_tai_minus_utc", last)?;
    Ok(d)
}

/// True if the embedded leap-second table does not cover the civil day of `jd`.
/// UTC conversions for such a date raise `TptPlanetaryError`.
#[pyfunction]
pub fn leap_second_table_expired(jd: &JulianDate) -> bool {
    EMBEDDED.is_expired_at(jd.inner)
}

/// Great-circle separation between pairs of sky positions, in degrees.
///
/// All four arguments are 1-D float64 NumPy arrays of equal length; `lon` and
/// `lat` are in degrees. Returns a float64 array of separations in degrees.
/// Non-finite inputs give NaN. Arrays of another dtype raise `TypeError`.
#[pyfunction]
pub fn angular_separation<'py>(
    py: Python<'py>,
    lon1: PyReadonlyArray1<'py, f64>,
    lat1: PyReadonlyArray1<'py, f64>,
    lon2: PyReadonlyArray1<'py, f64>,
    lat2: PyReadonlyArray1<'py, f64>,
) -> PyResult<Bound<'py, PyArray1<f64>>> {
    let (a, b, c, d) = (
        lon1.as_array(),
        lat1.as_array(),
        lon2.as_array(),
        lat2.as_array(),
    );
    let n = a.len();
    if b.len() != n || c.len() != n || d.len() != n {
        return Err(value_error(format!(
            "all arrays must have the same length (got {n}, {}, {}, {})",
            b.len(),
            c.len(),
            d.len()
        )));
    }
    let out: Vec<f64> = (0..n)
        .map(|i| {
            let p = SphericalCoord::new(Degrees(a[i]).to_radians(), Degrees(b[i]).to_radians());
            let q = SphericalCoord::new(Degrees(c[i]).to_radians(), Degrees(d[i]).to_radians());
            Degrees::from(core_separation(p, q)).0
        })
        .collect();
    Ok(out.into_pyarray(py))
}
