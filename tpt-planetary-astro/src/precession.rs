//! IAU 2006 precession of the equator and equinox (Capitaine, Wallace &
//! Chapront 2003; adopted by IAU 2006 Resolution B1).
//!
//! This module covers **precession only**. Nutation and the equation of the
//! equinoxes are in [`crate::nutation`]. This module does not implement the
//! celestial intermediate origin or the TIO locator s', and it does not apply
//! the ICRS -> J2000 frame bias (about 17 mas in the equator). The matrix returned here maps the mean
//! equator and equinox of J2000.0 to the mean equator and equinox of date.
//! Treating ICRS axes as the J2000 mean equator is the bias-free convention
//! used by [`icrs_to_mean_equator_of_date`].
//!
//! # Angles
//!
//! The three precession angles are polynomials in `t`, Julian centuries of
//! TT from J2000.0 (`t = (JD(TT) - 2451545.0) / 36525`). Their units are
//! arcseconds, and each polynomial has the form printed in CWC 2003 Table 1:
//!
//! - `zeta_A(t) = 2.650545 + 2306.083227 t + 0.2988499 t^2 + 0.01801828 t^3 - 0.000005971 t^4 - 0.0000003173 t^5`
//! - `z_A(t)    = -2.650545 + 2306.077181 t + 1.0927348 t^2 + 0.01826837 t^3 - 0.000028596 t^4 - 0.0000002904 t^5`
//! - `theta_A(t) = 2004.191903 t - 0.4294934 t^2 - 0.04182264 t^3 - 0.000007089 t^4 - 0.0000001274 t^5`
//!
//! Note that `zeta_A(0)` and `z_A(0)` are not zero: they are `+2.650545` and
//! `-2.650545` arcseconds. Their sum is zero, so `P(0)` is the identity, but
//! the individual angles carry a constant offset.
//!
//! # Matrix
//!
//! `P(t) = R3(-z_A) R2(theta_A) R3(-zeta_A)`, with the passive rotation
//! convention of this crate (`R3(a) = [[c, s, 0], [-s, c, 0], [0, 0, 1]]`,
//! `R2(a) = [[c, 0, -s], [0, 1, 0], [s, 0, c]]`). For a unit vector `v` in
//! the J2000 mean equatorial frame, `P(t) v` is the same direction in the mean
//! equatorial frame of date, and `P(t)^T` maps back.
//!
//! # Validation
//!
//! Checked against `erfa.p06e` (zeta, z, theta) and against the precession
//! matrix from `erfa.bp06` (pyerfa 2.0.1.5) over 1900-2100. See
//! `docs/validation.md`. Nutation is validated in [`crate::nutation`].

use core::f64::consts::PI;

/// Radians per arcsecond.
const RAD_PER_ARCSEC: f64 = PI / (180.0 * 3600.0);

/// Offset of `zeta_A` at J2000 (arcseconds), CWC 2003 Table 1.
const ZETA_0_ARCSEC: f64 = 2.650_545;
/// Offset of `z_A` at J2000 (arcseconds), CWC 2003 Table 1. Equal to `-ZETA_0_ARCSEC`.
const Z_0_ARCSEC: f64 = -2.650_545;

/// Precession angle `zeta_A` in arcseconds at `t` Julian centuries of TT from J2000.
///
/// Polynomial from Capitaine, Wallace & Chapront (2003), Table 1, as adopted
/// by IAU 2006 Resolution B1. Non-zero at `t = 0`: the value there is
/// `2.650545` arcseconds, and `zeta_A` has the rate `2306.083227` arcseconds
/// per century at `t = 0`.
///
/// ```
/// use tpt_planetary_astro::precession::zeta_a_arcsec;
/// assert!((zeta_a_arcsec(0.0) - 2.650_545).abs() < 1e-12);
/// // About 2309.05 arcseconds one century after J2000.
/// assert!((zeta_a_arcsec(1.0) - 2309.050).abs() < 1e-3);
/// ```
#[must_use]
pub fn zeta_a_arcsec(t: f64) -> f64 {
    ZETA_0_ARCSEC
        + t * (2_306.083_227
            + t * (0.298_849_9 + t * (0.018_018_28 + t * (-0.000_005_971 + t * -0.000_000_317_3))))
}

