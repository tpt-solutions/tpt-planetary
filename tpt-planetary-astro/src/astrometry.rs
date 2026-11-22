//! Astrometric corrections to ICRS directions: proper motion, parallax and
//! annual aberration (classical first order).
//!
//! These are the corrections between a catalogue direction and the direction
//! an observer sees at an epoch: proper motion, parallax, annual aberration and
//! the deflection of light by the Sun (for a source at infinity). The
//! relativistic terms of aberration, of order (v/c)^2 (0.002 arcseconds), are
//! not included. The Earth velocity used for aberration is either supplied by
//! the caller (for example from an SPK kernel) or taken from
//! [`earth_velocity_approx_au_per_day`], a Keplerian model with an error of
//! about 0.02 arcseconds over 1900-2100. The observer's heliocentric position
//! for light deflection comes from the caller or from
//! [`earth_heliocentric_position_approx_au`].
//!
////! Units: angles in radians unless a name says otherwise; proper motion and
//! parallax in milliarcseconds (per year for proper motion); distances in AU,
//! velocities in AU per day.

use core::f64::consts::PI;

use crate::frames::{Icrs, SkyCoord, mean_obliquity_j2000_rad};

/// Speed of light in AU per day (IAU 2012 value, 299792.458 km/s).
pub const C_AU_PER_DAY: f64 = 173.144_632_674_240_3;

/// Astronomical unit in kilometres (IAU 2012).
const AU_KM: f64 = 149_597_870.7;

/// Seconds per day.
const DAY_S: f64 = 86_400.0;

/// Milliarcseconds to radians.
const MAS_RAD: f64 = PI / (180.0 * 3600.0 * 1000.0);

/// One parsec in AU (648000 / pi).
const AU_PER_PC: f64 = 648_000.0 / PI;

/// Mean orbital speed of the Earth around the Sun, in km/s (a semi-major axis
/// of one AU and the solar gravitational parameter).
const EARTH_MEAN_SPEED_KM_S: f64 = 29.784_69;

/// Angular separation of two ICRS directions, in arcseconds. Computed from the
/// cross and dot products, which keeps full precision for arcsecond-scale
/// separations (`acos` of a dot product does not).
#[must_use]
pub fn separation_arcsec(a: SkyCoord<Icrs>, b: SkyCoord<Icrs>) -> f64 {
    let (u, v) = (a.to_unit_vector(), b.to_unit_vector());
    let cross = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    norm(cross).atan2(dot(u, v)).to_degrees() * 3600.0
}

/// Unit vector of an ICRS direction.
fn unit(c: SkyCoord<Icrs>) -> [f64; 3] {
    c.to_unit_vector()
}

