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
//! - ICRS, CIRS, TIRS and ITRS, in all ordered pairs, with the epoch and polar
//!   motion supplied in a [`TransformContext`] ([`FrameTransformAt`]; the CIO
//!   chain of [`crate::cio`])
//! - ICRS -> geodetic Alt/Az on WGS84 at a given epoch ([`icrs_to_altaz_at`]);
//!   no refraction, aberration, light deflection or parallax
//!
//! Angles are stored in radians. Constructors and accessors in degrees are
//! provided for convenience.

use core::f64::consts::{PI, TAU};
use core::marker::PhantomData;

use tpt_planetary_core::angle::Radians;
use tpt_planetary_core::ellipsoid::{Geodetic, WGS84};

use crate::cio;
use crate::eop::{ARCSEC_TO_RAD, EopError, EopTable};
use crate::time;

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
pub fn icrs_to_ecliptic_epoch_obliquity(
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
/// `s'` is taken as zero. At 2026 `s'` is about -5.9e-11 rad, which moves the
/// matrix by the same amount (about 0.4 mm on the surface). Under this
/// convention the celestial intermediate pole sits at `(xp, -yp, 1)` in ITRS,
/// to first order.
///
/// Checked against `erfa.pom00` (pyerfa 2.0.1.5) with `s' = 0`: ERFA documents
/// its matrix as the TIRS -> ITRS direction, so this matrix equals the
/// transpose of `pom00`'s output. The two agree to double-precision rounding
/// (see `docs/validation.md`).
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

/// Celestial intermediate reference system (CIRS): right ascension measured
/// from the CIO along the equator of the CIP (IERS Conventions 2010, Section
/// 5.4.2). Used for directions only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cirs;

/// Terrestrial intermediate reference system (TIRS): the Earth-fixed frame
/// before polar motion, with the TIO on its x-axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tirs;

/// International Terrestrial Reference System (ITRS), reached from TIRS by
/// polar motion. Used for directions only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Itrs;

impl Frame for Cirs {
    const NAME: &'static str = "CIRS";
}
impl Frame for Tirs {
    const NAME: &'static str = "TIRS";
}
impl Frame for Itrs {
    const NAME: &'static str = "ITRS";
}

/// Epoch and Earth orientation inputs for the CIO-based transformations.
///
/// The TT and UT1 epochs are two-part Julian dates `(hi, lo)` as in
/// [`crate::time`]. The polar coordinates are the CIP's `xp` and `yp` in
/// radians. No EOP table is read: the caller supplies the values, and zero
/// polar motion is a valid choice when it is not known. The TIO locator is
/// the secular value of [`crate::cio::tio_locator`].
///
/// ```
/// use tpt_planetary_astro::frames::TransformContext;
/// let ctx = TransformContext::new((2_451_545.0, 0.0), (2_451_545.0, 0.0), 0.1, 0.2);
/// assert!((ctx.xp * 206_264.806_247_096_36 - 0.1).abs() < 1e-12);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformContext {
    /// TT Julian date, high part.
    pub tt_hi: f64,
    /// TT Julian date, low part.
    pub tt_lo: f64,
    /// UT1 Julian date, high part.
    pub ut1_hi: f64,
    /// UT1 Julian date, low part.
    pub ut1_lo: f64,
    /// CIP polar coordinate `xp`, radians.
    pub xp: f64,
    /// CIP polar coordinate `yp`, radians.
    pub yp: f64,
}

impl TransformContext {
    /// Creates a context from the TT and UT1 two-part Julian dates and the
    /// polar coordinates in arcseconds.
    #[must_use]
    pub fn new(tt: (f64, f64), ut1: (f64, f64), xp_arcsec: f64, yp_arcsec: f64) -> Self {
        Self {
            tt_hi: tt.0,
            tt_lo: tt.1,
            ut1_hi: ut1.0,
            ut1_lo: ut1.1,
            xp: xp_arcsec * ARCSEC_TO_RAD,
            yp: yp_arcsec * ARCSEC_TO_RAD,
        }
    }
}

/// Error building a [`TransformContext`] from a UTC epoch and an EOP table.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum ContextError {
    /// The epoch could not be converted to TT (outside the leap-second table).
    Time(tpt_planetary_core::Error),
    /// The EOP table does not cover the epoch, or its rows are unusable.
    Eop(EopError),
}