/// Precession angle `z_A` in arcseconds at `t` Julian centuries of TT from J2000.
///
/// Polynomial from Capitaine, Wallace & Chapront (2003), Table 1. Non-zero at
/// `t = 0`: the value there is `-2.650545` arcseconds, and `z_A` has the rate
/// `2306.077181` arcseconds per century at `t = 0`.
///
/// ```
/// use tpt_planetary_astro::precession::z_a_arcsec;
/// assert!((z_a_arcsec(0.0) + 2.650_545).abs() < 1e-12);
/// ```
#[must_use]
pub fn z_a_arcsec(t: f64) -> f64 {
    Z_0_ARCSEC
        + t * (2_306.077_181
            + t * (1.092_734_8 + t * (0.018_268_37 + t * (-0.000_028_596 + t * -0.000_000_290_4))))
}

/// Precession angle `theta_A` in arcseconds at `t` Julian centuries of TT from J2000.
///
/// Polynomial from Capitaine, Wallace & Chapront (2003), Table 1. Zero at
/// `t = 0`, with the rate `2004.191903` arcseconds per century there.
///
/// ```
/// use tpt_planetary_astro::precession::theta_a_arcsec;
/// assert_eq!(theta_a_arcsec(0.0), 0.0);
/// ```
#[must_use]
pub fn theta_a_arcsec(t: f64) -> f64 {
    t * (2_004.191_903
        + t * (-0.429_493_4 + t * (-0.041_822_64 + t * (-0.000_007_089 + t * -0.000_000_127_4))))
}

/// The three precession angles `(zeta_A, z_A, theta_A)` in arcseconds at `t`.
///
/// ```
/// use tpt_planetary_astro::precession::precession_angles_arcsec;
/// let (zeta, z, theta) = precession_angles_arcsec(0.0);
/// assert!((zeta + z).abs() < 1e-12);
/// assert_eq!(theta, 0.0);
/// ```
#[must_use]
pub fn precession_angles_arcsec(t: f64) -> (f64, f64, f64) {
    (zeta_a_arcsec(t), z_a_arcsec(t), theta_a_arcsec(t))
}

/// Precession angles in radians, `(zeta_A, z_A, theta_A)`, at `t`.
#[must_use]
pub fn precession_angles_rad(t: f64) -> (f64, f64, f64) {
    let (zeta, z, theta) = precession_angles_arcsec(t);
    (
        zeta * RAD_PER_ARCSEC,
        z * RAD_PER_ARCSEC,
        theta * RAD_PER_ARCSEC,
    )
}

#[cfg(feature = "std")]
pub(crate) mod matrix_impl {
    use super::precession_angles_rad;
    use crate::frames::{Frame, Icrs, SkyCoord};

    /// 3x3 matrix, row-major.
    pub type Matrix3 = [[f64; 3]; 3];

    /// Mean equator and equinox of date (IAU 2006 precession, no nutation).
    ///
    /// Used as the target of [`icrs_to_mean_equator_of_date`]. Longitude is
    /// right ascension of date and latitude is declination of date.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct MeanEquatorOfDate;

    impl Frame for MeanEquatorOfDate {
        const NAME: &'static str = "MeanEquatorOfDate(IAU2006, no nutation)";
    }

    /// Mean ecliptic and equinox of date (IAU 2006 precession and mean
    /// obliquity, no nutation). Longitude is ecliptic longitude of date and
    /// latitude is ecliptic latitude of date.
    ///
    /// Used as the target of [`icrs_to_mean_ecliptic_of_date`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct MeanEclipticOfDate;

    impl Frame for MeanEclipticOfDate {
        const NAME: &'static str = "MeanEcliptic(IAU2006 of date, no nutation)";
    }

    /// Passive rotation about the x-axis by `theta` radians.
    fn rot_x(theta: f64) -> Matrix3 {
        let (s, c) = theta.sin_cos();
        [[1.0, 0.0, 0.0], [0.0, c, s], [0.0, -s, c]]
    }

    /// The ICRS to mean-ecliptic-of-date matrix at TT `(tt_hi, tt_lo)`:
    /// precession to the mean equator of date, then a rotation by the IAU 2006
    /// mean obliquity of date about the x-axis.
    fn icrs_to_mean_ecliptic_matrix(tt_hi: f64, tt_lo: f64) -> Matrix3 {
        let t = ((tt_hi - crate::time::J2000_JD) + tt_lo) / 36_525.0;
        let eps = crate::time::mean_obliquity_iau2006_rad(tt_hi, tt_lo);
        mat_mul(&rot_x(eps), &precession_matrix(t))
    }

