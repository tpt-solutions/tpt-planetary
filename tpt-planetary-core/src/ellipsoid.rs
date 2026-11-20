//! Reference ellipsoids: geodetic, geocentric and Earth-centred Cartesian
//! coordinates.
//!
//! [`WGS84`] is the reference ellipsoid used by GPS. Geodetic latitude is the
//! angle between the ellipsoid normal and the equatorial plane. Geocentric
//! latitude is the angle of the radius vector from the centre. They differ by
//! up to about 11.5 arcminutes at mid-latitudes.
//!
//! Geodetic to Earth-centred Cartesian (ECEF) conversion is closed form.
//! ECEF to geodetic conversion is an iterative fixed-point method. It stops
//! when the latitude changes by less than `1e-14` rad, or fails with
//! [`Error::NonConvergence`] after 50 iterations. In practice it converges in
//! a small number of iterations for points near the ellipsoid surface. The
//! polar axis is handled in closed form.
//!
//! ```
//! use tpt_planetary_core::angle::{Degrees, Radians};
//! use tpt_planetary_core::ellipsoid::{Geodetic, WGS84};
//!
//! let equator = Geodetic { longitude: Radians(0.0), latitude: Radians(0.0), height: 0.0 };
//! let v = WGS84.geodetic_to_ecef(equator);
//! assert!((v.x - 6_378_137.0).abs() < 1e-6);
//! ```

use glam::f64::DVec3;

use crate::angle::Radians;
use crate::constants::{WGS84_INVERSE_FLATTENING, WGS84_SEMI_MAJOR_AXIS};
use crate::error::Error;
use crate::math;

/// Maximum iterations for ECEF to geodetic conversion.
const MAX_ITERATIONS: usize = 50;

/// Convergence tolerance for ECEF to geodetic latitude, in radians.
const TOLERANCE: f64 = 1e-14;

/// An oblate reference ellipsoid of revolution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ellipsoid {
    semi_major_axis: f64,
    inverse_flattening: f64,
}

/// The WGS84 reference ellipsoid (NGA.STND.0036).
pub const WGS84: Ellipsoid = Ellipsoid {
    semi_major_axis: WGS84_SEMI_MAJOR_AXIS,
    inverse_flattening: WGS84_INVERSE_FLATTENING,
};

/// A position as geodetic longitude, latitude and height above the ellipsoid.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Geodetic {
    /// Longitude, east positive.
    pub longitude: Radians,
    /// Geodetic latitude, north positive.
    pub latitude: Radians,
    /// Height above the ellipsoid, in metres.
    pub height: f64,
}

impl Ellipsoid {
    /// Builds an ellipsoid from its semi-major axis (metres) and inverse
    /// flattening. The inverse flattening must be finite and greater than 1.
    ///
    /// ```
    /// use tpt_planetary_core::ellipsoid::Ellipsoid;
    /// let e = Ellipsoid::new(6_378_137.0, 298.257_223_563);
    /// assert_eq!(e.semi_major_axis(), 6_378_137.0);
    /// ```
    pub const fn new(semi_major_axis: f64, inverse_flattening: f64) -> Self {
        Ellipsoid {
            semi_major_axis,
            inverse_flattening,
        }
    }

    /// Returns the semi-major (equatorial) axis, in metres.
    pub fn semi_major_axis(self) -> f64 {
        self.semi_major_axis
    }

    /// Returns the flattening `f = 1 / inverse_flattening`.
    pub fn flattening(self) -> f64 {
        1.0 / self.inverse_flattening
    }

    /// Returns the semi-minor (polar) axis `b = a (1 - f)`, in metres.
    pub fn semi_minor_axis(self) -> f64 {
        self.semi_major_axis * (1.0 - self.flattening())
    }

    /// Returns the first eccentricity squared, `e^2 = f (2 - f)`.
    pub fn eccentricity_squared(self) -> f64 {
        let f = self.flattening();
        f * (2.0 - f)
    }

