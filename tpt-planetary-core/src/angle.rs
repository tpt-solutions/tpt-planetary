//! Angle newtypes and wrapping helpers.
//!
//! [`Radians`], [`Degrees`] and [`HourAngle`] are distinct types so that a
//! value in one unit cannot be passed where another is expected. Conversions
//! are explicit through [`From`] or the `to_*` methods.
//!
//! ```
//! use tpt_planetary_core::angle::{Degrees, Radians};
//!
//! let r: Radians = Degrees(180.0).into();
//! assert!((r.0 - core::f64::consts::PI).abs() < 1e-15);
//! assert!((Degrees(370.0).wrap_360().0 - 10.0).abs() < 1e-12);
//! ```

use core::f64::consts::{PI, TAU};

use crate::math;

/// An angle in radians. The inner value is public; angles have no invariant.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct Radians(pub f64);

/// An angle in degrees.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct Degrees(pub f64);

/// An angle in hours, where 24 hours is a full turn (15 degrees per hour).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct HourAngle(pub f64);

impl Radians {
    /// Converts to degrees.
    #[inline]
    pub fn to_degrees(self) -> Degrees {
        Degrees(self.0 * (180.0 / PI))
    }

    /// Converts to hours of right ascension (`24 h = 2 pi`).
    #[inline]
    pub fn to_hour_angle(self) -> HourAngle {
        HourAngle(self.0 * (12.0 / PI))
    }

    /// Wraps into `[0, 2 pi)`.
    #[inline]
    pub fn wrap_two_pi(self) -> Radians {
        Radians(wrap(self.0, TAU))
    }

    /// Wraps into `[-pi, pi)`.
    #[inline]
    pub fn wrap_pi(self) -> Radians {
        Radians(wrap(self.0 + PI, TAU) - PI)
    }

    /// Returns `(sin, cos)` of the angle.
    #[inline]
    pub fn sin_cos(self) -> (f64, f64) {
        math::sin_cos(self.0)
    }
}

impl Degrees {
    /// Converts to radians.
    #[inline]
    pub fn to_radians(self) -> Radians {
        Radians(self.0 * (PI / 180.0))
    }

    /// Wraps into `[0, 360)`.
    #[inline]
    pub fn wrap_360(self) -> Degrees {
        Degrees(wrap(self.0, 360.0))
    }

    /// Wraps into `[-180, 180)`.
    #[inline]
    pub fn wrap_180(self) -> Degrees {
        Degrees(wrap(self.0 + 180.0, 360.0) - 180.0)
    }
}

impl HourAngle {
    /// Converts to radians (`1 h = 15 deg`).
    #[inline]
    pub fn to_radians(self) -> Radians {
        Radians(self.0 * (PI / 12.0))
    }

    /// Wraps into `[0, 24)` hours.
    #[inline]
    pub fn wrap_24(self) -> HourAngle {
        HourAngle(wrap(self.0, 24.0))
    }
}

impl From<Degrees> for Radians {
    #[inline]
    fn from(d: Degrees) -> Self {
        d.to_radians()
    }
}

impl From<Radians> for Degrees {
    #[inline]
    fn from(r: Radians) -> Self {
        r.to_degrees()
    }
}

impl From<HourAngle> for Radians {
    #[inline]
    fn from(h: HourAngle) -> Self {
        h.to_radians()
    }
}

impl From<Radians> for HourAngle {
    #[inline]
    fn from(r: Radians) -> Self {
        r.to_hour_angle()
    }
}

/// Wraps `x` into `[0, period)`. Non-finite input is returned unchanged.
///
/// Guards against the rounding case where `x.rem_euclid(period)` returns
/// exactly `period` for a tiny negative `x`.
#[inline]
fn wrap(x: f64, period: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let r = x - period * math::floor(x / period);
    if r >= period {
        r - period
    } else if r < 0.0 {
        r + period
    } else {
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn degree_radian_round_trip() {
        assert!(close(Degrees(90.0).to_radians().0, PI / 2.0));
        assert!(close(Radians(PI).to_degrees().0, 180.0));
        let d = Degrees(37.25);
        assert!(close(Radians::from(d).to_degrees().0, 37.25));
    }

    #[test]
    fn hour_angle_conversions() {
        assert!(close(HourAngle(6.0).to_radians().0, PI / 2.0));
        assert!(close(Radians(PI).to_hour_angle().0, 12.0));
        assert!(close(HourAngle(12.0).to_radians().to_degrees().0, 180.0));
    }

    #[test]
    fn wrapping_table() {
        let cases: &[(f64, f64)] = &[
            (0.0, 0.0),
            (360.0, 0.0),
            (370.0, 10.0),
            (-90.0, 270.0),
            (-360.0, 0.0),
            (725.5, 5.5),
        ];
        for &(input, expected) in cases {
            assert!(
                close(Degrees(input).wrap_360().0, expected),
                "wrap_360({input})"
            );
        }
        assert!(close(Degrees(190.0).wrap_180().0, -170.0));
        assert!(close(Degrees(-180.0).wrap_180().0, -180.0));
        assert!(close(Degrees(180.0).wrap_180().0, -180.0));
        assert!(close(HourAngle(25.0).wrap_24().0, 1.0));
        assert!(close(HourAngle(-1.0).wrap_24().0, 23.0));
        assert!(close(Radians(-PI / 2.0).wrap_two_pi().0, 3.0 * PI / 2.0));
        assert!(close(Radians(3.0 * PI / 2.0).wrap_pi().0, -PI / 2.0));
    }

    #[test]
    fn wrap_result_is_always_in_range() {
        let mut x = -1000.0;
        while x < 1000.0 {
            let w = Degrees(x).wrap_360().0;
            assert!((0.0..360.0).contains(&w), "{x} -> {w}");
            x += 0.37;
        }
    }
}