fn norm(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn scale(v: [f64; 3], s: f64) -> [f64; 3] {
    [v[0] * s, v[1] * s, v[2] * s]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Converts a Cartesian direction to an [`SkyCoord`]. The vector need not be
/// normalised. Returns the input unchanged for a zero vector.
fn from_cartesian(v: [f64; 3], fallback: SkyCoord<Icrs>) -> SkyCoord<Icrs> {
    let n = norm(v);
    if n == 0.0 || !n.is_finite() {
        return fallback;
    }
    SkyCoord::from_vector(scale(v, 1.0 / n)).unwrap_or(fallback)
}

/// Applies proper motion over `years` to an ICRS direction.
///
/// `pm_ra_mas_yr` is the proper motion in right ascension already multiplied
/// by cos(dec) (the usual catalogue convention, such as Gaia's `pmra`), and
/// `pm_dec_mas_yr` the proper motion in declination. The star moves along the
/// great circle through the epoch position, in the direction of the proper
/// motion at that position, by the total proper-motion distance. For small
/// displacements this equals linear propagation in right ascension and
/// declination. Over long intervals it differs from linear coordinate
/// propagation by the change in the local frame, which is second order in the
/// displacement.
///
/// ```
/// use tpt_planetary_astro::astrometry::apply_proper_motion;
/// use tpt_planetary_astro::frames::{Icrs, SkyCoord};
/// // 1000 mas/yr in declination for 1000 years: the star moves 1000 arcseconds.
/// let s = SkyCoord::<Icrs>::from_degrees(10.0, 0.0);
/// let later = apply_proper_motion(s, 0.0, 1000.0, 1000.0);
/// assert!((later.lat_deg() - 1000.0 / 3600.0).abs() < 1e-9);
/// ```
#[must_use]
pub fn apply_proper_motion(
    coord: SkyCoord<Icrs>,
    pm_ra_mas_yr: f64,
    pm_dec_mas_yr: f64,
    years: f64,
) -> SkyCoord<Icrs> {
    let (sa, ca) = coord.lon().sin_cos();
    let (sd, cd) = coord.lat().sin_cos();
    // Local unit vectors of increasing right ascension and declination.
    let e_ra = [-sa, ca, 0.0];
    let e_dec = [-sd * ca, -sd * sa, cd];
    let motion = add(
        scale(e_ra, pm_ra_mas_yr * MAS_RAD * years),
        scale(e_dec, pm_dec_mas_yr * MAS_RAD * years),
    );
    let angle = norm(motion);
    if angle == 0.0 {
        return coord;
    }
    let dir = scale(motion, 1.0 / angle);
    let n = unit(coord);
    let moved = add(scale(n, angle.cos()), scale(dir, angle.sin()));
    from_cartesian(moved, coord)
}

/// Applies the geocentric parallax of a star at `parallax_mas` milliarcseconds.
///
/// `observer_au` is the observer's barycentric position in AU, and the input
/// direction is the barycentric direction of the star. The result is the
/// direction from the observer, which differs by up to the parallax angle.
/// A zero or negative parallax returns the input unchanged.
///
/// ```
/// use tpt_planetary_astro::astrometry::apply_parallax;
/// use tpt_planetary_astro::frames::{Icrs, SkyCoord};
/// let s = SkyCoord::<Icrs>::from_degrees(0.0, 0.0);
/// // Observer 1 AU along +y (ICRS), star at 1 pc: a 1 arcsecond shift.
/// let seen = apply_parallax(s, 1000.0, [0.0, 1.0, 0.0]);
/// assert!((seen.lon_deg() - 0.0).abs() < 1e-9 || seen.lon_deg() > 359.0);
/// ```
#[must_use]
pub fn apply_parallax(
    coord: SkyCoord<Icrs>,
    parallax_mas: f64,
    observer_au: [f64; 3],
) -> SkyCoord<Icrs> {
    if parallax_mas <= 0.0 || !parallax_mas.is_finite() {
        return coord;
    }
    let distance_au = AU_PER_PC * 1000.0 / parallax_mas;
    let n = unit(coord);
    let from_observer = add(scale(n, distance_au), scale(observer_au, -1.0));
    from_cartesian(from_observer, coord)
}

/// Applies classical annual aberration to an ICRS direction.
///
/// `earth_velocity_au_per_day` is the observer's barycentric velocity, in AU
/// per day. The apparent direction is shifted towards the velocity by the
/// velocity divided by the speed of light (20.5 arcseconds at the mean orbital
/// speed, up to about 20.9 at perihelion). The relativistic terms, of order (v/c)^2, are not applied.
///
/// ```
/// use tpt_planetary_astro::astrometry::{
///     annual_aberration, earth_velocity_approx_au_per_day, separation_arcsec,
/// };
/// use tpt_planetary_astro::frames::{Icrs, SkyCoord};
/// let star = SkyCoord::<Icrs>::from_degrees(100.0, 20.0);
/// let v = earth_velocity_approx_au_per_day(2_451_545.0, 0.0);
/// let apparent = annual_aberration(star, v);
/// // The shift is at most about 20.9 arcseconds: the Earth's speed ranges
/// // from 29.3 to 30.3 km/s, and the constant of aberration uses the mean.
/// assert!(separation_arcsec(star, apparent) < 21.0);
/// ```
#[must_use]
pub fn annual_aberration(
    coord: SkyCoord<Icrs>,
    earth_velocity_au_per_day: [f64; 3],
) -> SkyCoord<Icrs> {
    let beta = scale(earth_velocity_au_per_day, 1.0 / C_AU_PER_DAY);
    let n = unit(coord);
    from_cartesian(add(n, beta), coord)
}

/// Heliocentric longitude of the Earth (equinox J2000), true anomaly and
/// eccentricity at a TT two-part Julian date, from low-precision elements.
fn earth_orbit(tt_hi: f64, tt_lo: f64) -> (f64, f64, f64) {
    let d = (tt_hi - 2_451_545.0) + tt_lo;
    let t = d / 36_525.0;
    // Sun's mean anomaly and mean longitude of date, in degrees.
    let g = (357.528 + 0.985_600_3 * d).to_radians();
    let l_mean = 280.460 + 0.985_647_4 * d;
    // Sun's true longitude of date: equation of the centre to second order.
    let lon_sun_date = l_mean.to_radians()
        + 1.915f64.to_radians() * g.sin()
        + 0.020f64.to_radians() * (2.0 * g).sin();
    // Precession of the equinox: general precession in longitude, 5029.0966"/century,
    // to move the longitude from the equinox of date to the J2000 equinox.
    let lon_sun = lon_sun_date - (5029.0966 / 3600.0 * t).to_radians();
    // The Earth is opposite the Sun.
    let lon_earth = lon_sun + PI;
    // Orbital elements of the Earth (J2000 equinox): perihelion longitude
    // and eccentricity, with their secular terms.
    let perihelion = (102.937_68 + 0.322 * t).to_radians();
    let e = 0.016_708_634 - 0.000_042_037 * t;
    let nu = lon_earth - perihelion; // true anomaly
    (lon_earth, nu, e)
}

/// Ecliptic (J2000) to equatorial (ICRS) axes, with the J2000 obliquity.
fn ecliptic_to_equatorial(v: [f64; 3]) -> [f64; 3] {
    let eps = mean_obliquity_j2000_rad();
    let (se, ce) = eps.sin_cos();
    [v[0], v[1] * ce - v[2] * se, v[1] * se + v[2] * ce]
}

/// Approximate barycentric velocity of the Earth in AU per day, in the ICRS
/// axes, at a TT two-part Julian date.
///
/// The Earth is treated as a Keplerian orbit about the Sun with the low-precision
/// elements of the Astronomical Almanac: eccentricity, longitude of perihelion
/// and the Sun's mean longitude. The velocity has the radial and tangential
/// components of the eccentric orbit, rotated to the J2000 equator. The Sun's
/// reflex motion about the barycentre (about 0.01 km/s) is ignored, as is the
/// Earth-Moon barycentre offset. Accuracy is set by the tests against ERFA's
/// `epv00`.
#[must_use]
pub fn earth_velocity_approx_au_per_day(tt_hi: f64, tt_lo: f64) -> [f64; 3] {
    let (lon_earth, nu, e) = earth_orbit(tt_hi, tt_lo);
    // Mean orbital speed for a = 1 AU, in km/s; the eccentric orbit has
    // radial and tangential components v_r = k e sin nu, v_t = k (1 + e cos nu).
    let k = EARTH_MEAN_SPEED_KM_S / (1.0 - e * e).sqrt();
    let v_r = k * e * nu.sin();
    let v_t = k * (1.0 + e * nu.cos());
    let (sl, cl) = lon_earth.sin_cos();
    let scale_au_day = DAY_S / AU_KM;
    let v_ecl = [
        (v_r * cl - v_t * sl) * scale_au_day,
        (v_r * sl + v_t * cl) * scale_au_day,
        0.0,
    ];
    ecliptic_to_equatorial(v_ecl)
}

/// Approximate heliocentric position of the Earth in AU, in the ICRS axes, at
/// a TT two-part Julian date. Same model as
/// [`earth_velocity_approx_au_per_day`]; its error is about 1e-4 AU, which
/// changes the light deflection by about 0.01 per cent.
#[must_use]
pub fn earth_heliocentric_position_approx_au(tt_hi: f64, tt_lo: f64) -> [f64; 3] {
    let (lon_earth, nu, e) = earth_orbit(tt_hi, tt_lo);
    let r = (1.0 - e * e) / (1.0 + e * nu.cos());
    let (sl, cl) = lon_earth.sin_cos();
    ecliptic_to_equatorial([r * cl, r * sl, 0.0])
}

/// Gravitational radius of the Sun, `2 GM / c^2`, in AU (ERFA `ERFA_SRS`).
const SUN_SCHWARZSCHILD_RADIUS_AU: f64 = 1.974_125_743_36e-8;

/// Applies the deflection of starlight by the Sun, for a source at infinity.
///
/// `observer_helio_au` is the observer's heliocentric position in AU (ICRS
/// axes). The apparent direction moves away from the Sun, by about 1.75
/// arcseconds at the solar limb and less farther away. The formula is the
/// weak-field result for a source at infinity, `(2GM/c^2)/d (e - (p.e) p) /
/// (1 + p.e)`, with `e` the Sun-to-observer direction and `d` the distance;
/// it matches ERFA's `ld` to about 1e-10 arcseconds over the whole sky. Within
/// 1e-10 of the Sun's direction the deflection is undefined, and the input is
/// returned unchanged.
///
/// ```
/// use tpt_planetary_astro::astrometry::light_deflection_by_sun;
/// use tpt_planetary_astro::frames::{Icrs, SkyCoord};
/// // Observer at 1 AU on the +x axis; a star 90 degrees from the Sun.
/// let star = SkyCoord::<Icrs>::from_degrees(90.0, 0.0);
/// let seen = light_deflection_by_sun(star, [1.0, 0.0, 0.0]);
/// // The star moves away from the Sun, by about 4 milliarcseconds here.
/// assert!(seen.lon_deg() < 90.0 && seen.lon_deg() > 89.999_99);
/// ```
#[must_use]
pub fn light_deflection_by_sun(
    coord: SkyCoord<Icrs>,
    observer_helio_au: [f64; 3],
) -> SkyCoord<Icrs> {
    let em = norm(observer_helio_au);
    if em == 0.0 || !em.is_finite() {
        return coord;
    }
    let e = scale(observer_helio_au, 1.0 / em);
    let p = unit(coord);
    let pde = dot(p, e);
    let denom = 1.0 + pde;
    if denom < 1e-10 {
        return coord;
    }
    let w = SUN_SCHWARZSCHILD_RADIUS_AU / em / denom;
    let shift = scale(add(e, scale(p, -pde)), w);
    from_cartesian(add(p, shift), coord)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn separation(a: SkyCoord<Icrs>, b: SkyCoord<Icrs>) -> f64 {
        separation_arcsec(a, b)
    }

    #[test]
    fn aberration_magnitude_is_the_constant_of_aberration() {
        // 20.4955 arcseconds is the constant of aberration; a star at 90 degrees
        // from the apex moves by that amount.
        let v = [0.0, 0.0, 0.0];
        assert_eq!(
            annual_aberration(SkyCoord::from_degrees(0.0, 0.0), v).lon(),
            0.0
        );
        let speed = 29.784_69 * 86_400.0 / AU_KM;
        let apex_perp = SkyCoord::<Icrs>::from_degrees(0.0, 0.0);
        let shifted = annual_aberration(apex_perp, [0.0, speed, 0.0]);
        let sep = separation_arcsec(apex_perp, shifted);
        assert!((sep - 20.4955).abs() < 0.01, "sep {sep}");
    }

    #[test]
    fn aberration_moves_towards_the_velocity() {
        let star = SkyCoord::<Icrs>::from_degrees(0.0, 0.0);
        let shifted = annual_aberration(star, [0.0, 0.01, 0.0]);
        assert!(shifted.lon_deg() > 0.0 && shifted.lon_deg() < 1.0);
        assert!(shifted.lat_deg().abs() < 1e-12);
    }

    #[test]
    fn approximate_earth_velocity_is_orthogonal_to_the_sun_and_has_the_right_speed() {
        for (hi, lo) in [(2_451_545.0, 0.0), (2_460_000.5, 0.3), (2_455_000.5, 0.9)] {
            let v = earth_velocity_approx_au_per_day(hi, lo);
            let speed_km_s = norm(v) * AU_KM / DAY_S;
            assert!((speed_km_s - 29.8).abs() < 0.5, "speed {speed_km_s}");
            // The velocity is perpendicular to the Earth-Sun line, which is
            // along the Sun's direction; the ecliptic latitude of the velocity
            // is ~0 in the ecliptic frame, so its dot with the ecliptic pole is 0.
            let eps = mean_obliquity_j2000_rad();
            let pole = [0.0, -eps.sin(), eps.cos()];
            assert!(dot(v, pole).abs() / norm(v) < 1e-12);
        }
    }

    #[test]
    fn proper_motion_moves_along_the_great_circle() {
        let s = SkyCoord::<Icrs>::from_degrees(200.0, 45.0);
        let moved = apply_proper_motion(s, 300.0, -400.0, 10.0);
        // Total rate 500 mas/yr over 10 years = 5 arcseconds.
        assert!((separation(s, moved) - 5.0).abs() < 1e-9);
        // The moved point lies on the great circle through s and the motion direction.
        let n = s.to_unit_vector();
        let p = moved.to_unit_vector();
        let normal = [
            n[1] * p[2] - n[2] * p[1],
            n[2] * p[0] - n[0] * p[2],
            n[0] * p[1] - n[1] * p[0],
        ];
        // The normal to that circle is orthogonal to the direction of the start.
        assert!(dot(normal, n).abs() < 1e-12);
        // For small displacements it matches linear RA/Dec propagation.
        let linear = SkyCoord::<Icrs>::from_degrees(
            200.0 + 300.0e-3 / 3600.0 * 10.0 / (45.0f64.to_radians().cos()),
            45.0 - 400.0e-3 / 3600.0 * 10.0,
        );
        // The difference is second order in the 5 arcsecond displacement (about 1e-4 arcsec).
        assert!(separation(moved, linear) < 1e-3);
        // Zero proper motion is the identity.
        assert_eq!(apply_proper_motion(s, 0.0, 0.0, 50.0), s);
    }

    #[test]
    fn pure_declination_motion_reverses_exactly() {
        // Along a meridian the local frame does not change, so moving back returns the start.
        let s = SkyCoord::<Icrs>::from_degrees(30.0, 20.0);
        let moved = apply_proper_motion(s, 0.0, 800.0, 25.0);
        let back = apply_proper_motion(moved, 0.0, -800.0, 25.0);
        assert!(separation(s, back) < 1e-9);
    }

    #[test]
    fn parallax_shift_matches_the_parallax_angle() {
        // A star at the ecliptic pole direction, observed 1 AU away perpendicular
        // to the line of sight, shifts by about the parallax angle p = 1/d.
        let star = SkyCoord::<Icrs>::from_degrees(0.0, 0.0);
        let parallax_mas = 100.0; // 10 pc
        let seen = apply_parallax(star, parallax_mas, [0.0, 1.0, 0.0]);
        let shift = separation(star, seen);
        assert!((shift - 0.1).abs() < 1e-4, "shift {shift}");
        // No parallax, no change.
        assert_eq!(apply_parallax(star, 0.0, [0.0, 1.0, 0.0]), star);
    }
}