    /// Converts a geodetic position to Earth-centred Cartesian coordinates, in metres.
    ///
    /// The X axis passes through the equator at longitude 0, the Z axis
    /// through the north pole.
    pub fn geodetic_to_ecef(self, g: Geodetic) -> DVec3 {
        let e2 = self.eccentricity_squared();
        let (sb, cb) = g.latitude.sin_cos();
        let (sl, cl) = g.longitude.sin_cos();
        let n = self.semi_major_axis / math::sqrt(1.0 - e2 * sb * sb);
        DVec3::new(
            (n + g.height) * cb * cl,
            (n + g.height) * cb * sl,
            (n * (1.0 - e2) + g.height) * sb,
        )
    }

    /// Converts Earth-centred Cartesian coordinates (metres) to geodetic.
    ///
    /// # Errors
    ///
    /// [`Error::NonConvergence`] if the iteration does not converge (this does
    /// not happen for finite input in practice), or [`Error::ZeroVector`] if
    /// `v` is the origin, which lies inside the ellipsoid.
    pub fn ecef_to_geodetic(self, v: DVec3) -> Result<Geodetic, Error> {
        if !v.is_finite() {
            return Err(Error::NonConvergence);
        }
        if v == DVec3::ZERO {
            return Err(Error::ZeroVector);
        }
        let e2 = self.eccentricity_squared();
        let a = self.semi_major_axis;
        let b = self.semi_minor_axis();
        let p = math::hypot(v.x, v.y);
        let longitude = Radians(math::atan2(v.y, v.x));

        if p == 0.0 {
            // On the polar axis: the latitude is exact, and the height is the distance from the pole.
            let lat = if v.z >= 0.0 {
                core::f64::consts::FRAC_PI_2
            } else {
                -core::f64::consts::FRAC_PI_2
            };
            return Ok(Geodetic {
                longitude,
                latitude: Radians(lat),
                height: v.z.abs() - b,
            });
        }

        // Initial guess from the geocentric latitude.
        let mut lat = math::atan2(v.z, p * (1.0 - e2));
        let mut converged = false;
        for _ in 0..MAX_ITERATIONS {
            let (sb, cb) = math::sin_cos(lat);
            let n = a / math::sqrt(1.0 - e2 * sb * sb);
            let h = height_from(p, v.z, sb, cb, n, e2);
            let next = math::atan2(v.z, p * (1.0 - e2 * n / (n + h)));
            let delta = (next - lat).abs();
            lat = next;
            if delta < TOLERANCE {
                converged = true;
                break;
            }
        }
        if !converged {
            return Err(Error::NonConvergence);
        }
        let (sb, cb) = math::sin_cos(lat);
        let n = a / math::sqrt(1.0 - e2 * sb * sb);
        let height = height_from(p, v.z, sb, cb, n, e2);
        Ok(Geodetic {
            longitude,
            latitude: Radians(lat),
            height,
        })
    }

    /// Converts a geodetic latitude to geocentric latitude:
    /// `tan(lat_c) = (1 - e^2) tan(lat)`.
    ///
    /// ```
    /// use tpt_planetary_core::angle::Degrees;
    /// use tpt_planetary_core::ellipsoid::WGS84;
    /// let lat = WGS84.geodetic_to_geocentric_latitude(Degrees(45.0).to_radians());
    /// assert!(lat.to_degrees().0 < 45.0);
    /// ```
    pub fn geodetic_to_geocentric_latitude(self, lat: Radians) -> Radians {
        let k = 1.0 - self.eccentricity_squared();
        let (sb, cb) = lat.sin_cos();
        Radians(math::atan2(k * sb, cb))
    }

    /// Converts a geocentric latitude to geodetic latitude (the inverse of
    /// [`Ellipsoid::geodetic_to_geocentric_latitude`]).
    pub fn geocentric_to_geodetic_latitude(self, lat: Radians) -> Radians {
        let k = 1.0 - self.eccentricity_squared();
        let (sb, cb) = lat.sin_cos();
        Radians(math::atan2(sb, k * cb))
    }
}

/// The geodesic between two points on the ellipsoid surface, from the
/// inverse problem.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeodesicInverse {
    /// Length of the geodesic, in metres.
    pub distance: f64,
    /// Azimuth at the first point, clockwise from north.
    pub azimuth_start: Radians,
    /// Azimuth at the second point, continuing along the geodesic.
    pub azimuth_end: Radians,
}

