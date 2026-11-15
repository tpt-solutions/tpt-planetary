//! Spherical trigonometry on the unit sphere.
//!
//! Positions are [`SphericalCoord`] values (longitude and latitude, both in
//! [`Radians`]). Longitude is measured east from the x-axis and wrapped to
//! `[0, 2 pi)`; latitude is in `[-pi/2, pi/2]`.
//!
//! Angular separation uses the Vincenty formula, which is accurate for all
//! separations including nearly coincident and nearly antipodal points.
//!
//! ```
//! use tpt_planetary_core::angle::Degrees;
//! use tpt_planetary_core::sphere::{angular_separation, SphericalCoord};
//!
//! let a = SphericalCoord::new(Degrees(0.0).into(), Degrees(0.0).into());
//! let b = SphericalCoord::new(Degrees(0.0).into(), Degrees(90.0).into());
//! assert!((angular_separation(a, b).to_degrees().0 - 90.0).abs() < 1e-12);
//! ```

use glam::f64::DVec3;

use crate::angle::Radians;
use crate::error::Error;
use crate::math;

/// A point on the unit sphere as longitude and latitude.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct SphericalCoord {
    /// Longitude (east positive).
    pub longitude: Radians,
    /// Latitude (north positive), in `[-pi/2, pi/2]`.
    pub latitude: Radians,
}

impl SphericalCoord {
    /// Builds a point from longitude and latitude. No range check is applied.
    pub const fn new(longitude: Radians, latitude: Radians) -> Self {
        SphericalCoord {
            longitude,
            latitude,
        }
    }

    /// Returns the Cartesian unit vector `(cos lat cos lon, cos lat sin lon, sin lat)`.
    ///
    /// ```
    /// # use tpt_planetary_core::angle::Radians;
    /// # use tpt_planetary_core::sphere::SphericalCoord;
    /// let v = SphericalCoord::new(Radians(0.0), Radians(0.0)).to_cartesian();
    /// assert_eq!((v.x, v.y, v.z), (1.0, 0.0, 0.0));
    /// ```
    pub fn to_cartesian(self) -> DVec3 {
        let (sl, cl) = math::sin_cos(self.longitude.0);
        let (sb, cb) = math::sin_cos(self.latitude.0);
        DVec3::new(cb * cl, cb * sl, sb)
    }

    /// Converts a Cartesian vector to longitude and latitude.
    ///
    /// The vector does not need to be normalised. The longitude is wrapped to
    /// `[0, 2 pi)`.
    ///
    /// # Errors
    ///
    /// [`Error::ZeroVector`] if `v` is the zero vector.
    ///
    /// ```
    /// # use glam::f64::DVec3;
    /// # use tpt_planetary_core::sphere::SphericalCoord;
    /// let p = SphericalCoord::from_cartesian(DVec3::new(0.0, 0.0, 5.0))?;
    /// assert!((p.latitude.0 - core::f64::consts::FRAC_PI_2).abs() < 1e-15);
    /// # Ok::<(), tpt_planetary_core::Error>(())
    /// ```
    pub fn from_cartesian(v: DVec3) -> Result<Self, Error> {
        if v == DVec3::ZERO || !v.is_finite() {
            return Err(Error::ZeroVector);
        }
        let lon = math::atan2(v.y, v.x);
        let lat = math::atan2(v.z, math::hypot(v.x, v.y));
        Ok(SphericalCoord {
            longitude: Radians(lon).wrap_two_pi(),
            latitude: Radians(lat),
        })
    }
}

