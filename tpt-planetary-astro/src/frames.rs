//! Typed sky coordinates and frame transformations.
//!
//! A [`SkyCoord`] carries its frame as a type parameter, so a coordinate in
//! one frame cannot be passed where another is expected. Transformations are
//! expressed with the [`FrameTransform`] trait and are only available for the
//! frame pairs that are implemented.
//!
//! Implemented transforms:
//! - ICRS <-> Galactic (fixed rotation from the J2000 Galactic pole constants)
//! - ICRS <-> mean ecliptic of J2000 (IAU 1976 obliquity)
//! - ICRS -> true ecliptic with a caller-supplied obliquity (see
//!   [`TrueEcliptic`] for the limits)
//! - ICRS -> local horizontal (Alt/Az) for an observer and local sidereal time
//!
//! Angles are stored in radians. Constructors and accessors in degrees are
//! provided for convenience.

use core::f64::consts::{PI, TAU};
use core::marker::PhantomData;

use tpt_planetary_core::angle::Radians;
use tpt_planetary_core::ellipsoid::{Geodetic, WGS84};

/// Radians per degree.
const DEG: f64 = PI / 180.0;

/// Mean obliquity of the ecliptic at J2000 (IAU 1976): 84381.406 arcseconds.
const OBLIQUITY_J2000_ARCSEC: f64 = 84_381.406;

/// Galactic north pole, ICRS right ascension in degrees (IAU 1958 / Hipparcos).
const GALACTIC_POLE_RA_DEG: f64 = 192.859_48;

/// Galactic north pole, ICRS declination in degrees (IAU 1958 / Hipparcos).
const GALACTIC_POLE_DEC_DEG: f64 = 27.128_25;

/// Galactic longitude of the north celestial pole, in degrees (IAU 1958 / Hipparcos).
const GALACTIC_LON_OF_NCP_DEG: f64 = 122.931_92;

/// Marker trait for coordinate frames.
///
/// Implement this for a zero-sized type to add a frame. The name is used in
/// debug output and error messages.
pub trait Frame: Copy + core::fmt::Debug + 'static {
    /// Human-readable frame name.
    const NAME: &'static str;
}

/// Equatorial ICRS frame (the ICRS axes, J2000.0 realisation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Icrs;

/// Galactic frame defined by the IAU 1958 pole and node constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Galactic;

/// Mean ecliptic of J2000 (obliquity 84381.406 arcseconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MeanEcliptic;

/// Ecliptic with a caller-supplied obliquity.
///
/// The obliquity (mean obliquity plus nutation in obliquity, or any other
/// value the caller chooses) is applied as a rotation about the ICRS x-axis.
/// No precession of the equinox and no frame bias are applied, so this is
/// the true ecliptic only in the sense of the supplied obliquity relative to
/// the J2000 equinox. Full true-of-date ecliptic coordinates need precession
/// and nutation models, which are not implemented in this crate yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrueEcliptic;

/// Local horizontal frame (altitude and azimuth) for a fixed observer.
///
/// For `SkyCoord<AltAz>`, [`SkyCoord::lon`] is the azimuth measured from north
/// through east and [`SkyCoord::lat`] is the altitude. No atmospheric
/// refraction is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AltAz;

impl Frame for Icrs {
    const NAME: &'static str = "ICRS";
}
impl Frame for Galactic {
    const NAME: &'static str = "Galactic";
}
impl Frame for MeanEcliptic {
    const NAME: &'static str = "MeanEcliptic(J2000)";
}
impl Frame for TrueEcliptic {
    const NAME: &'static str = "TrueEcliptic";
}
impl Frame for AltAz {
    const NAME: &'static str = "AltAz";
}

/// A spherical coordinate in frame `F`, stored in radians.
///
/// `lon` is longitude-like (RA, galactic longitude, ecliptic longitude,
/// azimuth) and is normalised to `[0, 2*pi)`. `lat` is latitude-like and lies
/// in `[-pi/2, pi/2]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyCoord<F: Frame> {
    lon: f64,
    lat: f64,
    frame: PhantomData<F>,
}

impl<F: Frame> SkyCoord<F> {
    /// Creates a coordinate from longitude and latitude in radians.
    ///
    /// The longitude is wrapped to `[0, 2*pi)`. Latitudes outside
    /// `[-pi/2, pi/2]` are clamped.
    #[must_use]
    pub fn from_radians(lon: f64, lat: f64) -> Self {
        Self {
            lon: lon.rem_euclid(TAU),
            lat: lat.clamp(-PI / 2.0, PI / 2.0),
            frame: PhantomData,
        }
    }