/// The end point of a geodesic, from the direct problem.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeodesicDirect {
    /// Longitude of the end point, east positive.
    pub longitude: Radians,
    /// Geodetic latitude of the end point, north positive.
    pub latitude: Radians,
    /// Azimuth at the end point, continuing along the geodesic.
    pub azimuth_end: Radians,
}

/// Maximum iterations for Vincenty's iterations.
const VINCENTY_MAX_ITERATIONS: usize = 200;

/// Convergence tolerance for Vincenty's iterations, in radians (about
/// 6 micrometres of arc on the equator).
const VINCENTY_TOLERANCE: f64 = 1e-12;

impl Ellipsoid {
    /// Solves the geodesic inverse problem with Vincenty's formulae (1975).
    ///
    /// Points are given as longitude and geodetic latitude. The result is the
    /// shortest geodesic on the ellipsoid surface. The formulae are accurate
    /// to well under a millimetre for ordinary separations.
    ///
    /// # Errors
    ///
    /// [`Error::NonConvergence`] for nearly antipodal points, where Vincenty's
    /// iteration may not converge.
    pub fn geodesic_inverse(
        self,
        lon1: Radians,
        lat1: Radians,
        lon2: Radians,
        lat2: Radians,
    ) -> Result<GeodesicInverse, Error> {
        let a = self.semi_major_axis;
        let f = self.flattening();
        let b = self.semi_minor_axis();
        let big_l = lon2.0 - lon1.0;

        let (sin_u1, cos_u1) = reduced_latitude(lat1.0, f);
        let (sin_u2, cos_u2) = reduced_latitude(lat2.0, f);

        let mut lambda = big_l;
        let mut converged = false;
        let mut sin_sigma = 0.0;
        let mut cos_sigma = 1.0;
        let mut sigma = 0.0;
        let mut sin_alpha;
        let mut cos2_alpha = 1.0;
        let mut cos_2sigma_m = 0.0;
        for _ in 0..VINCENTY_MAX_ITERATIONS {
            let (sin_lambda, cos_lambda) = math::sin_cos(lambda);
            let t1 = cos_u2 * sin_lambda;
            let t2 = cos_u1 * sin_u2 - sin_u1 * cos_u2 * cos_lambda;
            sin_sigma = math::sqrt(t1 * t1 + t2 * t2);
            if sin_sigma == 0.0 {
                // Coincident points.
                return Ok(GeodesicInverse {
                    distance: 0.0,
                    azimuth_start: Radians(0.0),
                    azimuth_end: Radians(0.0),
                });
            }
            cos_sigma = sin_u1 * sin_u2 + cos_u1 * cos_u2 * cos_lambda;
            sigma = math::atan2(sin_sigma, cos_sigma);
            sin_alpha = cos_u1 * cos_u2 * sin_lambda / sin_sigma;
            cos2_alpha = 1.0 - sin_alpha * sin_alpha;
            cos_2sigma_m = if cos2_alpha != 0.0 {
                cos_sigma - 2.0 * sin_u1 * sin_u2 / cos2_alpha
            } else {
                // Equatorial line: the midpoint is on the equator.
                0.0
            };
            let c = f / 16.0 * cos2_alpha * (4.0 + f * (4.0 - 3.0 * cos2_alpha));
            let previous = lambda;
            lambda = big_l
                + (1.0 - c)
                    * f
                    * sin_alpha
                    * (sigma
                        + c * sin_sigma
                            * (cos_2sigma_m
                                + c * cos_sigma * (-1.0 + 2.0 * cos_2sigma_m * cos_2sigma_m)));
            if (lambda - previous).abs() < VINCENTY_TOLERANCE {
                converged = true;
                break;
            }
        }
        if !converged {
            return Err(Error::NonConvergence);
        }

        let (sin_lambda, cos_lambda) = math::sin_cos(lambda);
        let u2 = cos2_alpha * (a * a - b * b) / (b * b);
        let big_a = 1.0 + u2 / 16384.0 * (4096.0 + u2 * (-768.0 + u2 * (320.0 - 175.0 * u2)));
        let big_b = u2 / 1024.0 * (256.0 + u2 * (-128.0 + u2 * (74.0 - 47.0 * u2)));
        let delta_sigma = big_b
            * sin_sigma
            * (cos_2sigma_m
                + big_b / 4.0
                    * (cos_sigma * (-1.0 + 2.0 * cos_2sigma_m * cos_2sigma_m)
                        - big_b / 6.0
                            * cos_2sigma_m
                            * (-3.0 + 4.0 * sin_sigma * sin_sigma)
                            * (-3.0 + 4.0 * cos_2sigma_m * cos_2sigma_m)));
        let distance = b * big_a * (sigma - delta_sigma);

        let azimuth_start = math::atan2(
            cos_u2 * sin_lambda,
            cos_u1 * sin_u2 - sin_u1 * cos_u2 * cos_lambda,
        );
        let azimuth_end = math::atan2(
            cos_u1 * sin_lambda,
            -sin_u1 * cos_u2 + cos_u1 * sin_u2 * cos_lambda,
        );
        Ok(GeodesicInverse {
            distance,
            azimuth_start: Radians(azimuth_start),
            azimuth_end: Radians(azimuth_end),
        })
    }

