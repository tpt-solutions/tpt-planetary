//! CIP coordinates, CIO locator, TIO locator and the CIO-based celestial to
//! terrestrial chain (IAU 2006/2000A, IERS Conventions (2010), Chapter 5).
//!
//! # Series
//!
//! The coordinates of the Celestial Intermediate Pole (CIP) in the GCRS, X and
//! Y, are the IAU 2006/2000A series of Tables 5.2a and 5.2b (Eq. 5.16):
//!
//! `X = poly_X(t) + sum_j t^j sum_i [a_s,j sin(ARG) + a_c,j cos(ARG)]`
//!
//! and likewise for Y with `b_s, b_c`. The CIO locator `s` is recovered from
//! the quantity `s + XY/2` of Table 5.2d (Eq. 5.9 and the text of Section
//! 5.5.6): `s = (s + XY/2) - XY/2`. The tables are generated into
//! `cio_data.rs` by `tools/gen_cio_table.py`. Polynomial parts are the literal
//! coefficients printed with each table, in microarcseconds, with `t` in Julian
//! centuries of TT from J2000 (Section 5.5.6, Eq. 5.16). The fundamental
//! arguments are [`crate::nutation::fundamental_arguments`] (IERS Conventions
//! 2003, Eqs. 5.43 and 5.44), as the tables specify.
//!
//! The tables already include the frame bias at J2000.0 (the constant
//! `-16617` microarcseconds in X, Eq. 5.17), so the CIP coordinates returned
//! here are in the GCRS, whose axes are those of the ICRS. [`frame_bias_matrix`]
//! is the separate ICRS to mean-equator-and-equinox-of-J2000.0 rotation. It is
//! not used by the chain below.
//!
//! The TIO locator `s'` is the secular term `-47 microarcseconds * t`
//! (Eq. 5.13). The EO complementary terms are the non-polynomial part of
//! Table 5.2e, returned by [`eo_complementary_terms`].
//!
//! # Matrix convention
//!
//! Rotations are passive, as in [`crate::frames`]: `R1(a)`, `R2(a)` and `R3(a)`
//! are the coordinate-frame rotations `[[1,0,0],[0,c,s],[0,-s,c]]`,
//! `[[c,0,-s],[0,1,0],[s,0,c]]` and `[[c,s,0],[-s,c,0],[0,0,1]]`.
//!
//! Every matrix in this module maps a column vector from its input frame to
//! its output frame: `v_out = M v_in`. The chain is
//!
//! `v_ITRS = tirs_to_itrs(xp, yp, s') * R3(ERA) * icrs_to_cirs(t) * v_GCRS`
//!
//! where `icrs_to_cirs = R3(-(E + s)) R2(d) R3(E)` with `X = sin d cos E`,
//! `Y = sin d sin E` (IERS Conventions 2010, Eq. 5.6, written in the passive
//! form; the chapter's `Q(t)` is its transpose), and `tirs_to_itrs` is the
//! transpose of the polar-motion matrix `R3(-s') R2(xp) R1(yp)`, Eq. 5.3.
//! [`icrs_to_itrs`] returns that product. Given ERFA's own X, Y, s, ERA and
//! s', it reproduces ERFA's `c2t06a` matrix (GCRS to ITRS) to 2e-18. With
//! our X, Y, s from the tables, the difference from `c2t06a` is 3.6
//! microarcseconds, the size of the disagreement between ERFA's own X, Y
//! routes. See `docs/validation.md`.
//!
//! # Feature flags
//!
//! The series use sine and cosine, so this module needs `std`. It is gated like
//! [`crate::frames`].

use crate::cio_data::{EQ_BLOCKS, SXY_BLOCKS, X_BLOCKS, Y_BLOCKS};
use crate::frames::polar_motion_matrix;
use crate::nutation::{N_FUNDAMENTAL, RAD_PER_MICROARCSEC, Term, fundamental_arguments};
use crate::time::{DAYS_PER_CENTURY, J2000_JD, earth_rotation_angle};

/// A 3x3 matrix, row-major.
pub type Matrix3 = [[f64; 3]; 3];

