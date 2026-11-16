//! IAU 2000A_R06 nutation: fundamental arguments, nutation in longitude and
//! obliquity, the nutation matrix, and the equation of the equinoxes.
//!
//! This is the full 2000A luni-solar and planetary series, with the IAU 2006
//! adjustments as published in IERS Conventions (2010), Chapter 5, Tables 5.3a
//! and 5.3b (IAU 2000A_R06). The tables are generated into `nutation_data.rs`
//! by `tools/gen_nutation_table.py` from the published files.
//!
//! # Fundamental arguments
//!
//! [`fundamental_arguments`] returns F1..F14 at `t` Julian centuries of TT from
//! J2000, in radians, in the order
//! `l, l', F, D, Om, L_Me, L_Ve, L_E, L_Ma, L_J, L_Sa, L_U, L_Ne, p_A`.
//! The polynomials are Eqs. (5.43) and (5.44) of IERS Conventions (2010),
//! Chapter 5 (sections 5.7.2 and 5.7.3). TT is used in place of TDB, as the
//! chapter recommends.
//!
//! # Series
//!
//! With `ARG = sum_j N_j F_j` (multipliers `N_j` from the tables), the
//! series are (chapter Eq. 5.35):
//!
//! - `dpsi = sum_i [A_i sin(ARG) + A"_i cos(ARG)] + t sum_i [A'_i sin(ARG) + A"'_i cos(ARG)]`
//! - `deps = sum_i [B_i cos(ARG) + B"_i sin(ARG)] + t sum_i [B'_i cos(ARG) + B"'_i sin(ARG)]`
//!
//! The `sin` and `cos` fields of [`Term`] hold the first and second amplitude
//! column of each table, which is the sin and cos coefficient in both files.
//! Amplitudes are in microarcseconds and converted to radians here.
//!
//! # Matrix convention
//!
//! The nutation matrix is
//!
//! `N(t) = R1(-(eps0 + deps)) R3(-dpsi) R1(eps0)`,
//!
//! with the passive rotations of this crate: `R1(a) = [[1,0,0],[0,c,s],[0,-s,c]]`
//! and `R3(a) = [[c,s,0],[-s,c,0],[0,0,1]]`. `N` maps the mean equatorial
//! frame of date to the true equatorial frame of date. The full
//! precession-nutation matrix is `N(t) P(t)`, which maps the mean equator of
//! J2000.0 to the true equator of date. Frame bias is not included, as in
//! [`crate::precession`].
//!
//! # Equation of the equinoxes
//!
//! [`equation_of_equinoxes`] returns `dpsi * cos(eps0)` plus the complementary
//! terms of the equation of the origins (IERS Conventions 2010, Table 5.2e).
//! Frame bias is not included.
//!
//! # Feature flags
//!
//! [`Term`], [`fundamental_arguments`] and the table data need no `std`. The
//! series sums, the matrix and the frame helpers need the trigonometric
//! functions and are gated on `std`.

#[cfg(feature = "std")]
use crate::nutation_data::{LONGITUDE_J0, LONGITUDE_J1, OBLIQUITY_J0, OBLIQUITY_J1};

/// Julian date of J2000.0 (TT).
#[cfg(feature = "std")]
const J2000_JD: f64 = 2_451_545.0;
/// Days per Julian century.
#[cfg(feature = "std")]
const DAYS_PER_CENTURY: f64 = 36_525.0;
/// Radians per arcsecond.
const RAD_PER_ARCSEC: f64 = core::f64::consts::PI / (180.0 * 3600.0);
/// Radians per microarcsecond.
pub const RAD_PER_MICROARCSEC: f64 = RAD_PER_ARCSEC * 1e-6;

/// Number of fundamental arguments: five luni-solar and nine planetary.
pub const N_FUNDAMENTAL: usize = 14;

/// One term of a nutation series.
///
/// `args` are the integer multipliers of F1..F14. `sin` and `cos` are the
/// amplitudes, in microarcseconds, of `sin(ARG)` and `cos(ARG)`. In the j=1
/// tables they multiply `t` times the same trigonometric factors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Term {
    /// Multipliers of F1..F14.
    pub args: [i8; N_FUNDAMENTAL],
    /// Amplitude of `sin(ARG)`, microarcseconds.
    pub sin: f64,
    /// Amplitude of `cos(ARG)`, microarcseconds.
    pub cos: f64,
}