impl core::fmt::Display for ContextError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ContextError::Time(e) => write!(f, "cannot convert the epoch to TT: {e}"),
            ContextError::Eop(e) => write!(f, "Earth orientation data: {e}"),
        }
    }
}

impl From<EopError> for ContextError {
    fn from(e: EopError) -> Self {
        ContextError::Eop(e)
    }
}

impl TransformContext {
    /// Builds the context for a UTC epoch: TT from the leap-second table, UT1
    /// from `UT1 - UTC` in the EOP table, and the polar motion of the epoch.
    ///
    /// # Errors
    ///
    /// [`ContextError::Time`] if the epoch is outside the leap-second table, or
    /// [`ContextError::Eop`] if the EOP table has no row range covering it.
    pub fn from_utc_and_eop(
        utc: tpt_planetary_core::scale::Epoch<tpt_planetary_core::scale::Utc>,
        eop: &EopTable<'_>,
        leap: &tpt_planetary_core::leap::LeapSecondTable<'_>,
    ) -> Result<Self, ContextError> {
        use tpt_planetary_core::scale::{Epoch, Tt};

        let tt: Epoch<Tt> = utc.to_scale(leap).map_err(ContextError::Time)?;
        let utc_jd = utc.jd();
        let ut1 = eop.utc_to_ut1(utc_jd.day(), utc_jd.fraction())?;
        let mjd_utc = (utc_jd.day() - 2_400_000.5) + utc_jd.fraction();
        let (xp, yp) = eop.polar_motion_arcsec(mjd_utc)?;
        let tt_jd = tt.jd();
        Ok(TransformContext::new(
            (tt_jd.day(), tt_jd.fraction()),
            ut1,
            xp,
            yp,
        ))
    }
}

/// Transformation from frame `Self` to frame `To` that depends on the epoch
/// and Earth orientation, supplied as a [`TransformContext`].
pub trait FrameTransformAt<To: Frame>: Frame {
    /// Applies the transformation at the context's epoch.
    fn transform_at(coord: SkyCoord<Self>, ctx: &TransformContext) -> SkyCoord<To>;
}

impl<F: Frame> SkyCoord<F> {
    /// Transforms this coordinate into frame `T` at the epoch and polar motion
    /// of `ctx`.
    ///
    /// ```
    /// use tpt_planetary_astro::frames::{Cirs, Icrs, SkyCoord, TransformContext};
    /// let ctx = TransformContext::new((2_451_545.0, 0.0), (2_451_545.0, 0.0), 0.0, 0.0);
    /// let star = SkyCoord::<Icrs>::from_degrees(10.0, 20.0);
    /// let back = star.transform_at::<Cirs>(&ctx).transform_at::<Icrs>(&ctx);
    /// assert!((back.lon() - star.lon()).abs() < 1e-12);
    /// ```
    #[must_use]
    pub fn transform_at<T: Frame>(self, ctx: &TransformContext) -> SkyCoord<T>
    where
        F: FrameTransformAt<T>,
    {
        F::transform_at(self, ctx)
    }
}

/// Internal tag for the four CIO-chain frames, used to pick matrices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Icrs,
    Cirs,
    Tirs,
    Itrs,
}