/// Radians per arcsecond.
const RAD_PER_ARCSEC: f64 = core::f64::consts::PI / (180.0 * 3600.0);

/// Polynomial part of X (Table 5.2a), microarcseconds, coefficients of
/// `t^0 .. t^5`.
const X_POLY_UAS: [f64; 6] = [
    -16_617.0,
    2_004_191_898.0,
    -429_782.9,
    -198_618.34,
    7.578,
    5.9285,
];

/// Polynomial part of Y (Table 5.2b), microarcseconds.
const Y_POLY_UAS: [f64; 6] = [
    -6_951.0,
    -25_896.0,
    -22_407_274.7,
    1900.59,
    1112.526,
    0.1358,
];

/// Polynomial part of `s + XY/2` (Table 5.2d), microarcseconds.
const SXY_POLY_UAS: [f64; 6] = [94.0, 3808.65, -122.68, -72_574.11, 27.98, 15.62];

/// Frame bias: pole offset `xi0` at J2000.0, arcseconds (IERS 2010, Eqs. 5.17
/// and 5.33: `-16617.0` microarcseconds, or `-0.0166170` arcseconds).
const XI0_ARCSEC: f64 = -0.016_617_0;

/// Frame bias: pole offset `eta0` at J2000.0, arcseconds (Eq. 5.17: `-6819.2`
/// microarcseconds).
const ETA0_ARCSEC: f64 = -0.006_819_2;

/// Frame bias: right ascension of the mean equinox of J2000.0 in the GCRS,
/// `d_alpha0`, arcseconds (Eq. 5.21: `-0.01460` arcseconds).
const DALPHA0_ARCSEC: f64 = -0.014_60;

/// TIO locator rate, microarcseconds per Julian century (Eq. 5.13: `s' = -47 t`).
const TIO_RATE_UAS: f64 = -47.0;

/// Julian centuries of TT since J2000.0 from a two-part Julian date.
fn centuries(tt_hi: f64, tt_lo: f64) -> f64 {
    ((tt_hi - J2000_JD) + tt_lo) / DAYS_PER_CENTURY
}

/// Evaluates the polynomial `c[0] + c[1] t + ...` by Horner's rule.
fn poly(coeffs: &[f64; 6], t: f64) -> f64 {
    coeffs.iter().rev().fold(0.0, |acc, &c| acc * t + c)
}

/// Sum of `t^j * sum_i [sin_i sin(ARG_i) + cos_i cos(ARG_i)]` over the j-blocks,
/// in microarcseconds.
fn series_uas(blocks: &[&[Term]], f: &[f64; N_FUNDAMENTAL], t: f64) -> f64 {
    let mut total = 0.0;
    let mut tj = 1.0;
    for block in blocks {
        let mut sum = 0.0;
        for term in block.iter() {
            let arg: f64 = term
                .args
                .iter()
                .zip(f.iter())
                .map(|(&n, &fi)| f64::from(n) * fi)
                .sum();
            let (s, c) = arg.sin_cos();
            sum += term.sin * s + term.cos * c;
        }
        total += tj * sum;
        tj *= t;
    }
    total
}

/// Passive rotation about the x-axis.
fn rot_x(theta: f64) -> Matrix3 {
    let (s, c) = theta.sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, s], [0.0, -s, c]]
}

/// Passive rotation about the y-axis.
fn rot_y(theta: f64) -> Matrix3 {
    let (s, c) = theta.sin_cos();
    [[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]]
}

/// Passive rotation about the z-axis.
fn rot_z(theta: f64) -> Matrix3 {
    let (s, c) = theta.sin_cos();
    [[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]]
}

/// Matrix product `a * b`.
fn mat_mul(a: &Matrix3, b: &Matrix3) -> Matrix3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

/// Transpose of a 3x3 matrix.
fn transpose(m: &Matrix3) -> Matrix3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = m[j][i];
        }
    }
    out
}