/// Polynomial coefficients of F1..F5 (IERS 2010, Eq. 5.43), each row
/// `[constant, t, t^2, t^3, t^4]` in arcseconds. The constants are the
/// degree values printed in Eq. (5.43) multiplied by 3600.
const LUNISOLAR_ARGS_ARCSEC: [[f64; 5]; 5] = [
    // F1 = l: 134.96340251 deg
    [
        485_868.249_036,
        1_717_915_923.217_8,
        31.8792,
        0.051_635,
        -0.000_244_70,
    ],
    // F2 = l': 357.52910918 deg
    [
        1_287_104.793_048,
        129_596_581.048_1,
        -0.5532,
        0.000_136,
        -0.000_011_49,
    ],
    // F3 = F = L - Om: 93.27209062 deg
    [
        335_779.526_232,
        1_739_527_262.847_8,
        -12.7512,
        -0.001_037,
        0.000_004_17,
    ],
    // F4 = D: 297.85019547 deg
    [
        1_072_260.703_692,
        1_602_961_601.209_0,
        -6.3706,
        0.006_593,
        -0.000_031_69,
    ],
    // F5 = Om: 125.04455501 deg
    [
        450_160.398_036,
        -6_962_890.543_1,
        7.4722,
        0.007_702,
        -0.000_059_39,
    ],
];

/// Polynomial coefficients of F6..F13 (IERS 2010, Eq. 5.44), each row
/// `[constant, rate]` in radians with `t` in Julian centuries. Row `k` is
/// F(6+k): L_Me, L_Ve, L_E, L_Ma, L_J, L_Sa, L_U, L_Ne.
const PLANETARY_ARGS_RAD: [[f64; 2]; 8] = [
    [4.402_608_842, 2_608.790_314_157_4],
    [3.176_146_697, 1_021.328_554_621_1],
    [1.753_470_314, 628.307_584_999_1],
    [6.203_480_913, 334.061_242_670_0],
    [0.599_546_497, 52.969_096_264_1],
    [0.874_016_757, 21.329_910_496_0],
    [5.481_293_872, 7.478_159_856_7],
    [5.311_886_287, 3.813_303_563_8],
];

/// General precession in longitude p_A (IERS 2010, Eq. 5.44), radians:
/// `p_A = 0.02438175 t + 0.00000538691 t^2`.
const PA_RAD: [f64; 3] = [0.0, 0.024_381_75, 0.000_005_386_91];

/// Evaluates `c[0] + c[1] t + c[2] t^2 + ...` by Horner's rule.
fn horner(coeffs: &[f64], t: f64) -> f64 {
    coeffs.iter().rev().fold(0.0, |acc, &c| acc * t + c)
}

/// The fourteen fundamental arguments F1..F14 at `t` Julian centuries of TT
/// from J2000, in radians.
///
/// Order: `l, l', F, D, Om, L_Me, L_Ve, L_E, L_Ma, L_J, L_Sa, L_U, L_Ne, p_A`.
/// Polynomials from IERS Conventions (2010), Eqs. (5.43) and (5.44).
///
/// ```
/// use tpt_planetary_astro::nutation::fundamental_arguments;
/// let f = fundamental_arguments(0.0);
/// // At J2000 the Moon's mean anomaly is 134.96340251 degrees.
/// assert!((f[0].to_degrees() - 134.963_402_51).abs() < 1e-9);
/// ```
#[must_use]
pub fn fundamental_arguments(t: f64) -> [f64; N_FUNDAMENTAL] {
    let mut out = [0.0; N_FUNDAMENTAL];
    for (slot, coeffs) in out.iter_mut().take(5).zip(LUNISOLAR_ARGS_ARCSEC.iter()) {
        *slot = horner(coeffs, t) * RAD_PER_ARCSEC;
    }
    for (slot, coeffs) in out[5..13].iter_mut().zip(PLANETARY_ARGS_RAD.iter()) {
        *slot = horner(coeffs, t);
    }
    out[13] = horner(&PA_RAD, t);
    out
}

/// Which amplitudes of the j=1 block are kept.
///
/// The j=1 block holds `t` times the sin and cos amplitudes. Its `cos` column
/// in longitude (A"'_i) and its `sin` column in obliquity (B"'_i) are the
/// time-varying out-of-phase amplitudes; the other column is in-phase.
#[cfg(feature = "std")]
#[derive(Clone, Copy)]
struct J1Keep {
    sin: bool,
    cos: bool,
}