/// Matrix from ICRS to `to` at the context. For `Itrs` this is
/// `W R Q`, which equals [`cio::icrs_to_itrs`] at the same epoch.
fn matrix_from_icrs(to: Kind, ctx: &TransformContext) -> Mat3 {
    if to == Kind::Icrs {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let q = cio::icrs_to_cirs(ctx.tt_hi, ctx.tt_lo);
    if to == Kind::Cirs {
        return q;
    }
    let era = time::earth_rotation_angle(ctx.ut1_hi, ctx.ut1_lo);
    let r = cio::cirs_to_tirs(era);
    if to == Kind::Tirs {
        return mat_mul(&r, &q);
    }
    let sp = cio::tio_locator(ctx.tt_hi, ctx.tt_lo);
    let w = cio::tirs_to_itrs(ctx.xp, ctx.yp, sp);
    mat_mul(&w, &mat_mul(&r, &q))
}

/// Matrix from `from` to `to`. Each `matrix_from_icrs` is orthogonal, so the
/// inverse of one is its transpose.
fn matrix_between(from: Kind, to: Kind, ctx: &TransformContext) -> Mat3 {
    mat_mul(
        &matrix_from_icrs(to, ctx),
        &transpose(&matrix_from_icrs(from, ctx)),
    )
}

/// Rotates the unit vector `v` from `from` to `to` and returns the direction.
fn rotate_direction<To: Frame>(
    from: Kind,
    to: Kind,
    v: [f64; 3],
    ctx: &TransformContext,
) -> SkyCoord<To> {
    let m = matrix_between(from, to, ctx);
    SkyCoord::from_vector(mat_vec(&m, v)).unwrap_or_else(|| SkyCoord::from_radians(0.0, 0.0))
}

macro_rules! impl_frame_transform_at {
    ($($from:ident => $to:ident),* $(,)?) => {
        $(
            impl FrameTransformAt<$to> for $from {
                fn transform_at(coord: SkyCoord<Self>, ctx: &TransformContext) -> SkyCoord<$to> {
                    rotate_direction::<$to>(
                        Kind::$from,
                        Kind::$to,
                        coord.to_unit_vector(),
                        ctx,
                    )
                }
            }
        )*
    };
}

impl_frame_transform_at!(
    Icrs => Cirs,
    Icrs => Tirs,
    Icrs => Itrs,
    Cirs => Icrs,
    Cirs => Tirs,
    Cirs => Itrs,
    Tirs => Icrs,
    Tirs => Cirs,
    Tirs => Itrs,
    Itrs => Icrs,
    Itrs => Cirs,
    Itrs => Tirs,
);

/// Geodetic observer on the WGS84 ellipsoid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeodeticObserver {
    /// Geodetic longitude in radians, east positive.
    pub lon_rad: f64,
    /// Geodetic latitude in radians, north positive.
    pub lat_rad: f64,
    /// Height above the WGS84 ellipsoid in metres.
    pub height_m: f64,
}

impl GeodeticObserver {
    /// Creates an observer from longitude and latitude in degrees and a height
    /// in metres.
    #[must_use]
    pub fn from_degrees(lon_deg: f64, lat_deg: f64, height_m: f64) -> Self {
        Self {
            lon_rad: lon_deg * DEG,
            lat_rad: lat_deg * DEG,
            height_m,
        }
    }

    /// Geocentric ITRS position of the observer in metres, from the WGS84
    /// closed form in `tpt_planetary_core`.
    #[must_use]
    pub fn itrs_position(&self) -> [f64; 3] {
        geodetic_to_itrs(self.lon_rad, self.lat_rad, self.height_m)
    }
}

/// Altitude and azimuth of an ICRS direction (a direction at infinity) for a
/// geodetic observer, at the epoch and polar motion of `ctx`.
///
/// The direction is rotated to ITRS with the CIO-based chain (IERS 2010 Eq.
/// 5.1, [`cio::icrs_to_itrs`]). Its components along the observer's
/// ellipsoid normal (up), east and north then give the altitude and the
/// azimuth measured from north through east.
///
/// Not applied: atmospheric refraction, annual and diurnal aberration, light
/// deflection, and parallax. The observer's height does not change the
/// result for a direction at infinity, because only the local vertical
/// direction enters. It is accepted so the observer can be described fully.
/// The local vertical is the WGS84 ellipsoid normal, the direction of
/// geodetic latitude.
///
/// ```
/// use tpt_planetary_astro::frames::{GeodeticObserver, Icrs, SkyCoord, TransformContext, icrs_to_altaz_at};
/// let ctx = TransformContext::new((2_451_545.0, 0.0), (2_451_545.0, 0.0), 0.0, 0.0);
/// // At the North Pole the altitude of a star is its declination.
/// let obs = GeodeticObserver::from_degrees(0.0, 90.0, 0.0);
/// let star = SkyCoord::<Icrs>::from_degrees(0.0, 30.0);
/// let altaz = icrs_to_altaz_at(star, obs, &ctx);
/// assert!((altaz.lat_deg() - 30.0).abs() < 0.01);
/// ```
#[must_use]
pub fn icrs_to_altaz_at(
    coord: SkyCoord<Icrs>,
    observer: GeodeticObserver,
    ctx: &TransformContext,
) -> SkyCoord<AltAz> {
    let d = mat_vec(&matrix_from_icrs(Kind::Itrs, ctx), coord.to_unit_vector());
    let (sl, cl) = observer.lon_rad.sin_cos();
    let (sp, cp) = observer.lat_rad.sin_cos();
    let east = [-sl, cl, 0.0];
    let north = [-sp * cl, -sp * sl, cp];
    let up = [cp * cl, cp * sl, sp];
    let alt = dot3(up, d).clamp(-1.0, 1.0).asin();
    let az = dot3(east, d).atan2(dot3(north, d));
    SkyCoord::from_radians(az, alt)
}