    /// Converts an ICRS direction to the mean ecliptic and equinox of date at
    /// TT `(tt_hi, tt_lo)`.
    ///
    /// The ICRS axes are taken as the J2000 mean equator and equinox, as in
    /// [`icrs_to_mean_equator_of_date`], so the frame bias is not applied. Nutation
    /// is not applied either. At J2000.0 the result equals the J2000 mean ecliptic
    /// ([`crate::frames::MeanEcliptic`]) to the frame bias.
    ///
    /// ```
    /// use tpt_planetary_astro::frames::{Icrs, MeanEcliptic, SkyCoord};
    /// use tpt_planetary_astro::precession::icrs_to_mean_ecliptic_of_date;
    /// let star = SkyCoord::<Icrs>::from_degrees(10.0, 20.0);
    /// let j2000 = (tpt_planetary_astro::time::J2000_JD, 0.0);
    /// let date = icrs_to_mean_ecliptic_of_date(star, j2000.0, j2000.1);
    /// let ref_j2000 = star.transform_to::<MeanEcliptic>();
    /// assert!((date.lon() - ref_j2000.lon()).abs() < 1e-10);
    /// assert!((date.lat() - ref_j2000.lat()).abs() < 1e-10);
    /// ```
    #[must_use]
    pub fn icrs_to_mean_ecliptic_of_date(
        coord: SkyCoord<Icrs>,
        tt_hi: f64,
        tt_lo: f64,
    ) -> SkyCoord<MeanEclipticOfDate> {
        rotate_direction(
            &icrs_to_mean_ecliptic_matrix(tt_hi, tt_lo),
            coord.lon(),
            coord.lat(),
        )
    }

    /// Inverse of [`icrs_to_mean_ecliptic_of_date`].
    #[must_use]
    pub fn mean_ecliptic_of_date_to_icrs(
        coord: SkyCoord<MeanEclipticOfDate>,
        tt_hi: f64,
        tt_lo: f64,
    ) -> SkyCoord<Icrs> {
        rotate_direction(
            &transpose(&icrs_to_mean_ecliptic_matrix(tt_hi, tt_lo)),
            coord.lon(),
            coord.lat(),
        )
    }