/// Sums one series at `t` in radians:
/// `sum_j0 [sin * sin(ARG) + cos * cos(ARG)] + t * sum_j1 [same, filtered]`,
/// with the amplitudes in microarcseconds and `args` from
/// [`fundamental_arguments`].
#[cfg(feature = "std")]
fn sum_series(j0: &[Term], j1: &[Term], args: &[f64; N_FUNDAMENTAL], t: f64, keep: J1Keep) -> f64 {
    let trig = |term: &Term| -> (f64, f64) {
        let arg: f64 = term
            .args
            .iter()
            .zip(args.iter())
            .map(|(&n, &f)| f64::from(n) * f)
            .sum();
        arg.sin_cos()
    };
    let j0_sum: f64 = j0
        .iter()
        .map(|term| {
            let (s, c) = trig(term);
            term.sin * s + term.cos * c
        })
        .sum();
    let j1_sum: f64 = j1
        .iter()
        .map(|term| {
            let (s, c) = trig(term);
            let a = if keep.sin { term.sin * s } else { 0.0 };
            let b = if keep.cos { term.cos * c } else { 0.0 };
            a + b
        })
        .sum();
    (j0_sum + t * j1_sum) * RAD_PER_MICROARCSEC
}

/// Fraction of a Julian century of TT since J2000, from a two-part Julian date.
#[cfg(feature = "std")]
fn centuries_since_j2000(tt_hi: f64, tt_lo: f64) -> f64 {
    ((tt_hi - J2000_JD) + tt_lo) / DAYS_PER_CENTURY
}

/// Nutation in longitude and obliquity `(dpsi, deps)` in radians at `t`
/// Julian centuries of TT from J2000, IAU 2000A_R06 (full luni-solar and
/// planetary series).
///
/// ```
/// use tpt_planetary_astro::nutation::nutation_angles_t;
/// let (dpsi, deps) = nutation_angles_t(0.0);
/// // Largest term is about -17.2 arcseconds in longitude.
/// assert!(dpsi.abs() < 20.0 * 4.848e-6 && dpsi.abs() > 10.0 * 4.848e-6);
/// assert!(deps.abs() < 10.0 * 4.848e-6);
/// ```
#[cfg(feature = "std")]
#[must_use]
pub fn nutation_angles_t(t: f64) -> (f64, f64) {
    let args = fundamental_arguments(t);
    let all = J1Keep {
        sin: true,
        cos: true,
    };
    let dpsi = sum_series(&LONGITUDE_J0, &LONGITUDE_J1, &args, t, all);
    let deps = sum_series(&OBLIQUITY_J0, &OBLIQUITY_J1, &args, t, all);
    (dpsi, deps)
}

/// Variant of [`nutation_angles_t`] that drops the time-varying out-of-phase
/// amplitudes (A"'_i in longitude, B"'_i in obliquity), returning `(dpsi, deps)`
/// in radians.
///
/// This is the series implemented by the MHB2000 / SOFA `nut00a` code, which
/// is what pyerfa's `nut06a` evaluates. IERS Conventions (2010), Section 5.7.2,
/// notes the omission and its effect of about 2 microarcseconds per century in
/// the CIP location. It is provided only so that the validation suite can
/// separate that known convention difference from the rest of the model. It is
/// not the IERS 2010 expression, which is [`nutation_angles_t`].
#[cfg(feature = "std")]
#[must_use]
pub fn nutation_angles_t_without_out_of_phase_rates(t: f64) -> (f64, f64) {
    let args = fundamental_arguments(t);
    let dpsi = sum_series(
        &LONGITUDE_J0,
        &LONGITUDE_J1,
        &args,
        t,
        J1Keep {
            sin: true,
            cos: false,
        },
    );
    let deps = sum_series(
        &OBLIQUITY_J0,
        &OBLIQUITY_J1,
        &args,
        t,
        J1Keep {
            sin: false,
            cos: true,
        },
    );
    (dpsi, deps)
}

/// Nutation in longitude and obliquity `(dpsi, deps)` in radians at a TT
/// two-part Julian date. See [`nutation_angles_t`].
#[cfg(feature = "std")]
#[must_use]
pub fn nutation_angles(tt_hi: f64, tt_lo: f64) -> (f64, f64) {
    nutation_angles_t(centuries_since_j2000(tt_hi, tt_lo))
}

/// Equation of the equinoxes in radians: `dpsi * cos(eps0) + C`, where `eps0`
/// is the IAU 2006 mean obliquity and `C` is the sum of the complementary terms
/// of the equation of the origins (IERS Conventions 2010, Table 5.2e; see
/// [`crate::cio::eo_complementary_terms`]). Agrees with `erfa.ee06a` to about 6 uas over
/// 1900-2100 with the published tables (the out-of-phase residual in dpsi).
#[cfg(feature = "std")]
#[must_use]
pub fn equation_of_equinoxes(tt_hi: f64, tt_lo: f64) -> f64 {
    let (dpsi, _) = nutation_angles(tt_hi, tt_lo);
    dpsi * crate::time::mean_obliquity_iau2006_rad(tt_hi, tt_lo).cos()
        + crate::cio::eo_complementary_terms(tt_hi, tt_lo)
}

