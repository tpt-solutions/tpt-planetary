//! Bindings for `tpt-planetary-astro`: ICRS <-> Galactic transforms, Earth
//! rotation angles (ERA, GMST) and FITS-style celestial WCS (TAN, SIN, AIT).

use numpy::{IntoPyArray, PyArray1, PyReadonlyArray1};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyString};

use tpt_planetary_astro::frames::{
    Atmosphere, Galactic, GeodeticObserver, Icrs, SkyCoord, TransformContext,
    icrs_to_altaz_refracted_at,
};
use tpt_planetary_astro::time;
use tpt_planetary_astro::wcs::Wcs as CoreWcs;

use crate::core_api::JulianDate;
use crate::errors::{value_error, wcs_error};

type ArrayPair<'py> = (Bound<'py, PyArray1<f64>>, Bound<'py, PyArray1<f64>>);

/// Applies `f` element-wise to two equal-length float64 arrays and returns two
/// new arrays. Arrays are read through strided views, so no input copy is made.
fn map_pairs<'py>(
    py: Python<'py>,
    a: &PyReadonlyArray1<'py, f64>,
    b: &PyReadonlyArray1<'py, f64>,
    f: impl Fn(f64, f64) -> (f64, f64),
) -> PyResult<ArrayPair<'py>> {
    let (av, bv) = (a.as_array(), b.as_array());
    if av.len() != bv.len() {
        return Err(value_error(format!(
            "arrays must have the same length (got {} and {})",
            av.len(),
            bv.len()
        )));
    }
    let mut xs = Vec::with_capacity(av.len());
    let mut ys = Vec::with_capacity(av.len());
    for (&x, &y) in av.iter().zip(bv.iter()) {
        let (p, q) = f(x, y);
        xs.push(p);
        ys.push(q);
    }
    Ok((xs.into_pyarray(py), ys.into_pyarray(py)))
}

/// Converts ICRS (RA, Dec) in degrees to Galactic (l, b) in degrees.
///
/// Inputs are 1-D float64 arrays of equal length. Returns two float64 arrays
/// `(l, b)`, with `l` in `[0, 360)` and `b` in `[-90, 90]`.
#[pyfunction]
pub fn icrs_to_galactic<'py>(
    py: Python<'py>,
    ra: PyReadonlyArray1<'py, f64>,
    dec: PyReadonlyArray1<'py, f64>,
) -> PyResult<ArrayPair<'py>> {
    map_pairs(py, &ra, &dec, |ra, dec| {
        let c = SkyCoord::<Icrs>::from_degrees(ra, dec).transform_to::<Galactic>();
        (c.lon_deg(), c.lat_deg())
    })
}

/// Converts Galactic (l, b) in degrees to ICRS (RA, Dec) in degrees.
///
/// Same shape rules as `icrs_to_galactic`. Returns `(ra, dec)` arrays.
#[pyfunction]
pub fn galactic_to_icrs<'py>(
    py: Python<'py>,
    l: PyReadonlyArray1<'py, f64>,
    b: PyReadonlyArray1<'py, f64>,
) -> PyResult<ArrayPair<'py>> {
    map_pairs(py, &l, &b, |l, b| {
        let c = SkyCoord::<Galactic>::from_degrees(l, b).transform_to::<Icrs>();
        (c.lon_deg(), c.lat_deg())
    })
}

/// Earth Rotation Angle in radians, from a UT1 Julian Date.
///
/// The Julian Date's two components are passed to the Rust function as
/// `(day, fraction)`, so precision is kept.
#[pyfunction]
pub fn earth_rotation_angle(ut1: &JulianDate) -> f64 {
    time::earth_rotation_angle(ut1.inner.day(), ut1.inner.fraction())
}

/// Greenwich Mean Sidereal Time (IAU 2006) in radians.
///
/// `ut1` and `tt` are Julian Dates in UT1 and TT respectively.
#[pyfunction]
pub fn gmst_iau2006(ut1: &JulianDate, tt: &JulianDate) -> f64 {
    time::gmst_iau2006(
        ut1.inner.day(),
        ut1.inner.fraction(),
        tt.inner.day(),
        tt.inner.fraction(),
    )
}

/// Serialises a header dict to FITS 80-column cards (one per line).
///
/// Values may be `bool` (written as `T`/`F`), `str` (written as a quoted FITS
/// string) or a finite real number. Keywords must be ASCII and at most 8
/// characters.
fn header_text(header: &Bound<'_, PyDict>) -> PyResult<String> {
    let mut text = String::new();
    for (k, v) in header.iter() {
        let key: String = k
            .extract()
            .map_err(|_| value_error("header keys must be strings"))?;
        if key.is_empty() || key.len() > 8 || !key.is_ascii() || key.contains(char::is_whitespace) {
            return Err(value_error(format!(
                "'{key}' is not a valid FITS keyword (ASCII, 1 to 8 characters, no spaces)"
            )));
        }
        let value = if v.is_instance_of::<PyBool>() {
            if v.is_truthy()? {
                "T".to_string()
            } else {
                "F".to_string()
            }
        } else if v.is_instance_of::<PyString>() {
            let s: String = v.extract()?;
            if s.contains('\'') || !s.is_ascii() {
                return Err(value_error(format!(
                    "value of '{key}' must be ASCII without single quotes"
                )));
            }
            format!("'{s}'")
        } else {
            let x: f64 = v.extract().map_err(|_| {
                value_error(format!("value of '{key}' must be str, bool or number"))
            })?;
            if !x.is_finite() {
                return Err(value_error(format!("value of '{key}' must be finite")));
            }
            format!("{x:?}")
        };
        text.push_str(&format!("{key:<8}= {value}\n"));
    }
    Ok(text)
}