    /// Solves the geodesic direct problem with Vincenty's formulae (1975): the
    /// point reached from `(lon, lat)` after `distance` metres along the
    /// geodesic that leaves at `azimuth` (clockwise from north).
    ///
    /// # Errors
    ///
    /// [`Error::NonConvergence`] if the series iteration does not converge.
    pub fn geodesic_direct(
        self,
        lon: Radians,
        lat: Radians,
        azimuth: Radians,
        distance: f64,
    ) -> Result<GeodesicDirect, Error> {
        let a = self.semi_major_axis;
        let f = self.flattening();
        let b = self.semi_minor_axis();

        let (sin_u1, cos_u1) = reduced_latitude(lat.0, f);
        let (sin_alpha1, cos_alpha1) = math::sin_cos(azimuth.0);
        let sigma1 = math::atan2(sin_u1, cos_u1 * cos_alpha1);
        let sin_alpha = cos_u1 * sin_alpha1;
        let cos2_alpha = 1.0 - sin_alpha * sin_alpha;
        let u2 = cos2_alpha * (a * a - b * b) / (b * b);
        let big_a = 1.0 + u2 / 16384.0 * (4096.0 + u2 * (-768.0 + u2 * (320.0 - 175.0 * u2)));
        let big_b = u2 / 1024.0 * (256.0 + u2 * (-128.0 + u2 * (74.0 - 47.0 * u2)));

        let base = distance / (b * big_a);
        let mut sigma = base;
        let mut converged = false;
        for _ in 0..VINCENTY_MAX_ITERATIONS {
            let cos_2sigma_m = math::sin_cos(2.0 * sigma1 + sigma).1;
            let (sin_sigma, cos_sigma) = math::sin_cos(sigma);
            let delta_sigma = big_b
                * sin_sigma
                * (cos_2sigma_m
                    + big_b / 4.0
                        * (cos_sigma * (-1.0 + 2.0 * cos_2sigma_m * cos_2sigma_m)
                            - big_b / 6.0
                                * cos_2sigma_m
                                * (-3.0 + 4.0 * sin_sigma * sin_sigma)
                                * (-3.0 + 4.0 * cos_2sigma_m * cos_2sigma_m)));
            let previous = sigma;
            sigma = base + delta_sigma;
            if (sigma - previous).abs() < VINCENTY_TOLERANCE {
                converged = true;
                break;
            }
        }
        if !converged {
            return Err(Error::NonConvergence);
        }
        let cos_2sigma_m = math::sin_cos(2.0 * sigma1 + sigma).1;
        let (sin_sigma, cos_sigma) = math::sin_cos(sigma);

        let x = sin_u1 * cos_sigma + cos_u1 * sin_sigma * cos_alpha1;
        let y = sin_u1 * sin_sigma - cos_u1 * cos_sigma * cos_alpha1;
        let lat2 = math::atan2(x, (1.0 - f) * math::sqrt(sin_alpha * sin_alpha + y * y));
        let lambda = math::atan2(
            sin_sigma * sin_alpha1,
            cos_u1 * cos_sigma - sin_u1 * sin_sigma * cos_alpha1,
        );
        let c = f / 16.0 * cos2_alpha * (4.0 + f * (4.0 - 3.0 * cos2_alpha));
        let big_l = lambda
            - (1.0 - c)
                * f
                * sin_alpha
                * (sigma
                    + c * sin_sigma
                        * (cos_2sigma_m
                            + c * cos_sigma * (-1.0 + 2.0 * cos_2sigma_m * cos_2sigma_m)));
        let azimuth_end = math::atan2(
            sin_alpha,
            -sin_u1 * sin_sigma + cos_u1 * cos_sigma * cos_alpha1,
        );
        Ok(GeodesicDirect {
            longitude: Radians(lon.0 + big_l),
            latitude: Radians(lat2),
            azimuth_end: Radians(azimuth_end),
        })
    }
}