#[cfg(feature = "std")]
mod matrix_impl {
    use super::{centuries_since_j2000, nutation_angles_t};
    use crate::frames::{Frame, Icrs, SkyCoord};
    use crate::precession::matrix_impl::{mat_mul, rotate_direction};
    use crate::precession::{Matrix3, MeanEquatorOfDate, precession_matrix, transpose};
    use crate::time::mean_obliquity_iau2006_rad;

    /// True equator and equinox of date: mean equator of date with nutation
    /// applied. Frame bias is not included.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct TrueEquatorOfDate;

    impl Frame for TrueEquatorOfDate {
        const NAME: &'static str = "TrueEquatorOfDate(IAU2006/2000A_R06, no frame bias)";
    }

    /// Passive rotation about the x-axis (same convention as `frames`).
    fn rot_x(theta: f64) -> Matrix3 {
        let (s, c) = theta.sin_cos();
        [[1.0, 0.0, 0.0], [0.0, c, s], [0.0, -s, c]]
    }

    /// Passive rotation about the z-axis (same convention as `frames`).
    fn rot_z(theta: f64) -> Matrix3 {
        let (s, c) = theta.sin_cos();
        [[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]]
    }

    /// Nutation matrix `N = R1(-(eps0 + deps)) R3(-dpsi) R1(eps0)` from the
    /// nutation angles and mean obliquity, all in radians.
    ///
    /// ```
    /// use tpt_planetary_astro::nutation::nutation_matrix_from_angles;
    /// let n = nutation_matrix_from_angles(0.0, 0.0, 0.4);
    /// assert!((n[0][0] - 1.0).abs() < 1e-15);
    /// ```
    #[must_use]
    pub fn nutation_matrix_from_angles(dpsi: f64, deps: f64, eps0: f64) -> Matrix3 {
        mat_mul(
            &rot_x(-(eps0 + deps)),
            &mat_mul(&rot_z(-dpsi), &rot_x(eps0)),
        )
    }

    /// Nutation matrix `N(t)` at a TT two-part Julian date. It maps the mean
    /// equator of date to the true equator of date.
    #[must_use]
    pub fn nutation_matrix(tt_hi: f64, tt_lo: f64) -> Matrix3 {
        let t = centuries_since_j2000(tt_hi, tt_lo);
        let (dpsi, deps) = nutation_angles_t(t);
        let eps0 = mean_obliquity_iau2006_rad(tt_hi, tt_lo);
        nutation_matrix_from_angles(dpsi, deps, eps0)
    }

    /// Precession-nutation matrix `N(t) P(t)`: maps the mean equator of J2000.0
    /// to the true equator of date. Frame bias is not included.
    #[must_use]
    pub fn true_equator_of_date_matrix(tt_hi: f64, tt_lo: f64) -> Matrix3 {
        let t = centuries_since_j2000(tt_hi, tt_lo);
        mat_mul(&nutation_matrix(tt_hi, tt_lo), &precession_matrix(t))
    }

    /// Converts an ICRS direction to the true equator and equinox of date at
    /// the TT two-part Julian date. The ICRS axes are taken as the J2000 mean
    /// equator, so the frame bias (about 17 mas) is not applied.
    ///
    /// ```
    /// use tpt_planetary_astro::frames::{Icrs, SkyCoord};
    /// use tpt_planetary_astro::nutation::{icrs_to_true_equator_of_date, true_equator_of_date_to_icrs};
    /// let star = SkyCoord::<Icrs>::from_degrees(83.6, 22.0);
    /// let date = icrs_to_true_equator_of_date(star, 2_460_000.5, 0.25);
    /// let back = true_equator_of_date_to_icrs(date, 2_460_000.5, 0.25);
    /// assert!((back.lon() - star.lon()).abs() < 1e-13);
    /// assert!((back.lat() - star.lat()).abs() < 1e-13);
    /// ```
    #[must_use]
    pub fn icrs_to_true_equator_of_date(
        coord: SkyCoord<Icrs>,
        tt_hi: f64,
        tt_lo: f64,
    ) -> SkyCoord<TrueEquatorOfDate> {
        rotate_direction(
            &true_equator_of_date_matrix(tt_hi, tt_lo),
            coord.lon(),
            coord.lat(),
        )
    }