    /// Creates a coordinate from longitude and latitude in degrees.
    #[must_use]
    pub fn from_degrees(lon_deg: f64, lat_deg: f64) -> Self {
        Self::from_radians(lon_deg * DEG, lat_deg * DEG)
    }

    /// Longitude-like component in radians, in `[0, 2*pi)`.
    #[must_use]
    pub fn lon(&self) -> f64 {
        self.lon
    }

    /// Latitude-like component in radians.
    #[must_use]
    pub fn lat(&self) -> f64 {
        self.lat
    }

    /// Longitude-like component in degrees, in `[0, 360)`.
    #[must_use]
    pub fn lon_deg(&self) -> f64 {
        self.lon / DEG
    }

    /// Latitude-like component in degrees.
    #[must_use]
    pub fn lat_deg(&self) -> f64 {
        self.lat / DEG
    }

    /// Unit vector of this direction in frame `F` (Cartesian components).
    #[must_use]
    pub fn to_unit_vector(&self) -> [f64; 3] {
        let (sl, cl) = self.lon.sin_cos();
        let (sb, cb) = self.lat.sin_cos();
        [cb * cl, cb * sl, sb]
    }

    /// Builds a coordinate from a (not necessarily unit) Cartesian vector.
    ///
    /// Returns `None` for the zero vector.
    #[must_use]
    pub fn from_vector(v: [f64; 3]) -> Option<Self> {
        let r = norm(v);
        if r == 0.0 {
            return None;
        }
        let lon = v[1].atan2(v[0]);
        let lat = (v[2] / r).clamp(-1.0, 1.0).asin();
        Some(Self::from_radians(lon, lat))
    }

    /// Transforms this coordinate into frame `T`.
    ///
    /// Requires an implementation of [`FrameTransform`] from `F` to `T`.
    #[must_use]
    pub fn transform_to<T: Frame>(self) -> SkyCoord<T>
    where
        F: FrameTransform<T>,
    {
        F::transform(self)
    }
}

/// Transformation from frame `Self` to frame `To`.
pub trait FrameTransform<To: Frame>: Frame {
    /// Applies the transformation.
    fn transform(coord: SkyCoord<Self>) -> SkyCoord<To>;
}

type Mat3 = [[f64; 3]; 3];

fn mat_vec(m: &Mat3, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn mat_mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn transpose(m: &Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = m[j][i];
        }
    }
    out
}

fn norm(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// Passive rotation about the z-axis by `theta` radians.
fn rot_z(theta: f64) -> Mat3 {
    let (s, c) = theta.sin_cos();
    [[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]]
}

/// Passive rotation about the y-axis by `theta` radians.
fn rot_y(theta: f64) -> Mat3 {
    let (s, c) = theta.sin_cos();
    [[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]]
}

/// Passive rotation about the x-axis by `theta` radians.
fn rot_x(theta: f64) -> Mat3 {
    let (s, c) = theta.sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, s], [0.0, -s, c]]
}

/// ICRS -> Galactic rotation matrix, built from the pole and node constants.
///
/// Construction: `T = Rz(180 deg - l_NCP) * Ry(90 deg - delta_G) * Rz(alpha_G)`,
/// where `(alpha_G, delta_G)` is the Galactic pole in ICRS and `l_NCP` is the
/// Galactic longitude of the north celestial pole. The rows of `T` are the
/// Galactic axes expressed in ICRS components.
fn icrs_to_galactic_matrix() -> Mat3 {
    let alpha_g = GALACTIC_POLE_RA_DEG * DEG;
    let delta_g = GALACTIC_POLE_DEC_DEG * DEG;
    let l_ncp = GALACTIC_LON_OF_NCP_DEG * DEG;
    let first = rot_z(alpha_g);
    let second = rot_y(PI / 2.0 - delta_g);
    let third = rot_z(PI - l_ncp);
    mat_mul(&third, &mat_mul(&second, &first))
}

/// Mean-obliquity rotation (about the x-axis) from ICRS to ecliptic coordinates.
fn icrs_to_ecliptic_matrix(obliquity_rad: f64) -> Mat3 {
    rot_x(obliquity_rad)
}

/// Mean obliquity of the ecliptic at J2000 in radians.
#[must_use]
pub fn mean_obliquity_j2000_rad() -> f64 {
    OBLIQUITY_J2000_ARCSEC / 3600.0 * DEG
}