    pub(crate) fn mat_mul(a: &Matrix3, b: &Matrix3) -> Matrix3 {
        let mut out = [[0.0; 3]; 3];
        for (i, row) in out.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
            }
        }
        out
    }

    pub(crate) fn mat_vec(m: &Matrix3, v: [f64; 3]) -> [f64; 3] {
        [
            m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
            m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
            m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
        ]
    }

    /// Transpose of a 3x3 matrix.
    #[must_use]
    pub fn transpose(m: &Matrix3) -> Matrix3 {
        let mut out = [[0.0; 3]; 3];
        for (i, row) in out.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = m[j][i];
            }
        }
        out
    }

    /// Passive rotation about the z-axis (same convention as `frames`).
    fn rot_z(theta: f64) -> Matrix3 {
        let (s, c) = theta.sin_cos();
        [[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]]
    }

    /// Passive rotation about the y-axis (same convention as `frames`).
    fn rot_y(theta: f64) -> Matrix3 {
        let (s, c) = theta.sin_cos();
        [[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]]
    }

    /// IAU 2006 precession matrix `P(t) = R3(-z_A) R2(theta_A) R3(-zeta_A)`.
    ///
    /// Maps a vector in the mean equatorial frame of J2000.0 to the mean
    /// equatorial frame of date at `t` Julian centuries of TT from J2000.
    /// Precession only: no nutation and no frame bias.
    ///
    /// ```
    /// use tpt_planetary_astro::precession::precession_matrix;
    /// let p = precession_matrix(0.0);
    /// for i in 0..3 {
    ///     for j in 0..3 {
    ///         let delta = if i == j { 1.0 } else { 0.0 };
    ///         assert!((p[i][j] - delta).abs() < 1e-15);
    ///     }
    /// }
    /// ```
    #[must_use]
    pub fn precession_matrix(t: f64) -> Matrix3 {
        let (zeta, z, theta) = precession_angles_rad(t);
        mat_mul(&rot_z(-z), &mat_mul(&rot_y(theta), &rot_z(-zeta)))
    }

    /// Transpose of [`precession_matrix`]: maps the mean equator of date back
    /// to the mean equator of J2000.0.
    ///
    /// ```
    /// use tpt_planetary_astro::precession::{precession_matrix, precession_matrix_transpose};
    /// let p = precession_matrix(1.0);
    /// let q = precession_matrix_transpose(1.0);
    /// assert!((p[0][1] - q[1][0]).abs() < 1e-15);
    /// ```
    #[must_use]
    pub fn precession_matrix_transpose(t: f64) -> Matrix3 {
        transpose(&precession_matrix(t))
    }

    /// Converts an ICRS direction to the mean equator and equinox of date at
    /// `t` Julian centuries of TT from J2000.
    ///
    /// The ICRS axes are taken as the J2000 mean equator and equinox, so the
    /// ICRS -> J2000 frame bias is not applied (about 17 mas). Nutation and the
    /// equation of the equinoxes are not applied, so the result is not the true
    /// equator of date.
    ///
    /// This is a free function rather than a [`FrameTransform`](crate::frames::FrameTransform)
    /// implementation because the transform depends on the epoch `t`, which the
    /// trait does not carry.
    ///
    /// ```
    /// use tpt_planetary_astro::frames::{Icrs, SkyCoord};
    /// use tpt_planetary_astro::precession::icrs_to_mean_equator_of_date;
    /// let star = SkyCoord::<Icrs>::from_degrees(10.0, 20.0);
    /// let date = icrs_to_mean_equator_of_date(star, 0.0);
    /// // At J2000 the mean equator of date equals ICRS (no bias applied).
    /// assert!((date.lon() - star.lon()).abs() < 1e-14);
    /// assert!((date.lat() - star.lat()).abs() < 1e-14);
    /// ```
    #[must_use]
    pub fn icrs_to_mean_equator_of_date(
        coord: SkyCoord<Icrs>,
        t: f64,
    ) -> SkyCoord<MeanEquatorOfDate> {
        rotate_direction(&precession_matrix(t), coord.lon(), coord.lat())
    }

    /// Inverse of [`icrs_to_mean_equator_of_date`]: mean equator of date back
    /// to ICRS (J2000 mean equator, no frame bias).
    ///
    /// ```
    /// use tpt_planetary_astro::frames::{Icrs, SkyCoord};
    /// use tpt_planetary_astro::precession::{icrs_to_mean_equator_of_date, mean_equator_of_date_to_icrs};
    /// let star = SkyCoord::<Icrs>::from_degrees(250.0, -40.0);
    /// let back = mean_equator_of_date_to_icrs(icrs_to_mean_equator_of_date(star, 0.5), 0.5);
    /// assert!((back.lon() - star.lon()).abs() < 1e-13);
    /// assert!((back.lat() - star.lat()).abs() < 1e-13);
    /// ```
    #[must_use]
    pub fn mean_equator_of_date_to_icrs(
        coord: SkyCoord<MeanEquatorOfDate>,
        t: f64,
    ) -> SkyCoord<Icrs> {
        rotate_direction(&precession_matrix_transpose(t), coord.lon(), coord.lat())
    }

    /// Applies a 3x3 rotation to the unit vector of `(lon, lat)` and returns the
    /// direction in the target frame. The result is a unit vector, so the
    /// longitude and latitude are recovered directly.
    pub(crate) fn rotate_direction<T: Frame>(m: &Matrix3, lon: f64, lat: f64) -> SkyCoord<T> {
        let (sl, cl) = lon.sin_cos();
        let (sb, cb) = lat.sin_cos();
        let v = mat_vec(m, [cb * cl, cb * sl, sb]);
        let out_lon = v[1].atan2(v[0]);
        let out_lat = v[2].clamp(-1.0, 1.0).asin();
        SkyCoord::from_radians(out_lon, out_lat)
    }
}