/// Sine and cosine of the reduced latitude `U`, where `tan U = (1 - f) tan lat`.
fn reduced_latitude(lat: f64, f: f64) -> (f64, f64) {
    let (sin_lat, cos_lat) = math::sin_cos(lat);
    let (sin_u, cos_u) = ((1.0 - f) * sin_lat, cos_lat);
    let norm = math::hypot(sin_u, cos_u);
    (sin_u / norm, cos_u / norm)
}

/// Height above the ellipsoid of the point `(p, z)` at geodetic latitude
/// given by `sin_b` and `cos_b`, with prime-vertical radius `n`.
///
/// Uses the form that is well conditioned near the equator and near the poles.
fn height_from(p: f64, z: f64, sb: f64, cb: f64, n: f64, e2: f64) -> f64 {
    if cb.abs() > sb.abs() {
        p / cb - n
    } else {
        z / sb - n * (1.0 - e2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::angle::Degrees;

    fn geo(lon: f64, lat: f64, h: f64) -> Geodetic {
        Geodetic {
            longitude: Degrees(lon).to_radians(),
            latitude: Degrees(lat).to_radians(),
            height: h,
        }
    }

    #[test]
    fn wgs84_derived_constants() {
        // b = 6 356 752.314 245 m; e^2 = 0.006 694 379 990 14 (WGS84 published values).
        assert!((WGS84.semi_minor_axis() - 6_356_752.314_245).abs() < 1e-6);
        assert!((WGS84.eccentricity_squared() - 0.006_694_379_990_14).abs() < 1e-12);
    }

    #[test]
    fn geodetic_to_ecef_golden_points() {
        // Equator at longitude 0, height 0: X is the equatorial radius.
        let v = WGS84.geodetic_to_ecef(geo(0.0, 0.0, 0.0));
        assert!((v - DVec3::new(6_378_137.0, 0.0, 0.0)).length() < 1e-6);
        // North pole: Z is the polar radius b.
        let v = WGS84.geodetic_to_ecef(geo(0.0, 90.0, 0.0));
        assert!(v.x.abs() < 1e-6 && v.y.abs() < 1e-6);
        assert!((v.z - WGS84.semi_minor_axis()).abs() < 1e-6);
        // Longitude 90 degrees east on the equator maps to +Y.
        let v = WGS84.geodetic_to_ecef(geo(90.0, 0.0, 0.0));
        assert!(v.x.abs() < 1e-6 && (v.y - 6_378_137.0).abs() < 1e-6);
    }

    #[test]
    fn ecef_round_trip_table() {
        let table = [
            geo(0.0, 0.0, 0.0),
            geo(0.0, 0.0, 1_000.0),
            geo(-122.4, 37.8, 16.0),
            geo(151.2, -33.9, 58.0),
            geo(10.0, 89.9999, 12.0),
            geo(-70.0, -89.0, 2_800.0),
            geo(45.0, 45.0, -400.0),
            geo(180.0, -23.5, 35_786_000.0),
        ];
        for g in table {
            let v = WGS84.geodetic_to_ecef(g);
            let back = WGS84.ecef_to_geodetic(v).unwrap();
            assert!((back.latitude.0 - g.latitude.0).abs() < 1e-11, "lat {g:?}");
            assert!(
                (back.height - g.height).abs() < 1e-5,
                "height {g:?} -> {back:?}"
            );
            let dlon = (back.longitude.0 - g.longitude.0).rem_euclid(core::f64::consts::TAU);
            assert!(dlon.min(core::f64::consts::TAU - dlon) < 1e-11, "lon {g:?}");
        }
    }

    #[test]
    fn poles_and_axis_are_exact() {
        let north = WGS84
            .ecef_to_geodetic(DVec3::new(0.0, 0.0, 6_356_752.314_245 + 100.0))
            .unwrap();
        assert!((north.latitude.0 - core::f64::consts::FRAC_PI_2).abs() < 1e-15);
        assert!((north.height - 100.0).abs() < 1e-6);
        assert_eq!(WGS84.ecef_to_geodetic(DVec3::ZERO), Err(Error::ZeroVector));
    }

    #[test]
    fn geocentric_latitude_round_trip_and_gap() {
        let lat = Degrees(45.0).to_radians();
        let gc = WGS84.geodetic_to_geocentric_latitude(lat);
        // At 45 degrees the geodetic-geocentric difference is about 0.19 degrees.
        let diff = (lat.0 - gc.0).to_degrees();
        assert!((diff - 0.192).abs() < 0.01, "diff = {diff}");
        let back = WGS84.geocentric_to_geodetic_latitude(gc);
        assert!((back.0 - lat.0).abs() < 1e-15);
        // Equator and poles are fixed points.
        assert_eq!(WGS84.geodetic_to_geocentric_latitude(Radians(0.0)).0, 0.0);
    }

    #[test]
    fn vincenty_inverse_reference_pair() {
        // Vincenty (1975) worked example: Flinders Peak to Buninyong on WGS84.
        let (lon1, lat1) = (
            Degrees(144.424_867_888_888_9),
            Degrees(-37.951_033_416_666_67),
        );
        let (lon2, lat2) = (
            Degrees(143.926_495_527_777_8),
            Degrees(-37.652_821_138_888_89),
        );
        let g = WGS84
            .geodesic_inverse(
                lon1.to_radians(),
                lat1.to_radians(),
                lon2.to_radians(),
                lat2.to_radians(),
            )
            .unwrap();
        assert!(
            (g.distance - 54_972.271).abs() < 1e-3,
            "distance {}",
            g.distance
        );
        // 306 deg 52' 05.37" and 127 deg 10' 25.07".
        let dms = |d: f64, m: f64, s: f64| d + m / 60.0 + s / 3600.0;
        // The paper's second azimuth points back towards the first point; the
        // forward azimuth at the second point is 180 degrees more.
        let start = g.azimuth_start.to_degrees().0.rem_euclid(360.0);
        let back = (g.azimuth_end.to_degrees().0 + 180.0).rem_euclid(360.0);
        assert!(
            (start - dms(306.0, 52.0, 5.37)).abs() < 1e-5,
            "start {start}"
        );
        assert!((back - dms(127.0, 10.0, 25.07)).abs() < 1e-5, "back {back}");
    }

    #[test]
    fn vincenty_direct_inverts_inverse() {
        let start = (Degrees(-122.4).to_radians(), Degrees(37.8).to_radians());
        let end = (Degrees(151.2).to_radians(), Degrees(-33.9).to_radians());
        let inv = WGS84
            .geodesic_inverse(start.0, start.1, end.0, end.1)
            .unwrap();
        let dir = WGS84
            .geodesic_direct(start.0, start.1, inv.azimuth_start, inv.distance)
            .unwrap();
        assert!((dir.latitude.0 - end.1.0).abs() < 1e-10);
        let dlon = (dir.longitude.0 - end.0.0).rem_euclid(core::f64::consts::TAU);
        assert!(dlon.min(core::f64::consts::TAU - dlon) < 1e-10);
        assert!((dir.azimuth_end.0 - inv.azimuth_end.0).abs() < 1e-10);
    }

    #[test]
    fn vincenty_equator_and_coincident_points() {
        // Along the equator the geodesic is an arc of the equatorial circle.
        let g = WGS84
            .geodesic_inverse(
                Radians(0.0),
                Radians(0.0),
                Degrees(1.0).to_radians(),
                Radians(0.0),
            )
            .unwrap();
        let arc = WGS84.semi_major_axis() * Degrees(1.0).to_radians().0;
        assert!((g.distance - arc).abs() < 1e-6, "{} vs {arc}", g.distance);
        assert!((g.azimuth_start.to_degrees().0 - 90.0).abs() < 1e-9);
        let same = WGS84
            .geodesic_inverse(Radians(0.5), Radians(0.2), Radians(0.5), Radians(0.2))
            .unwrap();
        assert_eq!(same.distance, 0.0);
    }
}