impl FrameTransform<Galactic> for Icrs {
    fn transform(coord: SkyCoord<Icrs>) -> SkyCoord<Galactic> {
        let v = mat_vec(&icrs_to_galactic_matrix(), coord.to_unit_vector());
        SkyCoord::from_vector(v).unwrap_or_else(|| SkyCoord::from_radians(0.0, 0.0))
    }
}

impl FrameTransform<Icrs> for Galactic {
    fn transform(coord: SkyCoord<Galactic>) -> SkyCoord<Icrs> {
        let m = transpose(&icrs_to_galactic_matrix());
        let v = mat_vec(&m, coord.to_unit_vector());
        SkyCoord::from_vector(v).unwrap_or_else(|| SkyCoord::from_radians(0.0, 0.0))
    }
}

impl FrameTransform<MeanEcliptic> for Icrs {
    fn transform(coord: SkyCoord<Icrs>) -> SkyCoord<MeanEcliptic> {
        let m = icrs_to_ecliptic_matrix(mean_obliquity_j2000_rad());
        let v = mat_vec(&m, coord.to_unit_vector());
        SkyCoord::from_vector(v).unwrap_or_else(|| SkyCoord::from_radians(0.0, 0.0))
    }
}

impl FrameTransform<Icrs> for MeanEcliptic {
    fn transform(coord: SkyCoord<MeanEcliptic>) -> SkyCoord<Icrs> {
        let m = transpose(&icrs_to_ecliptic_matrix(mean_obliquity_j2000_rad()));
        let v = mat_vec(&m, coord.to_unit_vector());
        SkyCoord::from_vector(v).unwrap_or_else(|| SkyCoord::from_radians(0.0, 0.0))
    }
}

/// Converts an ICRS coordinate to ecliptic coordinates for a supplied obliquity.
///
/// `true_obliquity_rad` should be the mean obliquity plus the nutation in
/// obliquity, in radians. See [`TrueEcliptic`] for what this does and does not
/// include.
#[must_use]
pub fn icrs_to_true_ecliptic(
    coord: SkyCoord<Icrs>,
    true_obliquity_rad: f64,
) -> SkyCoord<TrueEcliptic> {
    let m = icrs_to_ecliptic_matrix(true_obliquity_rad);
    let v = mat_vec(&m, coord.to_unit_vector());
    SkyCoord::from_vector(v).unwrap_or_else(|| SkyCoord::from_radians(0.0, 0.0))
}

/// Converts an ICRS coordinate to ecliptic coordinates using the IAU 2006 mean
/// obliquity at a TT epoch.
///
/// The rotation is about the ICRS x-axis by the mean obliquity at the epoch,
/// from [`crate::time::mean_obliquity_iau2006_rad`]. Precession of the equinox
/// is not applied, so the origin stays on the J2000 equinox. The result is
/// therefore not the mean ecliptic of date. It is the ecliptic whose
/// obliquity is evaluated at the epoch, the same frame as [`TrueEcliptic`]
/// with the mean obliquity. At J2000.0 it equals [`MeanEcliptic`].
#[must_use]
pub fn icrs_to_ecliptic_at_epoch(
    coord: SkyCoord<Icrs>,
    tt_hi: f64,
    tt_lo: f64,
) -> SkyCoord<TrueEcliptic> {
    icrs_to_true_ecliptic(coord, crate::time::mean_obliquity_iau2006_rad(tt_hi, tt_lo))
}

/// Converts geodetic longitude, latitude and height to ITRS (ECEF)
/// Cartesian coordinates in metres, on the WGS84 ellipsoid.
///
/// `lon_rad` is east positive and `lat_rad` is geodetic latitude, north
/// positive. The closed form is in [`tpt_planetary_core::ellipsoid`].
#[must_use]
pub fn geodetic_to_itrs(lon_rad: f64, lat_rad: f64, height_m: f64) -> [f64; 3] {
    let v = WGS84.geodetic_to_ecef(Geodetic {
        longitude: Radians(lon_rad),
        latitude: Radians(lat_rad),
        height: height_m,
    });
    [v.x, v.y, v.z]
}

