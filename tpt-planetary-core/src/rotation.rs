//! Rotations as unit quaternions, built on `glam`.
//!
//! [`Quaternion`] is `glam::DQuat` (f64). The helpers here fix the conventions
//! used across the crate: Euler angles are yaw (about Z), pitch (about Y) and
//! roll (about X), applied intrinsically in that order (ZYX). A positive
//! rotation follows the right-hand rule.
//!
//! ```
//! use glam::f64::DVec3;
//! use tpt_planetary_core::angle::Radians;
//! use tpt_planetary_core::rotation::{from_axis_angle, Quaternion};
//!
//! // A quarter turn about Z takes X to Y.
//! let q: Quaternion = from_axis_angle(DVec3::Z, Radians(core::f64::consts::FRAC_PI_2))?;
//! let v = q * DVec3::X;
//! assert!((v - DVec3::Y).length() < 1e-12);
//! # Ok::<(), tpt_planetary_core::Error>(())
//! ```

use glam::EulerRot;
use glam::f64::{DQuat, DVec3};

use crate::angle::Radians;
use crate::error::Error;

/// A rotation quaternion. Unit quaternions represent rotations.
pub type Quaternion = DQuat;

/// Builds a rotation from intrinsic yaw (Z), pitch (Y) and roll (X) angles.
///
/// ```
/// use glam::f64::DVec3;
/// use tpt_planetary_core::angle::Radians;
/// use tpt_planetary_core::rotation::from_euler_zyx;
///
/// let q = from_euler_zyx(Radians(core::f64::consts::FRAC_PI_2), Radians(0.0), Radians(0.0));
/// assert!((q * DVec3::X - DVec3::Y).length() < 1e-12);
/// ```
pub fn from_euler_zyx(yaw: Radians, pitch: Radians, roll: Radians) -> Quaternion {
    DQuat::from_euler(EulerRot::ZYX, yaw.0, pitch.0, roll.0)
}

/// Decomposes a rotation into `(yaw, pitch, roll)` using the ZYX convention.
///
/// Pitch is in `[-pi/2, pi/2]`. At pitch = +-pi/2 (gimbal lock) yaw and roll
/// are not unique, so only their combination is determined.
///
/// ```
/// use tpt_planetary_core::angle::Radians;
/// use tpt_planetary_core::rotation::{from_euler_zyx, to_euler_zyx};
///
/// let q = from_euler_zyx(Radians(0.3), Radians(-0.2), Radians(0.1));
/// let (yaw, pitch, roll) = to_euler_zyx(q);
/// assert!((yaw.0 - 0.3).abs() < 1e-12);
/// assert!((pitch.0 + 0.2).abs() < 1e-12);
/// assert!((roll.0 - 0.1).abs() < 1e-12);
/// ```
pub fn to_euler_zyx(q: Quaternion) -> (Radians, Radians, Radians) {
    let (yaw, pitch, roll) = q.to_euler(EulerRot::ZYX);
    (Radians(yaw), Radians(pitch), Radians(roll))
}

/// Builds a rotation of `angle` about `axis`.
///
/// `axis` is normalised first.
///
/// # Errors
///
/// [`Error::InvalidRotation`] if `axis` is zero, or is not finite.
///
/// ```
/// use glam::f64::DVec3;
/// use tpt_planetary_core::angle::Radians;
/// use tpt_planetary_core::rotation::from_axis_angle;
///
/// let q = from_axis_angle(DVec3::new(0.0, 0.0, 4.0), Radians(core::f64::consts::PI))?;
/// assert!((q * DVec3::X + DVec3::X).length() < 1e-12);
/// # Ok::<(), tpt_planetary_core::Error>(())
/// ```
pub fn from_axis_angle(axis: DVec3, angle: Radians) -> Result<Quaternion, Error> {
    if !axis.is_finite() || axis.length_squared() == 0.0 {
        return Err(Error::InvalidRotation);
    }
    Ok(DQuat::from_axis_angle(axis.normalize(), angle.0))
}

/// Decomposes a rotation into a unit axis and an angle in `[0, pi]`.
///
/// ```
/// use glam::f64::DVec3;
/// use tpt_planetary_core::angle::Radians;
/// use tpt_planetary_core::rotation::{from_axis_angle, to_axis_angle};
///
/// let q = from_axis_angle(DVec3::Y, Radians(0.7))?;
/// let (axis, angle) = to_axis_angle(q);
/// assert!((axis - DVec3::Y).length() < 1e-12);
/// assert!((angle.0 - 0.7).abs() < 1e-12);
/// # Ok::<(), tpt_planetary_core::Error>(())
/// ```
pub fn to_axis_angle(q: Quaternion) -> (DVec3, Radians) {
    let (axis, angle) = q.to_axis_angle();
    (axis, Radians(angle))
}