/// Coordinates `(X, Y)` of the CIP in the GCRS at TT `(tt_hi, tt_lo)`, radians
/// (dimensionless direction cosines, as in Eq. 5.7).
///
/// The series is IAU 2006/2000A (Table 5.2a for X, 5.2b for Y). Values
/// include the frame bias at J2000.0.
///
/// ```
/// use tpt_planetary_astro::cio::cip_xy;
/// let (x, y) = cip_xy(2_451_545.0, 0.0);
/// // X and Y are a few arcseconds at J2000.0.
/// assert!((x * 206_264.806).abs() < 20.0 && (y * 206_264.806).abs() < 20.0);
/// ```
#[must_use]
pub fn cip_xy(tt_hi: f64, tt_lo: f64) -> (f64, f64) {
    let t = centuries(tt_hi, tt_lo);
    let f = fundamental_arguments(t);
    let x = (poly(&X_POLY_UAS, t) + series_uas(&X_BLOCKS, &f, t)) * RAD_PER_MICROARCSEC;
    let y = (poly(&Y_POLY_UAS, t) + series_uas(&Y_BLOCKS, &f, t)) * RAD_PER_MICROARCSEC;
    (x, y)
}

/// CIO locator `s` at TT `(tt_hi, tt_lo)`, radians.
///
/// `s = (s + XY/2) - XY/2`, where `s + XY/2` is Table 5.2d (IAU 2006/2000A,
/// Eq. 5.9), and `X`, `Y` are from [`cip_xy`]. The constant term is the value
/// fixed for continuity of UT1 at 2003 January 1 (Section 5.5.6). The
/// quantity `s` is a few tens of microarcseconds per century.
///
/// ```
/// use tpt_planetary_astro::cio::cio_locator;
/// // s is a few milliarcseconds at J2000.0.
/// assert!((cio_locator(2_451_545.0, 0.0) * 206_264.806).abs() < 0.01);
/// ```
#[must_use]
pub fn cio_locator(tt_hi: f64, tt_lo: f64) -> f64 {
    let t = centuries(tt_hi, tt_lo);
    let f = fundamental_arguments(t);
    let sxy = (poly(&SXY_POLY_UAS, t) + series_uas(&SXY_BLOCKS, &f, t)) * RAD_PER_MICROARCSEC;
    let (x, y) = cip_xy(tt_hi, tt_lo);
    sxy - 0.5 * x * y
}

/// TIO locator `s'` at TT `(tt_hi, tt_lo)`, radians.
///
/// `s' = -47 microarcseconds * t` (IERS 2010, Eq. 5.13), the value the chapter
/// adopts for the current wobble amplitudes. The general form (Eqs. 5.4 and
/// 5.12) depends on the history of the polar motion, which this module does not
/// take as input. The chapter bounds the resulting `s` at 0.4 mas per century.
#[must_use]
pub fn tio_locator(tt_hi: f64, tt_lo: f64) -> f64 {
    TIO_RATE_UAS * centuries(tt_hi, tt_lo) * RAD_PER_MICROARCSEC
}

/// Complementary terms of the equation of the origins, radians: the
/// non-polynomial part of Table 5.2e (`C'_s`, `C'_c`, j = 0 and j = 1).
///
/// In the ERA-based GST expression (Table 5.2e, Section 5.5.7),
/// `GST = ERA + polynomial + dpsi cos(eps0) + eo_complementary_terms`.
/// The polynomial and `dpsi cos(eps0)` parts are not computed here. The
/// `dpsi cos(eps0)` part is [`crate::nutation::equation_of_equinoxes`]. The
/// CIO-based chain in this module does not use these terms.
#[must_use]
pub fn eo_complementary_terms(tt_hi: f64, tt_lo: f64) -> f64 {
    let t = centuries(tt_hi, tt_lo);
    let f = fundamental_arguments(t);
    series_uas(&EQ_BLOCKS, &f, t) * RAD_PER_MICROARCSEC
}