/// Polar motion rotation `W = R2(xp) R1(yp)` from ITRS to TIRS, in radians.
///
/// Passive rotations as in IERS Conventions 2010, Chapter 5. The TIO locator
/// `s'` is taken as zero, which changes positions by about 2e-10 rad
/// (about 1.5 mm on the surface). Under this convention the celestial
/// intermediate pole sits at `(xp, -yp, 1)` in ITRS, to first order. The sign
/// convention is recalled from memory and unvalidated.
#[must_use]
pub fn polar_motion_matrix(xp_rad: f64, yp_rad: f64) -> [[f64; 3]; 3] {
    mat_mul(&rot_y(xp_rad), &rot_x(yp_rad))
}

/// Applies the polar motion rotation to an ITRS vector, giving TIRS.
///
/// See [`polar_motion_matrix`] for the convention.
#[must_use]
pub fn itrs_to_tirs(v: [f64; 3], xp_rad: f64, yp_rad: f64) -> [f64; 3] {
    mat_vec(&polar_motion_matrix(xp_rad, yp_rad), v)
}

/// Geodetic observer location used for local horizontal coordinates.
///
/// Only the direction-relevant geodetic latitude and east longitude are used.
/// Height and polar motion are not applied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Observer {
    /// Geodetic latitude in radians, north positive.
    pub lat_rad: f64,
    /// Geodetic longitude in radians, east positive.
    pub lon_rad: f64,
}

impl Observer {
    /// Creates an observer from latitude and longitude in degrees.
    #[must_use]
    pub fn from_degrees(lat_deg: f64, lon_deg: f64) -> Self {
        Self {
            lat_rad: lat_deg * DEG,
            lon_rad: lon_deg * DEG,
        }
    }
}

/// Converts an ICRS coordinate to local horizontal (Alt/Az) coordinates.
///
/// `local_sidereal_time_rad` is the local apparent or mean sidereal time in
/// radians (GAST or GMST plus the observer's east longitude, as the caller
/// chooses). The ICRS right ascension is used as if it were the apparent right
/// ascension of date, so precession, nutation, aberration and refraction are
/// not included. Azimuth is measured from north through east.
#[must_use]
pub fn icrs_to_altaz(
    coord: SkyCoord<Icrs>,
    observer: Observer,
    local_sidereal_time_rad: f64,
) -> SkyCoord<AltAz> {
    let ra = coord.lon();
    let dec = coord.lat();
    let hour_angle = local_sidereal_time_rad - ra;
    let (sin_h, cos_h) = hour_angle.sin_cos();
    let (sin_d, cos_d) = dec.sin_cos();
    let (sin_p, cos_p) = observer.lat_rad.sin_cos();

    let sin_alt = (sin_p * sin_d + cos_p * cos_d * cos_h).clamp(-1.0, 1.0);
    let alt = sin_alt.asin();
    let az = (-sin_h * cos_d).atan2(cos_p * sin_d - sin_p * cos_d * cos_h);
    SkyCoord::from_radians(az, alt)
}