/// Celestial World Coordinate System (FITS WCS, one of TAN, SIN or AIT).
///
/// Build it from a FITS-style header dict with `Wcs.from_header`. The
/// transforms take 1-D float64 arrays and return float64 arrays. Pixel
/// coordinates are FITS 1-based. Positions outside the projection's domain
/// return NaN.
#[pyclass(name = "Wcs", module = "tpt_planetary", frozen)]
pub struct Wcs {
    inner: CoreWcs,
}

#[pymethods]
impl Wcs {
    /// Builds a WCS from a header dict such as
    /// `{"CRPIX1": 1.0, "CRPIX2": 1.0, "CRVAL1": 10.0, "CRVAL2": 20.0,
    ///   "CTYPE1": "RA---TAN", "CTYPE2": "DEC--TAN", "CDELT1": -0.001,
    ///   "CDELT2": 0.001}`.
    ///
    /// Supports `CRPIXn`, `CRVALn`, `CDELTn`, `CTYPEn`, `PCi_j`, `CDi_j`,
    /// `LONPOLE` and `LATPOLE`. Raises `ValueError` for a missing or unsupported
    /// projection, mismatched axes or a singular matrix.
    #[staticmethod]
    fn from_header(header: &Bound<'_, PyDict>) -> PyResult<Self> {
        let text = header_text(header)?;
        let inner = CoreWcs::from_header(&text).map_err(wcs_error)?;
        Ok(Self { inner })
    }

    /// Reference world coordinate `(CRVAL1, CRVAL2)` in degrees.
    #[getter]
    fn crval(&self) -> (f64, f64) {
        self.inner.crval_deg()
    }

    /// Converts FITS 1-based pixel positions to `(lon, lat)` in degrees.
    fn pixel_to_world<'py>(
        &self,
        py: Python<'py>,
        x: PyReadonlyArray1<'py, f64>,
        y: PyReadonlyArray1<'py, f64>,
    ) -> PyResult<ArrayPair<'py>> {
        map_pairs(py, &x, &y, |x, y| {
            self.inner
                .pixel_to_world(x, y)
                .unwrap_or((f64::NAN, f64::NAN))
        })
    }

    /// Converts `(lon, lat)` in degrees to FITS 1-based pixel positions `(x, y)`.
    fn world_to_pixel<'py>(
        &self,
        py: Python<'py>,
        lon: PyReadonlyArray1<'py, f64>,
        lat: PyReadonlyArray1<'py, f64>,
    ) -> PyResult<ArrayPair<'py>> {
        map_pairs(py, &lon, &lat, |lon, lat| {
            self.inner
                .world_to_pixel(lon, lat)
                .unwrap_or((f64::NAN, f64::NAN))
        })
    }
}

/// Converts ICRS (RA, Dec) in degrees to local Alt/Az for a geodetic observer.
///
/// `ra` and `dec` are float64 arrays of equal length. The observer is WGS84
/// geodetic longitude (east), latitude and height. `tt` and `ut1` are
/// JulianDate values, and `xp_arcsec`, `yp_arcsec` the polar motion (zero for
/// none). The result has no aberration or parallax. Refraction is applied when
/// `pressure_hpa` is positive, with the ERFA refraction model; zero pressure
/// gives the geometric position.
///
/// Returns two float64 arrays `(az, alt)` in degrees, azimuth north through
/// east in `[0, 360)`.
#[pyfunction]
#[pyo3(signature = (
    ra,
    dec,
    lon_deg,
    lat_deg,
    height_m,
    tt,
    ut1,
    xp_arcsec = 0.0,
    yp_arcsec = 0.0,
    pressure_hpa = 0.0,
    temperature_c = 15.0,
    relative_humidity = 0.5,
    wavelength_um = 0.55
))]
#[allow(clippy::too_many_arguments)]
pub fn icrs_to_altaz<'py>(
    py: Python<'py>,
    ra: PyReadonlyArray1<'py, f64>,
    dec: PyReadonlyArray1<'py, f64>,
    lon_deg: f64,
    lat_deg: f64,
    height_m: f64,
    tt: PyRef<'py, JulianDate>,
    ut1: PyRef<'py, JulianDate>,
    xp_arcsec: f64,
    yp_arcsec: f64,
    pressure_hpa: f64,
    temperature_c: f64,
    relative_humidity: f64,
    wavelength_um: f64,
) -> PyResult<ArrayPair<'py>> {
    let ctx = TransformContext::new(
        (tt.inner.day(), tt.inner.fraction()),
        (ut1.inner.day(), ut1.inner.fraction()),
        xp_arcsec,
        yp_arcsec,
    );
    let observer = GeodeticObserver::from_degrees(lon_deg, lat_deg, height_m);
    let atmosphere = Atmosphere {
        pressure_hpa,
        temperature_c,
        relative_humidity,
        wavelength_um,
    };
    map_pairs(py, &ra, &dec, |ra, dec| {
        let star = SkyCoord::<Icrs>::from_degrees(ra, dec);
        let a = icrs_to_altaz_refracted_at(star, observer, &ctx, atmosphere);
        (a.lon_deg(), a.lat_deg())
    })
}