/// Angular separation between two points, by the Vincenty formula.
///
/// The formula is `atan2(|num|, den)` with
/// `num = sqrt((cos b2 sin dl)^2 + (cos b1 sin b2 - sin b1 cos b2 cos dl)^2)`
/// and `den = sin b1 sin b2 + cos b1 cos b2 cos dl`. It remains accurate for
/// very small and near-180-degree separations, where the plain spherical law of
/// cosines loses precision. The result is in `[0, pi]`.
///
/// ```
/// use tpt_planetary_core::angle::Radians;
/// use tpt_planetary_core::sphere::{angular_separation, SphericalCoord};
///
/// let a = SphericalCoord::new(Radians(0.0), Radians(0.0));
/// let b = SphericalCoord::new(Radians(core::f64::consts::PI), Radians(0.0));
/// assert!((angular_separation(a, b).0 - core::f64::consts::PI).abs() < 1e-15);
/// ```
pub fn angular_separation(a: SphericalCoord, b: SphericalCoord) -> Radians {
    let (s1, c1) = a.latitude.sin_cos();
    let (s2, c2) = b.latitude.sin_cos();
    let (sd, cd) = Radians(b.longitude.0 - a.longitude.0).sin_cos();
    let num = math::hypot(c2 * sd, c1 * s2 - s1 * c2 * cd);
    let den = s1 * s2 + c1 * c2 * cd;
    Radians(math::atan2(num, den))
}

/// Position angle of `to` as seen from `from`, measured from north through east.
///
/// This is the initial bearing of the great circle from `from` to `to`. The
/// result is wrapped to `[0, 2 pi)`. It is undefined for coincident points; the
/// function then returns `0`.
///
/// ```
/// use tpt_planetary_core::angle::Radians;
/// use tpt_planetary_core::sphere::{position_angle, SphericalCoord};
///
/// let origin = SphericalCoord::new(Radians(0.0), Radians(0.0));
/// let east = SphericalCoord::new(Radians(0.1), Radians(0.0));
/// assert!((position_angle(origin, east).0 - core::f64::consts::FRAC_PI_2).abs() < 1e-12);
/// ```
pub fn position_angle(from: SphericalCoord, to: SphericalCoord) -> Radians {
    let (s1, c1) = from.latitude.sin_cos();
    let (s2, c2) = to.latitude.sin_cos();
    let (sd, cd) = Radians(to.longitude.0 - from.longitude.0).sin_cos();
    let y = sd * c2;
    let x = c1 * s2 - s1 * c2 * cd;
    Radians(math::atan2(y, x)).wrap_two_pi()
}

/// Destination reached by travelling `distance` along the great circle that
/// leaves `from` on the given initial `bearing` (north through east).
///
/// ```
/// use tpt_planetary_core::angle::Radians;
/// use tpt_planetary_core::sphere::{destination, SphericalCoord};
///
/// let start = SphericalCoord::new(Radians(0.0), Radians(0.0));
/// let end = destination(start, Radians(0.0), Radians(0.5));
/// assert!((end.latitude.0 - 0.5).abs() < 1e-12);
/// assert!(end.longitude.0.abs() < 1e-12);
/// ```
pub fn destination(from: SphericalCoord, bearing: Radians, distance: Radians) -> SphericalCoord {
    let (s1, c1) = from.latitude.sin_cos();
    let (sd, cd) = distance.sin_cos();
    let (sb, cb) = bearing.sin_cos();
    let sin_lat2 = (s1 * cd + c1 * sd * cb).clamp(-1.0, 1.0);
    let lat2 = math::atan2(sin_lat2, math::sqrt((1.0 - sin_lat2) * (1.0 + sin_lat2)));
    let lon2 = from.longitude.0 + math::atan2(sb * sd * c1, cd - s1 * sin_lat2);
    SphericalCoord {
        longitude: Radians(lon2).wrap_two_pi(),
        latitude: Radians(lat2),
    }
}