/// Spherical linear interpolation between two rotations along the shortest arc.
///
/// `t = 0` returns `a` and `t = 1` returns `b`. The inputs should be unit
/// quaternions.
///
/// ```
/// use glam::f64::DVec3;
/// use tpt_planetary_core::angle::Radians;
/// use tpt_planetary_core::rotation::{from_axis_angle, slerp};
///
/// let a = from_axis_angle(DVec3::Z, Radians(0.0))?;
/// let b = from_axis_angle(DVec3::Z, Radians(core::f64::consts::FRAC_PI_2))?;
/// let mid = slerp(a, b, 0.5);
/// assert!((mid * DVec3::X - DVec3::new(0.5_f64.sqrt(), 0.5_f64.sqrt(), 0.0)).length() < 1e-12);
/// # Ok::<(), tpt_planetary_core::Error>(())
/// ```
pub fn slerp(a: Quaternion, b: Quaternion, t: f64) -> Quaternion {
    a.slerp(b, t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f64::consts::{FRAC_PI_2, PI};

    #[test]
    fn principal_axis_rotations() {
        let rz = from_axis_angle(DVec3::Z, Radians(FRAC_PI_2)).unwrap();
        assert!((rz * DVec3::X - DVec3::Y).length() < 1e-12);
        let rx = from_axis_angle(DVec3::X, Radians(FRAC_PI_2)).unwrap();
        assert!((rx * DVec3::Y - DVec3::Z).length() < 1e-12);
        let ry = from_axis_angle(DVec3::Y, Radians(FRAC_PI_2)).unwrap();
        assert!((ry * DVec3::Z - DVec3::X).length() < 1e-12);
    }

    #[test]
    fn euler_matches_composed_axis_rotations() {
        // Intrinsic ZYX: yaw about Z first, then pitch about the new Y, then roll about the new X.
        let (y, p, r) = (0.4, -0.3, 0.2);
        let q = from_euler_zyx(Radians(y), Radians(p), Radians(r));
        let rz = from_axis_angle(DVec3::Z, Radians(y)).unwrap();
        let ry = from_axis_angle(DVec3::Y, Radians(p)).unwrap();
        let rx = from_axis_angle(DVec3::X, Radians(r)).unwrap();
        let composed = rz * ry * rx;
        let v = DVec3::new(0.3, -0.7, 0.64);
        assert!((q * v - composed * v).length() < 1e-12);
    }

    #[test]
    fn euler_round_trip_table() {
        let table = [
            (0.0, 0.0, 0.0),
            (0.3, -0.2, 0.1),
            (-2.5, 1.2, 2.0),
            (3.0, 0.0, -1.0),
        ];
        for (y, p, r) in table {
            let q = from_euler_zyx(Radians(y), Radians(p), Radians(r));
            let (y2, p2, r2) = to_euler_zyx(q);
            // Compare the rotations, not the angles, because decompositions are not unique.
            let q2 = from_euler_zyx(y2, p2, r2);
            assert!(q.dot(q2).abs() > 1.0 - 1e-12, "({y}, {p}, {r})");
        }
    }

    #[test]
    fn axis_angle_round_trip_and_validation() {
        let axis = DVec3::new(1.0, 2.0, 2.0).normalize();
        let q = from_axis_angle(axis, Radians(1.1)).unwrap();
        let (a2, ang) = to_axis_angle(q);
        assert!((a2 - axis).length() < 1e-12);
        assert!((ang.0 - 1.1).abs() < 1e-12);
        assert_eq!(
            from_axis_angle(DVec3::ZERO, Radians(1.0)),
            Err(Error::InvalidRotation)
        );
        assert_eq!(
            from_axis_angle(DVec3::new(f64::NAN, 0.0, 1.0), Radians(1.0)),
            Err(Error::InvalidRotation)
        );
    }

    #[test]
    fn slerp_endpoints_midpoint_and_shortest_path() {
        let a = from_axis_angle(DVec3::Z, Radians(0.0)).unwrap();
        let b = from_axis_angle(DVec3::Z, Radians(PI / 2.0)).unwrap();
        assert!((slerp(a, b, 0.0) * DVec3::X - DVec3::X).length() < 1e-12);
        assert!((slerp(a, b, 1.0) * DVec3::X - DVec3::Y).length() < 1e-12);
        let mid = slerp(a, b, 0.5);
        let expected = from_axis_angle(DVec3::Z, Radians(PI / 4.0)).unwrap();
        assert!(mid.dot(expected).abs() > 1.0 - 1e-12);
        // Shortest path: interpolating towards -q (same rotation) must not take the long way.
        let neg_b = DQuat::from_xyzw(-b.x, -b.y, -b.z, -b.w);
        let mid2 = slerp(a, neg_b, 0.5);
        assert!(mid2.dot(expected).abs() > 1.0 - 1e-12);
    }
}
