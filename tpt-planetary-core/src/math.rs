//! Crate-private math helpers.
//!
//! `core` does not provide `sqrt`, `floor`, `sin`, `cos` or `atan2` for `f64`
//! (they live in `std` or need a libm backend). To stay `no_std` without adding
//! a dependency, the trigonometric functions are routed through `glam`, which is
//! built with its `libm` feature, and `sqrt` and `floor` are implemented here.

use glam::f64::DVec2;

/// Returns `(sin(x), cos(x))`.
#[inline]
pub(crate) fn sin_cos(x: f64) -> (f64, f64) {
    let v = DVec2::from_angle(x);
    (v.y, v.x)
}

/// Returns `sin(x)`.
#[inline]
pub(crate) fn sin(x: f64) -> f64 {
    sin_cos(x).0
}

/// Two-argument arctangent, result in `[-pi, pi]`.
///
/// Returns `0.0` when both arguments are zero (the convention used by `atan2`).
#[inline]
pub(crate) fn atan2(y: f64, x: f64) -> f64 {
    DVec2::new(x, y).to_angle()
}

/// Euclidean length `sqrt(a^2 + b^2)` without destructive overflow.
#[inline]
pub(crate) fn hypot(a: f64, b: f64) -> f64 {
    DVec2::new(a, b).length()
}

/// Square root by Newton iteration. Returns NaN for negative input.
pub(crate) fn sqrt(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    // Initial guess from halving the exponent bits; Newton converges quadratically.
    let mut y = f64::from_bits((x.to_bits() >> 1) + 0x1ff8_0000_0000_0000);
    for _ in 0..8 {
        y = 0.5 * (y + x / y);
    }
    y
}

/// Largest integer not greater than `x`.
pub(crate) fn floor(x: f64) -> f64 {
    if !x.is_finite() || x.abs() >= 9.0e15 {
        return x;
    }
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqrt_matches_exact_squares() {
        for n in 0..200 {
            let x = (n * n) as f64;
            assert_eq!(sqrt(x), n as f64);
        }
        assert!((sqrt(2.0) - core::f64::consts::SQRT_2).abs() < 1e-15);
        assert!(sqrt(-1.0).is_nan());
    }

    #[test]
    fn floor_handles_signs() {
        assert_eq!(floor(2.7), 2.0);
        assert_eq!(floor(-2.2), -3.0);
        assert_eq!(floor(-2.0), -2.0);
        assert_eq!(floor(0.0), 0.0);
    }

    #[test]
    fn trig_basics() {
        let (s, c) = sin_cos(0.0);
        assert_eq!(s, 0.0);
        assert_eq!(c, 1.0);
        assert!((atan2(1.0, 1.0) - core::f64::consts::FRAC_PI_4).abs() < 1e-15);
        assert!((hypot(3.0, 4.0) - 5.0).abs() < 1e-15);
    }
}