/// Ambient conditions at the observer, used for atmospheric refraction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Atmosphere {
    /// Pressure at the observer in hPa (millibar). Zero switches refraction off.
    pub pressure_hpa: f64,
    /// Ambient temperature in degrees Celsius.
    pub temperature_c: f64,
    /// Relative humidity, from 0 (dry) to 1 (saturated).
    pub relative_humidity: f64,
    /// Observing wavelength in micrometres. Values up to 100 select the
    /// optical/IR refractivity; larger values select the radio case.
    pub wavelength_um: f64,
}

impl Atmosphere {
    /// The constants `(A, B)` in radians of the refraction model
    /// `dZ = A tan Z + B tan^3 Z`; see [`refraction_coefficients`].
    #[must_use]
    pub fn refraction_coefficients(&self) -> (f64, f64) {
        refraction_coefficients(
            self.pressure_hpa,
            self.temperature_c,
            self.relative_humidity,
            self.wavelength_um,
        )
    }
}

/// Constants `(A, B)` of the atmospheric refraction model `dZ = A tan Z +
/// B tan^3 Z`, in radians, where `Z` is the observed zenith distance and `dZ`
/// is the amount to add to it to get the true (in vacuo) zenith distance.
///
/// This is the ERFA `eraRefco` model (Stone 1996 with the Hohenkerk and
/// Sinclair refractivity; the same constants as SOFA). Inputs are clamped to
/// the ranges that model accepts (temperature -150 to 100 C, pressure 0 to
/// 10000 hPa, humidity 0 to 1, wavelength 0.1 um to 1e6 um). Zero pressure
/// gives `(0, 0)`.
///
/// Against `erfa.refco` (pyerfa 2.0.1.5) the optical/IR constants agree to
/// about 2e-16 relative and the radio constants to about 4e-15 over a grid of
/// pressures, temperatures, humidities and wavelengths.
#[must_use]
pub fn refraction_coefficients(
    pressure_hpa: f64,
    temperature_c: f64,
    relative_humidity: f64,
    wavelength_um: f64,
) -> (f64, f64) {
    let optic = wavelength_um <= 100.0;
    let t = temperature_c.clamp(-150.0, 100.0);
    let p = pressure_hpa.clamp(0.0, 10_000.0);
    let r = relative_humidity.clamp(0.0, 1.0);
    let w = wavelength_um.clamp(0.1, 1.0e6);
    // Partial pressure of water vapour, hPa.
    let pw = if p > 0.0 {
        let ps = 10f64.powf((0.7859 + 0.03477 * t) / (1.0 + 0.00412 * t))
            * (1.0 + p * (4.5e-6 + 6e-10 * t * t));
        r * ps / (1.0 - (1.0 - r) * ps / p)
    } else {
        0.0
    };
    let tk = t + 273.15;
    let gamma = if optic {
        let w2 = w * w;
        (77.534_84e-6 + (4.391_08e-7 + 3.666e-9 / w2) / w2) * p / tk - 11.2684e-6 * pw / tk
    } else {
        77.6890e-6 * p / tk - (6.3938e-6 - 0.375_463 / tk) * pw / tk
    };
    let mut beta = 4.4474e-6 * tk;
    if !optic {
        beta -= 0.0074 * pw * beta;
    }
    let a = gamma * (1.0 - beta);
    let b = -gamma * (beta - gamma / 2.0);
    (a, b)
}