    /// Inverse of [`icrs_to_true_equator_of_date`].
    #[must_use]
    pub fn true_equator_of_date_to_icrs(
        coord: SkyCoord<TrueEquatorOfDate>,
        tt_hi: f64,
        tt_lo: f64,
    ) -> SkyCoord<Icrs> {
        rotate_direction(
            &transpose(&true_equator_of_date_matrix(tt_hi, tt_lo)),
            coord.lon(),
            coord.lat(),
        )
    }

    /// Converts a direction from the mean equator of date to the true equator
    /// of date at the TT two-part Julian date (nutation only).
    #[must_use]
    pub fn mean_to_true_equator_of_date(
        coord: SkyCoord<MeanEquatorOfDate>,
        tt_hi: f64,
        tt_lo: f64,
    ) -> SkyCoord<TrueEquatorOfDate> {
        rotate_direction(&nutation_matrix(tt_hi, tt_lo), coord.lon(), coord.lat())
    }

    /// Converts a direction from the true equator of date back to the mean
    /// equator of date (inverse of nutation only).
    #[must_use]
    pub fn true_to_mean_equator_of_date(
        coord: SkyCoord<TrueEquatorOfDate>,
        tt_hi: f64,
        tt_lo: f64,
    ) -> SkyCoord<MeanEquatorOfDate> {
        rotate_direction(
            &transpose(&nutation_matrix(tt_hi, tt_lo)),
            coord.lon(),
            coord.lat(),
        )
    }
}

#[cfg(feature = "std")]
pub use matrix_impl::{
    TrueEquatorOfDate, icrs_to_true_equator_of_date, mean_to_true_equator_of_date, nutation_matrix,
    nutation_matrix_from_angles, true_equator_of_date_matrix, true_equator_of_date_to_icrs,
    true_to_mean_equator_of_date,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fundamental_arguments_at_j2000_match_constants() {
        let f = fundamental_arguments(0.0);
        // Constants of Eq. (5.43) and (5.44) at t = 0, in degrees.
        assert!((f[0].to_degrees() - 134.963_402_51).abs() < 1e-9);
        assert!((f[1].to_degrees() - 357.529_109_18).abs() < 1e-9);
        assert!((f[2].to_degrees() - 93.272_090_62).abs() < 1e-9);
        assert!((f[3].to_degrees() - 297.850_195_47).abs() < 1e-9);
        assert!((f[4].to_degrees() - 125.044_555_01).abs() < 1e-9);
        assert!((f[5] - 4.402_608_842).abs() < 1e-15);
        assert_eq!(f[13], 0.0);
    }

    #[cfg(feature = "std")]
    #[test]
    fn term_counts_match_headers() {
        assert_eq!(LONGITUDE_J0.len(), 1320);
        assert_eq!(LONGITUDE_J1.len(), 38);
        assert_eq!(OBLIQUITY_J0.len(), 1037);
        assert_eq!(OBLIQUITY_J1.len(), 19);
    }

    #[cfg(feature = "std")]
    #[test]
    fn first_longitude_term_is_the_principal_nutation() {
        assert_eq!(
            LONGITUDE_J0[0].args,
            [0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(LONGITUDE_J0[0].sin, -17_206_424.18);
        assert_eq!(LONGITUDE_J0[0].cos, 3_338.60);
    }

    #[cfg(feature = "std")]
    #[test]
    fn nutation_matrix_is_orthogonal() {
        use super::matrix_impl::nutation_matrix;
        let n = nutation_matrix(2_451_545.0, 0.3);
        for i in 0..3 {
            for j in 0..3 {
                let dot: f64 = (0..3).map(|k| n[i][k] * n[j][k]).sum();
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!((dot - expected).abs() < 1e-14);
            }
        }
    }

    #[cfg(feature = "std")]
    #[test]
    fn nutation_at_j2000_matches_validated_values() {
        // At J2000 the Moon's node is at 125.04 degrees, so the principal term
        // -17.2064 arcsec sin(Om) gives about -13.9 arcsec in longitude, and
        // +9.2052 arcsec cos(Om) plus the other terms gives about -5.8 arcsec
        // in obliquity. These are the values the validation suite compares
        // with ERFA nut06a (which agrees to the microarcsecond level).
        let (dpsi, deps) = nutation_angles_t(0.0);
        let arcsec = |r: f64| r / RAD_PER_ARCSEC;
        assert!(
            (arcsec(dpsi) + 13.932).abs() < 1e-3,
            "dpsi = {}",
            arcsec(dpsi)
        );
        assert!(
            (arcsec(deps) + 5.769).abs() < 1e-3,
            "deps = {}",
            arcsec(deps)
        );
    }
}