/// Point a fraction `f` of the way along the great circle from `a` to `b`.
///
/// `f = 0` returns `a` and `f = 1` returns `b`; values outside `[0, 1]`
/// extrapolate along the same circle.
///
/// # Errors
///
/// [`Error::Antipodal`] if `a` and `b` are antipodal, where the great circle
/// is not unique.
///
/// ```
/// use tpt_planetary_core::angle::Radians;
/// use tpt_planetary_core::sphere::{great_circle_interpolate, SphericalCoord};
///
/// let a = SphericalCoord::new(Radians(0.0), Radians(0.0));
/// let b = SphericalCoord::new(Radians(core::f64::consts::FRAC_PI_2), Radians(0.0));
/// let mid = great_circle_interpolate(a, b, 0.5)?;
/// assert!((mid.longitude.0 - core::f64::consts::FRAC_PI_4).abs() < 1e-12);
/// # Ok::<(), tpt_planetary_core::Error>(())
/// ```
pub fn great_circle_interpolate(
    a: SphericalCoord,
    b: SphericalCoord,
    f: f64,
) -> Result<SphericalCoord, Error> {
    let u = a.to_cartesian();
    let v = b.to_cartesian();
    let cos_d = u.dot(v);
    let sin_d = u.cross(v).length();
    if sin_d < 1e-15 {
        return if cos_d > 0.0 {
            Ok(a)
        } else {
            Err(Error::Antipodal)
        };
    }
    let d = math::atan2(sin_d, cos_d);
    let wa = math::sin((1.0 - f) * d) / sin_d;
    let wb = math::sin(f * d) / sin_d;
    SphericalCoord::from_cartesian(u * wa + v * wb)
}