#[cfg(feature = "std")]
pub use matrix_impl::{
    Matrix3, MeanEclipticOfDate, MeanEquatorOfDate, icrs_to_mean_ecliptic_of_date,
    icrs_to_mean_equator_of_date, mean_ecliptic_of_date_to_icrs, mean_equator_of_date_to_icrs,
    precession_matrix, precession_matrix_transpose, transpose,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Analytic rate check: d(zeta_A)/dt at t = 0 is the linear coefficient.
    fn central_difference(f: fn(f64) -> f64, h: f64) -> f64 {
        (f(h) - f(-h)) / (2.0 * h)
    }

    #[test]
    fn zeta_at_j2000_is_offset_2_650545_arcsec() {
        assert_eq!(zeta_a_arcsec(0.0), 2.650_545);
    }

    #[test]
    fn z_at_j2000_is_offset_minus_2_650545_arcsec() {
        assert_eq!(z_a_arcsec(0.0), -2.650_545);
    }

    #[test]
    fn zeta_and_z_offsets_are_equal_and_opposite_at_j2000() {
        // The two constant offsets cancel, which is what makes P(0) the identity.
        assert_eq!(zeta_a_arcsec(0.0) + z_a_arcsec(0.0), 0.0);
    }

    #[cfg(feature = "std")]
    #[test]
    fn precession_matrix_is_identity_at_j2000() {
        let p = super::matrix_impl::precession_matrix(0.0);
        for (i, row) in p.iter().enumerate() {
            for (j, &v) in row.iter().enumerate() {
                let delta = if i == j { 1.0 } else { 0.0 };
                assert!((v - delta).abs() < 1e-15, "P[{i}][{j}] = {v}");
            }
        }
    }

    #[cfg(feature = "std")]
    #[test]
    fn precession_matrix_is_orthogonal_and_transpose_inverts_it() {
        use super::matrix_impl::{precession_matrix, precession_matrix_transpose};
        for &t in &[-2.0, -0.5, 0.0, 0.7, 1.0, 2.0] {
            let p = precession_matrix(t);
            let q = precession_matrix_transpose(t);
            for (i, p_row) in p.iter().enumerate() {
                for (j, _) in q.iter().enumerate() {
                    let pq: f64 = p_row.iter().zip(&q).map(|(a, q_row)| a * q_row[j]).sum();
                    let expected = if i == j { 1.0 } else { 0.0 };
                    assert!((pq - expected).abs() < 1e-14, "t={t} PP^T[{i}][{j}]={pq}");
                }
            }
        }
    }

    #[test]
    fn theta_at_j2000_is_zero() {
        assert_eq!(theta_a_arcsec(0.0), 0.0);
    }

    #[test]
    fn zeta_rate_at_j2000_is_2306_083227_arcsec_per_century() {
        let rate = central_difference(zeta_a_arcsec, 1e-4);
        assert!((rate - 2_306.083_227).abs() < 1e-6, "rate={rate}");
    }

    #[test]
    fn z_rate_at_j2000_is_2306_077181_arcsec_per_century() {
        let rate = central_difference(z_a_arcsec, 1e-4);
        assert!((rate - 2_306.077_181).abs() < 1e-6, "rate={rate}");
    }

    #[test]
    fn theta_rate_at_j2000_is_2004_191903_arcsec_per_century() {
        let rate = central_difference(theta_a_arcsec, 1e-4);
        assert!((rate - 2_004.191_903).abs() < 1e-6, "rate={rate}");
    }

    #[test]
    fn angles_at_one_century_match_hand_evaluation() {
        // Hand evaluation of the full polynomials at t = 1.
        let zeta = 2.650_545 + 2_306.083_227 + 0.298_849_9 + 0.018_018_28
            - 0.000_005_971
            - 0.000_000_317_3;
        let z = -2.650_545 + 2_306.077_181 + 1.092_734_8 + 0.018_268_37
            - 0.000_028_596
            - 0.000_000_290_4;
        let theta = 2_004.191_903 - 0.429_493_4 - 0.041_822_64 - 0.000_007_089 - 0.000_000_127_4;
        assert!((zeta_a_arcsec(1.0) - zeta).abs() < 1e-12);
        assert!((z_a_arcsec(1.0) - z).abs() < 1e-12);
        assert!((theta_a_arcsec(1.0) - theta).abs() < 1e-12);
    }
}

#[cfg(all(test, feature = "std"))]
mod ecliptic_of_date_tests {
    use super::*;
    use crate::frames::{Icrs, SkyCoord};
    use crate::time::J2000_JD;

    #[test]
    fn ecliptic_pole_moves_by_planetary_precession() {
        // The north pole of the mean ecliptic of date, expressed in ICRS, is
        // separated from the J2000 ecliptic pole by the planetary precession
        // pi_A, about 47.0 arcseconds per century (Lieske 1977).
        let pole_j2000 = mean_ecliptic_of_date_to_icrs(
            SkyCoord::<MeanEclipticOfDate>::from_radians(0.0, core::f64::consts::FRAC_PI_2),
            J2000_JD,
            0.0,
        );
        let t1 = J2000_JD + 36_525.0;
        let pole_date = mean_ecliptic_of_date_to_icrs(
            SkyCoord::<MeanEclipticOfDate>::from_radians(0.0, core::f64::consts::FRAC_PI_2),
            t1,
            0.0,
        );
        let v1 = pole_j2000.to_unit_vector();
        let v2 = pole_date.to_unit_vector();
        let dot = (v1[0] * v2[0] + v1[1] * v2[1] + v1[2] * v2[2]).clamp(-1.0, 1.0);
        let sep_arcsec = dot.acos().to_degrees() * 3600.0;
        assert!((sep_arcsec - 47.0).abs() < 0.2, "sep {sep_arcsec}");

        // Round trip through the date frame returns the ICRS direction.
        let star = SkyCoord::<Icrs>::from_degrees(10.0, 20.0);
        let rt =
            mean_ecliptic_of_date_to_icrs(icrs_to_mean_ecliptic_of_date(star, t1, 0.0), t1, 0.0);
        assert!((rt.lon() - star.lon()).abs() < 1e-12);
        assert!((rt.lat() - star.lat()).abs() < 1e-12);
    }
}
