//! Body orientation from binary PCK kernels: the rotation from J2000 to a body
//! frame such as ITRF93, from the Euler angles stored in type-2 segments.
//!
//! The angles `(a1, a2, a3)` are a 3-1-3 rotation: the matrix that takes J2000
//! axes to the body frame is `R3(a3) R1(a2) R3(a1)`, where `Rk(t)` is the
//! passive rotation about axis `k` by `t`, as in SPICE's `rotate` and `pxform`.
//! This is the convention SPICE uses in `m2eul(M, 3, 1, 3)`; the module tests
//! check the matrix against SPICE's `pxform` (see the validation suite).

use crate::ephemeris::{EphemerisError, SpkFile};

/// NAIF reference frame code of J2000 (the equator and equinox).
pub const J2000: i32 = 1;

/// NAIF reference frame code of ECLIPJ2000 (the J2000 ecliptic), the reference
/// frame of the Earth orientation kernels.
pub const ECLIPJ2000: i32 = 17;

/// Obliquity of the J2000 ecliptic in arcseconds, as SPICE's ECLIPJ2000 frame uses
/// it (84381.448 arcseconds; the IAU 2006 value 84381.406 differs by 0.042).
pub const ECLIPJ2000_OBLIQUITY_ARCSEC: f64 = 84_381.448;

/// NAIF frame id of ITRF93, the Earth-fixed frame of binary Earth orientation kernels.
pub const ITRF93: i32 = 3000;

/// Passive rotation about the z axis by `t` radians.
fn rot3(t: f64) -> [[f64; 3]; 3] {
    let (s, c) = t.sin_cos();
    [[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]]
}

/// Passive rotation about the x axis by `t` radians.
fn rot1(t: f64) -> [[f64; 3]; 3] {
    let (s, c) = t.sin_cos();
    [[1.0, 0.0, 0.0], [0.0, c, s], [0.0, -s, c]]
}

fn mat_mul(a: [[f64; 3]; 3], b: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

/// The J2000-to-body rotation matrix for 3-1-3 Euler angles `(a1, a2, a3)`:
/// `R3(a3) R1(a2) R3(a1)`.
#[must_use]
pub fn euler_matrix(angles: [f64; 3]) -> [[f64; 3]; 3] {
    mat_mul(rot3(angles[2]), mat_mul(rot1(angles[1]), rot3(angles[0])))
}

/// The J2000-to-`body_frame` rotation matrix at `et` (TDB seconds past J2000),
/// from a binary PCK in `spk`. `body_frame` is the body-fixed frame id, such as
/// [`ITRF93`].
///
/// The segment's Euler angles relate the body frame to its reference frame. For
/// ECLIPJ2000 (the Earth kernels) the J2000 equator is reached by one more rotation
/// about the x axis through the ecliptic obliquity:
/// `M = R3(a3) R1(a2) R3(a1) R1(obliquity)`. This matches SPICE's `pxform` to
/// about 1e-15 at the epochs checked (see the validation suite).
///
/// # Errors
///
/// [`EphemerisError::UnsupportedReference`] for a reference frame other than J2000 or
/// ECLIPJ2000; otherwise the errors of [`SpkFile::orientation_angles`].
pub fn orientation_matrix(
    spk: &SpkFile<'_>,
    body_frame: i32,
    et: f64,
) -> Result<[[f64; 3]; 3], EphemerisError> {
    let (angles, reference) = spk.orientation_angles(body_frame, et)?;
    let body = euler_matrix(angles);
    match reference {
        J2000 => Ok(body),
        ECLIPJ2000 => {
            let eps = ECLIPJ2000_OBLIQUITY_ARCSEC * core::f64::consts::PI / (180.0 * 3600.0);
            Ok(mat_mul(body, rot1(eps)))
        }
        other => Err(EphemerisError::UnsupportedReference(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euler_matrix_is_a_rotation() {
        let m = euler_matrix([0.7, -0.3, 2.1]);
        // Orthogonal with determinant one.
        for i in 0..3 {
            for j in 0..3 {
                let dot: f64 = (0..3).map(|k| m[i][k] * m[j][k]).sum();
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((dot - want).abs() < 1e-14);
            }
        }
        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        assert!((det - 1.0).abs() < 1e-14);
    }

    #[test]
    fn euler_matrix_matches_spice_m2eul_example() {
        // SPICE's m2eul(pxform(J2000, ITRF93)) at 2017-01-01 00:00 TDB gives
        // (a1, a2, a3) = (1.5417469957812906, 0.0016386441372813873, 0.2094035130047053)
        // for M = R3(a3) R1(a2) R3(a1). The first row of that matrix:
        let m = euler_matrix([
            1.541_746_995_781_290_6,
            0.001_638_644_137_281_387_3,
            0.209_403_513_004_705_3,
        ]);
        assert!((m[0][0] - (-1.793_777_43e-1)).abs() < 1e-8);
        assert!((m[0][1] - 9.837_802_14e-1).abs() < 1e-8);
        assert!((m[2][2] - 9.999_986_57e-1).abs() < 1e-8);
    }

    #[test]
    fn ecliptic_reference_adds_the_obliquity_rotation() {
        // The BPC angles at 2017-01-01 (from the segment record) with the ECLIPJ2000
        // rotation reproduce SPICE's pxform(J2000, ITRF93) to better than 1e-14.
        let angles = [
            3.137_474_456_245_484_6,
            0.409_048_303_822_866_5,
            4.896_521_571_860_712_5,
        ];
        let eps = ECLIPJ2000_OBLIQUITY_ARCSEC * core::f64::consts::PI / (180.0 * 3600.0);
        let m = mat_mul(euler_matrix(angles), rot1(eps));
        let spice_row0 = [-1.793_777_43e-1, 9.837_802_14e-1, 3.406_354_23e-4];
        for k in 0..3 {
            // SPICE printed 9 significant digits here, so the check is at 1e-9.
            assert!((m[0][k] - spice_row0[k]).abs() < 1e-9, "column {k}");
        }
    }
}