/// Signed angular distance of `p` from the great circle through `a` and `b`.
///
/// The result is positive when `p` lies to the left of the direction of
/// travel from `a` to `b`, as seen from outside the sphere, and negative to
/// the right. It is in `[-pi/2, pi/2]`.
///
/// # Errors
///
/// [`Error::ZeroVector`] if `a` and `b` coincide or are antipodal, because the
/// great circle is then undefined.
///
/// ```
/// use tpt_planetary_core::angle::Radians;
/// use tpt_planetary_core::sphere::{cross_track_angle, SphericalCoord};
///
/// let a = SphericalCoord::new(Radians(0.0), Radians(0.0));
/// let b = SphericalCoord::new(Radians(core::f64::consts::FRAC_PI_2), Radians(0.0));
/// // A point due north of the equator is to the left of travel eastwards.
/// let north = SphericalCoord::new(Radians(0.5), Radians(0.2));
/// assert!(cross_track_angle(north, a, b)?.0 > 0.0);
/// # Ok::<(), tpt_planetary_core::Error>(())
/// ```
pub fn cross_track_angle(
    p: SphericalCoord,
    a: SphericalCoord,
    b: SphericalCoord,
) -> Result<Radians, Error> {
    let n = a.to_cartesian().cross(b.to_cartesian());
    if n.length() < 1e-15 {
        return Err(Error::ZeroVector);
    }
    let n = n.normalize();
    let pv = p.to_cartesian();
    let d = pv.dot(n);
    let perp = (pv - n * d).length();
    Ok(Radians(math::atan2(d, perp)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};

    fn p(lon_deg: f64, lat_deg: f64) -> SphericalCoord {
        SphericalCoord::new(
            crate::angle::Degrees(lon_deg).to_radians(),
            crate::angle::Degrees(lat_deg).to_radians(),
        )
    }

    #[test]
    fn separation_golden_values() {
        // Equator to pole is exactly 90 degrees.
        assert!((angular_separation(p(0.0, 0.0), p(0.0, 90.0)).0 - FRAC_PI_2).abs() < 1e-15);
        // Two points 90 degrees apart along the equator.
        assert!((angular_separation(p(0.0, 0.0), p(90.0, 0.0)).0 - FRAC_PI_2).abs() < 1e-15);
        // Antipodes are exactly pi apart.
        assert!((angular_separation(p(0.0, 0.0), p(180.0, 0.0)).0 - PI).abs() < 1e-15);
        assert!((angular_separation(p(10.0, 45.0), p(190.0, -45.0)).0 - PI).abs() < 1e-14);
        // Identity: zero separation for coincident points.
        assert_eq!(angular_separation(p(37.0, -12.0), p(37.0, -12.0)).0, 0.0);
    }

    #[test]
    fn separation_matches_spherical_law_of_cosines() {
        // For (0,0) and (1 deg, 1 deg), cos d = cos(1 deg) * cos(1 deg) exactly.
        let d = angular_separation(p(0.0, 0.0), p(1.0, 1.0)).0;
        let cos1 = (1.0_f64.to_radians()).cos();
        assert!((d.cos() - cos1 * cos1).abs() < 1e-14);
    }

    #[test]
    fn separation_is_symmetric_and_bounded() {
        let pts = [p(0.0, 0.0), p(120.0, 30.0), p(-75.0, -60.0), p(359.0, 89.0)];
        for &a in &pts {
            for &b in &pts {
                let ab = angular_separation(a, b).0;
                let ba = angular_separation(b, a).0;
                assert!((ab - ba).abs() < 1e-14);
                assert!((0.0..=PI + 1e-15).contains(&ab));
            }
        }
    }

    #[test]
    fn position_angle_cardinal_directions() {
        let o = p(0.0, 0.0);
        assert!(position_angle(o, p(0.0, 10.0)).0.abs() < 1e-12); // north
        assert!((position_angle(o, p(10.0, 0.0)).0 - FRAC_PI_2).abs() < 1e-12); // east
        assert!((position_angle(o, p(0.0, -10.0)).0 - PI).abs() < 1e-12); // south
        assert!((position_angle(o, p(-10.0, 0.0)).0 - 1.5 * PI).abs() < 1e-12); // west
    }

    #[test]
    fn destination_then_separation_recovers_distance() {
        let start = p(20.0, 35.0);
        let bearing = Radians(0.8);
        let dist = Radians(0.3);
        let end = destination(start, bearing, dist);
        assert!((angular_separation(start, end).0 - dist.0).abs() < 1e-12);
        // Heading from start to end should match the bearing we set out on.
        assert!((position_angle(start, end).0 - bearing.0).abs() < 1e-10);
    }

    #[test]
    fn great_circle_interpolation_endpoints_and_midpoint() {
        let a = p(0.0, 0.0);
        let b = p(90.0, 0.0);
        assert_eq!(great_circle_interpolate(a, b, 0.0).unwrap(), a);
        let end = great_circle_interpolate(a, b, 1.0).unwrap();
        assert!(angular_separation(end, b).0 < 1e-12);
        let mid = great_circle_interpolate(a, b, 0.5).unwrap();
        assert!((mid.longitude.0 - FRAC_PI_4).abs() < 1e-12);
        assert!(mid.latitude.0.abs() < 1e-12);
        assert_eq!(
            great_circle_interpolate(a, p(180.0, 0.0), 0.5),
            Err(Error::Antipodal)
        );
    }

    #[test]
    fn cross_track_sign_and_magnitude() {
        let a = p(0.0, 0.0);
        let b = p(90.0, 0.0);
        // Point on the great circle has zero cross-track distance.
        assert!(cross_track_angle(p(45.0, 0.0), a, b).unwrap().0.abs() < 1e-12);
        // 10 degrees north of the equator, on the meridian at 45 degrees east.
        let left = cross_track_angle(p(45.0, 10.0), a, b).unwrap().0;
        assert!((left - 10.0_f64.to_radians()).abs() < 1e-12);
        let right = cross_track_angle(p(45.0, -10.0), a, b).unwrap().0;
        assert!((right + 10.0_f64.to_radians()).abs() < 1e-12);
        assert_eq!(cross_track_angle(a, a, a), Err(Error::ZeroVector));
    }

    #[test]
    fn cartesian_round_trip_table() {
        let table = [
            (0.0, 0.0),
            (90.0, 0.0),
            (-45.0, 60.0),
            (200.0, -89.0),
            (359.9, 0.1),
        ];
        for (lon, lat) in table {
            let s = p(lon, lat);
            let back = SphericalCoord::from_cartesian(s.to_cartesian()).unwrap();
            assert!(angular_separation(s, back).0 < 1e-14, "({lon}, {lat})");
            assert!((back.longitude.0 - s.longitude.0.rem_euclid(2.0 * PI)).abs() < 1e-12);
        }
        assert_eq!(
            SphericalCoord::from_cartesian(DVec3::ZERO),
            Err(Error::ZeroVector)
        );
    }
}