/// Frame bias matrix: rotation from the ICRS (GCRS-aligned) axes to the mean
/// equator and equinox of J2000.0, built from the IERS 2010 constants
/// `xi0`, `eta0` and `d_alpha0` (Eqs. 5.17, 5.21, 5.33).
///
/// `B = R1(-eta0) R2(xi0) R3(d_alpha0)`, passive. Not applied in the chain:
/// the CIP series already include the bias (see the module documentation).
///
/// ```
/// use tpt_planetary_astro::cio::frame_bias_matrix;
/// let b = frame_bias_matrix();
/// // Nearly the identity: off-diagonal terms of order 1e-8 rad.
/// assert!((b[0][0] - 1.0).abs() < 1e-12);
/// ```
#[must_use]
pub fn frame_bias_matrix() -> Matrix3 {
    let xi0 = XI0_ARCSEC * RAD_PER_ARCSEC;
    let eta0 = ETA0_ARCSEC * RAD_PER_ARCSEC;
    let dalpha0 = DALPHA0_ARCSEC * RAD_PER_ARCSEC;
    mat_mul(&rot_x(-eta0), &mat_mul(&rot_y(xi0), &rot_z(dalpha0)))
}

/// Matrix from ICRS (GCRS-aligned) to CIRS at TT `(tt_hi, tt_lo)`:
/// `Q^T = R3(-(E + s)) R2(d) R3(E)` with `E = atan2(Y, X)` and
/// `d = atan(sqrt(X^2 + Y^2) / sqrt(1 - X^2 - Y^2))`.
///
/// Passive form of IERS 2010 Eq. (5.6), `Q = R3(-E) R2(-d) R3(E) R3(s)`. The
/// chapter's `Q` is the transpose of this matrix.
#[must_use]
pub fn icrs_to_cirs(tt_hi: f64, tt_lo: f64) -> Matrix3 {
    let (x, y) = cip_xy(tt_hi, tt_lo);
    icrs_to_cirs_from_xys(x, y, cio_locator(tt_hi, tt_lo))
}

/// [`icrs_to_cirs`] from given CIP coordinates `x`, `y` and CIO locator `s`
/// (radians). Lets the matrix algebra be checked separately from the series.
#[must_use]
pub fn icrs_to_cirs_from_xys(x: f64, y: f64, s: f64) -> Matrix3 {
    let e = y.atan2(x);
    let r2 = x * x + y * y;
    let d = r2.sqrt().atan2((1.0 - r2).sqrt());
    mat_mul(&rot_z(-(e + s)), &mat_mul(&rot_y(d), &rot_z(e)))
}

/// Matrix from CIRS to TIRS, the Earth rotation: `R3(ERA)`, passive (IERS 2010,
/// Eq. 5.5, transposed to the passive convention). `era` in radians.
#[must_use]
pub fn cirs_to_tirs(era: f64) -> Matrix3 {
    rot_z(era)
}

/// Matrix from TIRS to ITRS: the transpose of `R3(-s') R2(xp) R1(yp)`, the
/// polar-motion matrix of IERS 2010 Eq. (5.3) in the passive convention.
///
/// `xp`, `yp` are the CIP polar coordinates in radians and `sp` is the TIO
/// locator in radians. The ITRS to TIRS matrix is
/// [`crate::frames::polar_motion_matrix`] with the `R3(-s')` factor applied.
/// The sign of `s'` was checked against ERFA's `sp00`-based `c2t06a`.
#[must_use]
pub fn tirs_to_itrs(xp: f64, yp: f64, sp: f64) -> Matrix3 {
    transpose(&mat_mul(&rot_z(-sp), &polar_motion_matrix(xp, yp)))
}