/// Observed zenith distance for a true zenith distance `z_true`, given the
/// refraction constants `(a, b)`: solves `Z = z_true - (a tan Z + b tan^3 Z)`
/// by fixed-point iteration. The iteration contracts by about `a / cos^2 Z`
/// per step, so it converges for the zenith distances in use here.
fn refract_zenith(z_true: f64, a: f64, b: f64) -> f64 {
    let mut z = z_true;
    for _ in 0..50 {
        let t = z.tan();
        let next = z_true - (a * t + b * t * t * t);
        let done = (next - z).abs() < 1e-15;
        z = next;
        if done {
            break;
        }
    }
    z
}

/// Altitude and azimuth of an ICRS direction for a geodetic observer, with
/// atmospheric refraction applied to the geometric altitude.
///
/// The geometric result is [`icrs_to_altaz_at`]; the refraction model is
/// [`refraction_coefficients`]. The observed zenith distance is found from the
/// true one by fixed-point iteration, and the azimuth is unchanged.
///
/// Accuracy: the constants match ERFA, and against ERFA's own refraction
/// inside `erfa.atco13` (same geometric zenith distance, pressure 1013.25 hPa,
/// 15 C, 50 % humidity, 0.55 um) the refracted zenith distance agrees to
/// 0.003 arcsec for true zenith distances up to 75 degrees. At 82 degrees the
/// two differ by 0.04 arcsec, because ERFA inverts the same model by a
/// different expansion. The model itself is an approximation of a
/// ray-traced atmosphere, with errors of tens of milliarcseconds at 80
/// degrees.
///
/// The refraction is applied for true zenith distances below 85 degrees.
/// Above that, and below the horizon, the geometric position is returned,
/// because the model diverges there.
#[must_use]
pub fn icrs_to_altaz_refracted_at(
    coord: SkyCoord<Icrs>,
    observer: GeodeticObserver,
    ctx: &TransformContext,
    atmosphere: Atmosphere,
) -> SkyCoord<AltAz> {
    let geometric = icrs_to_altaz_at(coord, observer, ctx);
    if atmosphere.pressure_hpa.is_nan() || atmosphere.pressure_hpa <= 0.0 {
        return geometric;
    }
    let z_true = PI / 2.0 - geometric.lat();
    if z_true.is_nan() || z_true >= 85.0 * DEG {
        return geometric;
    }
    let (a, b) = atmosphere.refraction_coefficients();
    let z_obs = refract_zenith(z_true, a, b);
    SkyCoord::from_radians(geometric.lon(), PI / 2.0 - z_obs)
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
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
        let a = icrs_to_ecliptic_epoch_obliquity(c, crate::time::J2000_JD, 0.0);
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
        let later = icrs_to_ecliptic_epoch_obliquity(npole, crate::time::J2000_JD + 36_525.0, 0.0);
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

    fn test_context() -> TransformContext {
        TransformContext::new((2_460_000.5, 0.25), (2_460_000.5, 0.2499), 0.1, 0.25)
    }

    fn vec_gap(a: [f64; 3], b: [f64; 3]) -> f64 {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    }

    fn angle_gap(a: f64, b: f64) -> f64 {
        let d = (a - b).rem_euclid(TAU);
        d.min(TAU - d)
    }

    #[test]
    fn cirs_tirs_itrs_round_trip_is_identity() {
        let c = test_context();
        for (ra, dec) in [(0.0, 0.0), (123.4, -45.6), (300.0, 80.0), (250.0, -89.0)] {
            let star = SkyCoord::<Icrs>::from_degrees(ra, dec);
            let back = star
                .transform_at::<Cirs>(&c)
                .transform_at::<Tirs>(&c)
                .transform_at::<Itrs>(&c)
                .transform_at::<Tirs>(&c)
                .transform_at::<Cirs>(&c)
                .transform_at::<Icrs>(&c);
            assert!(vec_gap(star.to_unit_vector(), back.to_unit_vector()) < 1e-13);
            let direct = star.transform_at::<Itrs>(&c).transform_at::<Icrs>(&c);
            assert!(vec_gap(star.to_unit_vector(), direct.to_unit_vector()) < 1e-13);
        }
    }

    #[test]
    fn chain_composition_equals_icrs_to_itrs() {
        let c = test_context();
        let m = cio::icrs_to_itrs(c.tt_hi, c.tt_lo, c.ut1_hi, c.ut1_lo, c.xp, c.yp);
        for (ra, dec) in [(0.0, 0.0), (45.0, 10.0), (200.0, -70.0)] {
            let star = SkyCoord::<Icrs>::from_degrees(ra, dec);
            let v = star.to_unit_vector();
            let via_chain = star
                .transform_at::<Cirs>(&c)
                .transform_at::<Tirs>(&c)
                .transform_at::<Itrs>(&c)
                .to_unit_vector();
            assert!(vec_gap(mat_vec(&m, v), via_chain) < 1e-13);
        }
    }

    #[test]
    fn polar_motion_only_enters_the_tirs_to_itrs_step() {
        let c = test_context();
        let zero = TransformContext {
            xp: 0.0,
            yp: 0.0,
            ..c
        };
        let star = SkyCoord::<Icrs>::from_degrees(77.0, 12.0);
        let tirs = star.transform_at::<Tirs>(&c);
        assert!(
            vec_gap(
                tirs.to_unit_vector(),
                star.transform_at::<Tirs>(&zero).to_unit_vector()
            ) < 1e-15
        );
        let moved = star.transform_at::<Itrs>(&c);
        let unmoved = star.transform_at::<Itrs>(&zero);
        assert!(vec_gap(moved.to_unit_vector(), unmoved.to_unit_vector()) > 1e-8);
    }

    #[test]
    fn star_at_zenith_has_altitude_90() {
        let c = test_context();
        let obs = GeodeticObserver::from_degrees(-70.0, -30.0, 2000.0);
        let (sl, cl) = obs.lon_rad.sin_cos();
        let (sp, cp) = obs.lat_rad.sin_cos();
        let up = [cp * cl, cp * sl, sp];
        let m = matrix_from_icrs(Kind::Itrs, &c);
        let icrs_dir = mat_vec(&transpose(&m), up);
        let star = SkyCoord::<Icrs>::from_vector(icrs_dir).expect("nonzero");
        let altaz = icrs_to_altaz_at(star, obs, &c);
        assert!(
            (altaz.lat() - PI / 2.0).abs() < 1e-12,
            "alt={}",
            altaz.lat()
        );
    }

    #[test]
    fn meridian_transit_altitude_and_azimuth() {
        // At transit the star is on the observer's meridian: its altitude is
        // 90 deg - |lat - dec|, and it lies due north when dec > lat, due
        // south when dec < lat.
        let c = test_context();
        let obs = GeodeticObserver::from_degrees(-70.0, -30.0, 2000.0);
        let m = matrix_from_icrs(Kind::Itrs, &c);
        let (sl, cl) = obs.lon_rad.sin_cos();
        for dec_deg in [-80.0_f64, -50.0, -10.0, 15.0, 60.0] {
            let dec = dec_deg * DEG;
            let (sd, cd) = dec.sin_cos();
            // ITRS direction with longitude equal to the observer's: transit.
            let d_itrs = [cd * cl, cd * sl, sd];
            let icrs_dir = mat_vec(&transpose(&m), d_itrs);
            let star = SkyCoord::<Icrs>::from_vector(icrs_dir).expect("nonzero");
            let altaz = icrs_to_altaz_at(star, obs, &c);
            let expect_alt = PI / 2.0 - (obs.lat_rad - dec).abs();
            assert!(
                (altaz.lat() - expect_alt).abs() < 1e-12,
                "dec={dec_deg}: alt={} expected {expect_alt}",
                altaz.lat()
            );
            let expect_az = if dec > obs.lat_rad { 0.0 } else { PI };
            assert!(angle_gap(altaz.lon(), expect_az) < 1e-10, "dec={dec_deg}");
        }
    }

    #[test]
    fn geodetic_observer_itrs_position_is_on_the_ellipsoid() {
        let obs = GeodeticObserver::from_degrees(0.0, 0.0, 0.0);
        let p = obs.itrs_position();
        assert!((p[0] - WGS84.semi_major_axis()).abs() < 1e-6);
    }

    #[test]
    fn refraction_constants_match_erfa_refco() {
        // Reference values from erfa.refco (pyerfa 2.0.1.5).
        let (a, b) = refraction_coefficients(1013.25, 15.0, 0.5, 0.55);
        assert!((a - 0.000_277_195_406_842_925_06).abs() < 1e-16);
        assert!((b + 3.171_695_036_812_495_7e-7).abs() < 1e-19);
        let (a, b) = refraction_coefficients(800.0, -5.0, 1.0, 0.7);
        assert!((a - 0.000_233_580_201_323_805_69).abs() < 1e-16);
        assert!((b + 2.515_483_750_111_349e-7).abs() < 1e-19);
        // Radio case (wavelength above 100 um) and zero pressure.
        let (a, b) = refraction_coefficients(1013.25, 15.0, 1.0, 500.0);
        assert!((a - 0.000_349_809_457_216_720_8).abs() < 1e-16);
        assert!((b + 3.306_280_035_631_262e-7).abs() < 1e-19);
        assert_eq!(refraction_coefficients(0.0, 15.0, 0.5, 0.55), (0.0, 0.0));
    }

    #[test]
    fn refracted_zenith_distance_matches_erfa_atco13() {
        // Pairs of (true, observed) zenith distance from erfa.atco13 with
        // pressure 1013.25 hPa, 15 C, humidity 0.5, wavelength 0.55 um.
        let (a, b) = refraction_coefficients(1013.25, 15.0, 0.5, 0.55);
        let cases = [
            (1.429_280_087_844_303_f64, 1.427_465_121_309_391_6_f64), // 81.9 deg
            (1.313_388_954_891_147_5, 1.312_357_549_068_745_4),       // 75.3 deg
            (1.142_944_881_793_565_7, 1.142_341_328_163_741_1),       // 65.5 deg
            (0.979_870_208_247_958_6, 0.979_458_452_618_181_1),       // 56.1 deg
        ];
        let arcsec = PI / 180.0 / 3600.0;
        for (z_true, z_erfa) in cases {
            let z_obs = refract_zenith(z_true, a, b);
            let diff = (z_obs - z_erfa).abs() / arcsec;
            // Tight at moderate zenith distances; ERFA inverts the model with a
            // different expansion, so the largest case allows 0.05 arcsec.
            let tol = if z_true > 1.4 { 0.05 } else { 0.005 };
            assert!(diff < tol, "z_true {z_true}: {diff} arcsec from ERFA");
        }
    }

    #[test]
    fn refraction_lifts_objects_and_leaves_zenith_and_azimuth_alone() {
        let ctx = TransformContext::new((2_451_545.0, 0.0), (2_451_545.0, 0.0), 0.0, 0.0);
        let obs = GeodeticObserver::from_degrees(0.0, 45.0, 0.0);
        let star = SkyCoord::<Icrs>::from_degrees(30.0, 60.0);
        let dry = Atmosphere {
            pressure_hpa: 0.0,
            temperature_c: 15.0,
            relative_humidity: 0.5,
            wavelength_um: 0.55,
        };
        let air = Atmosphere {
            pressure_hpa: 1013.25,
            ..dry
        };
        let geometric = icrs_to_altaz_at(star, obs, &ctx);
        assert_eq!(icrs_to_altaz_refracted_at(star, obs, &ctx, dry), geometric);
        let refracted = icrs_to_altaz_refracted_at(star, obs, &ctx, air);
        // Refraction raises the altitude by about A tan Z (about a minute of arc here).
        let lift = refracted.lat() - geometric.lat();
        let z = PI / 2.0 - geometric.lat();
        let (a, _) = air.refraction_coefficients();
        assert!(lift > 0.0 && (lift - a * z.tan()).abs() < 0.02 * lift);
        assert!(angle_gap(refracted.lon(), geometric.lon()) < 1e-15);

        // Below the horizon the geometric position is returned unchanged.
        let low = SkyCoord::<Icrs>::from_degrees(0.0, -60.0);
        assert_eq!(
            icrs_to_altaz_refracted_at(low, obs, &ctx, air),
            icrs_to_altaz_at(low, obs, &ctx)
        );
    }
}