/// Returns the local sidereal time in radians for an observer, given GAST or
/// GMST in radians and the observer's east longitude in radians.
#[must_use]
pub fn local_sidereal_time(sidereal_time_rad: f64, observer: Observer) -> f64 {
    (sidereal_time_rad + observer.lon_rad).rem_euclid(TAU)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close_deg(a: f64, b: f64, tol: f64) -> bool {
        let d = (a - b).rem_euclid(360.0);
        d.min(360.0 - d) < tol
    }

    #[test]
    #[allow(clippy::excessive_precision, clippy::needless_range_loop)]
    fn icrs_galactic_matrix_matches_published_hipparcos_values() {
        // Published Hipparcos ICRS -> Galactic matrix (from memory; used as a
        // consistency check on the construction only, not a SOFA vector).
        let expected: Mat3 = [
            [
                -0.054_875_560_416_215_4,
                -0.873_437_090_234_885_0,
                -0.483_835_015_548_713_2,
            ],
            [
                0.494_109_427_875_583_7,
                -0.444_829_629_960_011_2,
                0.746_982_244_497_218_9,
            ],
            [
                -0.867_666_149_019_004_7,
                -0.198_076_373_431_201_5,
                0.455_983_776_175_066_9,
            ],
        ];
        let m = icrs_to_galactic_matrix();
        for i in 0..3 {
            for j in 0..3 {
                assert!(
                    (m[i][j] - expected[i][j]).abs() < 1e-9,
                    "({i},{j}) {} vs {}",
                    m[i][j],
                    expected[i][j]
                );
            }
        }
    }

    #[test]
    fn galactic_pole_maps_to_b_90() {
        let pole = SkyCoord::<Icrs>::from_degrees(192.859_48, 27.128_25);
        let g = pole.transform_to::<Galactic>();
        assert!((g.lat_deg() - 90.0).abs() < 1e-6, "b={}", g.lat_deg());
    }

    #[test]
    fn galactic_centre_maps_to_l0_b0() {
        // Galactic centre (l=0, b=0) in ICRS; reference point used for a consistency check.
        let gc = SkyCoord::<Icrs>::from_degrees(266.404_99, -28.936_17);
        let g = gc.transform_to::<Galactic>();
        assert!(close_deg(g.lon_deg(), 0.0, 1e-4), "l={}", g.lon_deg());
        assert!(g.lat_deg().abs() < 1e-4, "b={}", g.lat_deg());
    }

    #[test]
    fn icrs_galactic_round_trip() {
        for k in 0..30 {
            let c = SkyCoord::<Icrs>::from_degrees(f64::from(k) * 12.1, -80.0 + f64::from(k) * 5.3);
            let back = c.transform_to::<Galactic>().transform_to::<Icrs>();
            assert!(close_deg(back.lon_deg(), c.lon_deg(), 1e-9));
            assert!((back.lat_deg() - c.lat_deg()).abs() < 1e-9);
        }
    }

    #[test]
    fn icrs_north_pole_has_ecliptic_latitude_90_minus_obliquity() {
        let npole = SkyCoord::<Icrs>::from_degrees(0.0, 90.0);
        let e = npole.transform_to::<MeanEcliptic>();
        let expected_lat = 90.0 - OBLIQUITY_J2000_ARCSEC / 3600.0;
        assert!(
            (e.lat_deg() - expected_lat).abs() < 1e-9,
            "lat={}",
            e.lat_deg()
        );
        assert!(close_deg(e.lon_deg(), 90.0, 1e-9));
    }

    #[test]
    fn ecliptic_round_trip() {
        for k in 0..30 {
            let c = SkyCoord::<Icrs>::from_degrees(f64::from(k) * 11.9, 60.0 - f64::from(k) * 4.1);
            let back = c.transform_to::<MeanEcliptic>().transform_to::<Icrs>();
            assert!(close_deg(back.lon_deg(), c.lon_deg(), 1e-9));
            assert!((back.lat_deg() - c.lat_deg()).abs() < 1e-9);
        }
    }

    #[test]
    fn vernal_equinox_is_ecliptic_origin() {
        let eq = SkyCoord::<Icrs>::from_degrees(0.0, 0.0).transform_to::<MeanEcliptic>();
        assert!(close_deg(eq.lon_deg(), 0.0, 1e-9));
        assert!(eq.lat_deg().abs() < 1e-9);
    }

    #[test]
    fn true_ecliptic_with_mean_obliquity_equals_mean_ecliptic() {
        let c = SkyCoord::<Icrs>::from_degrees(123.4, -21.0);
        let a = icrs_to_true_ecliptic(c, mean_obliquity_j2000_rad());
        let b = c.transform_to::<MeanEcliptic>();
        assert!(close_deg(a.lon_deg(), b.lon_deg(), 1e-9));
        assert!((a.lat_deg() - b.lat_deg()).abs() < 1e-9);
    }

    #[test]
    fn zenith_is_at_altitude_90() {
        let obs = Observer::from_degrees(45.0, 0.0);
        // Object at dec = latitude, hour angle 0 -> zenith.
        let lst = 1.234;
        let ra = lst;
        let c = SkyCoord::<Icrs>::from_radians(ra, obs.lat_rad);
        let h = icrs_to_altaz(c, obs, lst);
        assert!((h.lat_deg() - 90.0).abs() < 1e-9, "alt={}", h.lat_deg());
    }

    #[test]
    fn southern_meridian_transit_is_due_south() {
        // Dec 35, observer latitude 45, on meridian: altitude 80, azimuth 180.
        let obs = Observer::from_degrees(45.0, 0.0);
        let lst = 0.7;
        let c = SkyCoord::<Icrs>::from_radians(lst, 35.0 * DEG);
        let h = icrs_to_altaz(c, obs, lst);
        assert!((h.lat_deg() - 80.0).abs() < 1e-9, "alt={}", h.lat_deg());
        assert!(close_deg(h.lon_deg(), 180.0, 1e-9), "az={}", h.lon_deg());
    }

    #[test]
    fn northern_meridian_transit_is_due_north() {
        // Dec 60, observer latitude 45, on meridian: north of zenith, azimuth 0.
        let obs = Observer::from_degrees(45.0, 0.0);
        let lst = 2.0;
        let c = SkyCoord::<Icrs>::from_radians(lst, 60.0 * DEG);
        let h = icrs_to_altaz(c, obs, lst);
        assert!((h.lat_deg() - 75.0).abs() < 1e-9, "alt={}", h.lat_deg());
        assert!(close_deg(h.lon_deg(), 0.0, 1e-9), "az={}", h.lon_deg());
    }

    use tpt_planetary_core::constants::WGS84_SEMI_MAJOR_AXIS;

    #[test]
    fn epoch_ecliptic_at_j2000_matches_mean_ecliptic() {
        let c = SkyCoord::<Icrs>::from_degrees(47.3, -12.8);
        let a = icrs_to_ecliptic_at_epoch(c, crate::time::J2000_JD, 0.0);
        let b = c.transform_to::<MeanEcliptic>();
        assert!(close_deg(a.lon_deg(), b.lon_deg(), 1e-9));
        assert!((a.lat_deg() - b.lat_deg()).abs() < 1e-9);
    }

    #[test]
    fn epoch_ecliptic_moves_with_obliquity() {
        // One century later the obliquity is about 46.84 arcsec smaller, so the
        // north ecliptic pole moves: the ICRS north pole's ecliptic latitude
        // changes by the same amount.
        let npole = SkyCoord::<Icrs>::from_degrees(0.0, 90.0);
        let later = icrs_to_ecliptic_at_epoch(npole, crate::time::J2000_JD + 36_525.0, 0.0);
        let j2000 = npole.transform_to::<MeanEcliptic>();
        let shift_arcsec = (later.lat_deg() - j2000.lat_deg()).abs() * 3600.0;
        assert!(
            (shift_arcsec - 46.836_769).abs() < 0.01,
            "shift={shift_arcsec}"
        );
    }

    #[test]
    fn observer_on_equator_at_lon_zero_maps_to_semi_major_axis() {
        let [x, y, z] = geodetic_to_itrs(0.0, 0.0, 0.0);
        assert!((x - WGS84_SEMI_MAJOR_AXIS).abs() < 1e-6);
        assert!(y.abs() < 1e-6 && z.abs() < 1e-6);
    }

    #[test]
    fn observer_at_pole_maps_to_semi_minor_axis() {
        let [x, y, z] = geodetic_to_itrs(0.0, PI / 2.0, 0.0);
        assert!(x.abs() < 1e-6 && y.abs() < 1e-6);
        assert!((z - WGS84.semi_minor_axis()).abs() < 1e-6);
    }

    #[test]
    fn polar_motion_is_identity_at_zero_and_orthogonal() {
        let w = polar_motion_matrix(0.0, 0.0);
        for (i, row) in w.iter().enumerate() {
            for (j, cell) in row.iter().enumerate() {
                let expect = if i == j { 1.0 } else { 0.0 };
                assert!((cell - expect).abs() < 1e-15);
            }
        }
        // A non-zero pole gives an orthogonal matrix with unit determinant.
        let w = polar_motion_matrix(3.0e-7, -5.0e-7);
        for i in 0..3 {
            for j in 0..3 {
                let dot: f64 = (0..3).map(|k| w[i][k] * w[j][k]).sum();
                let expect = if i == j { 1.0 } else { 0.0 };
                assert!((dot - expect).abs() < 1e-15);
            }
        }
    }

    #[test]
    fn polar_motion_maps_cip_to_tirs_z_axis() {
        // With the convention in polar_motion_matrix, the CIP unit vector in
        // ITRS is W^T z, which is (sin xp, -sin yp cos xp, cos xp cos yp).
        let (xp, yp) = (2.0e-6_f64, 3.0e-6_f64);
        let u = [xp.sin(), -yp.sin() * xp.cos(), xp.cos() * yp.cos()];
        let z = itrs_to_tirs(u, xp, yp);
        assert!(z[0].abs() < 1e-15 && z[1].abs() < 1e-15 && (z[2] - 1.0).abs() < 1e-15);
        // To first order the CIP is at (xp, -yp, 1).
        assert!((u[0] - xp).abs() < 1e-11 && (u[1] + yp).abs() < 1e-11);
    }

    #[test]
    fn local_sidereal_time_adds_east_longitude() {
        let obs = Observer::from_degrees(0.0, 90.0);
        let lst = local_sidereal_time(0.5, obs);
        assert!((lst - (0.5 + PI / 2.0)).abs() < 1e-12);
    }
}