/// Matrix from ICRS (GCRS-aligned) to ITRS: `M = tirs_to_itrs * R3(ERA) *
/// icrs_to_cirs`, with `v_ITRS = M v_ICRS`.
///
/// `tt_hi + tt_lo` is the TT Julian date, `ut1_hi + ut1_lo` the UT1 Julian date
/// (the ERA is evaluated from UT1), and `xp`, `yp` are the polar coordinates
/// of the CIP in radians. The TIO locator and CIO locator come from this
/// module. ERFA's `c2t06a` (GCRS to ITRS) differs from this matrix by 3.6
/// microarcseconds at most over 1900-2100 (see `docs/validation.md`).
///
/// ```
/// use tpt_planetary_astro::cio::icrs_to_itrs;
/// let m = icrs_to_itrs(2_451_545.0, 0.0, 2_451_545.0, 0.0, 0.0, 0.0);
/// // An orthonormal matrix: the first row has unit length.
/// let n: f64 = m[0].iter().map(|v| v * v).sum();
/// assert!((n - 1.0).abs() < 1e-12);
/// ```
#[must_use]
pub fn icrs_to_itrs(tt_hi: f64, tt_lo: f64, ut1_hi: f64, ut1_lo: f64, xp: f64, yp: f64) -> Matrix3 {
    let (x, y) = cip_xy(tt_hi, tt_lo);
    itrs_from_components(
        x,
        y,
        cio_locator(tt_hi, tt_lo),
        earth_rotation_angle(ut1_hi, ut1_lo),
        tio_locator(tt_hi, tt_lo),
        xp,
        yp,
    )
}

/// The chain of [`icrs_to_itrs`] from its five components, all in radians:
/// CIP `x`, `y`, CIO locator `s`, Earth rotation angle `era`, TIO locator `sp`,
/// and polar coordinates `xp`, `yp`.
#[must_use]
pub fn itrs_from_components(
    x: f64,
    y: f64,
    s: f64,
    era: f64,
    sp: f64,
    xp: f64,
    yp: f64,
) -> Matrix3 {
    let q = icrs_to_cirs_from_xys(x, y, s);
    let r = cirs_to_tirs(era);
    let w = tirs_to_itrs(xp, yp, sp);
    mat_mul(&w, &mat_mul(&r, &q))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn max_dev_from_identity(m: &Matrix3) -> f64 {
        let mut worst: f64 = 0.0;
        for i in 0..3 {
            for j in 0..3 {
                let p: f64 = (0..3).map(|k| m[i][k] * m[j][k]).sum();
                let target = if i == j { 1.0 } else { 0.0 };
                worst = worst.max((p - target).abs());
            }
        }
        worst
    }

    #[test]
    fn block_sizes_match_the_tables() {
        assert_eq!(
            X_BLOCKS.iter().map(|b| b.len()).collect::<Vec<_>>(),
            [1306, 253, 36, 4, 1]
        );
        assert_eq!(
            Y_BLOCKS.iter().map(|b| b.len()).collect::<Vec<_>>(),
            [962, 277, 30, 5, 1]
        );
        assert_eq!(
            SXY_BLOCKS.iter().map(|b| b.len()).collect::<Vec<_>>(),
            [33, 3, 25, 4, 1]
        );
        assert_eq!(
            EQ_BLOCKS.iter().map(|b| b.len()).collect::<Vec<_>>(),
            [33, 1]
        );
    }

    #[test]
    fn xy_at_j2000_is_a_few_arcseconds() {
        let (x, y) = cip_xy(J2000_JD, 0.0);
        let x_as = x / RAD_PER_ARCSEC;
        let y_as = y / RAD_PER_ARCSEC;
        // Dominated by the 9.2 arcsec obliquity and 6.8 arcsec X terms of Om.
        assert!(x_as.abs() < 20.0, "X = {x_as} arcsec");
        assert!(y_as.abs() < 20.0, "Y = {y_as} arcsec");
    }

    #[test]
    fn frame_bias_is_a_small_rotation() {
        let b = frame_bias_matrix();
        assert!(max_dev_from_identity(&b) < 1e-14);
    }

    #[test]
    fn chain_is_orthonormal() {
        let m = icrs_to_itrs(2_460_000.5, 0.25, 2_460_000.5, 0.2501, 1e-6, 2e-6);
        assert!(max_dev_from_identity(&m) < 1e-12);
        let c = icrs_to_cirs(2_460_000.5, 0.25);
        assert!(max_dev_from_identity(&c) < 1e-12);
    }

    #[test]
    fn tio_locator_is_minus_47_microarcsec_per_century() {
        let s = tio_locator(J2000_JD + DAYS_PER_CENTURY, 0.0);
        assert!((s / RAD_PER_MICROARCSEC + 47.0).abs() < 1e-9);
    }
}
