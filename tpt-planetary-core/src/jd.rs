//! Julian Date and Modified Julian Date as two-part `f64` values.
//!
//! A single `f64` holds a Julian Date with a resolution of about 40
//! microseconds at the present epoch: the unit in the last place of 2.46e6
//! days is 2^-31 days. Storing the integer day and the day fraction
//! separately keeps the fraction at full `f64` precision.
//!
//! Both parts are `f64`. The day part is always integer-valued and the
//! fraction is always in `[0, 1)`. Every constructor normalises its input, so
//! the derived ordering and equality are correct.
//!
//! ```
//! use tpt_planetary_core::jd::JulianDate;
//!
//! let j2000 = JulianDate::new(2_451_545.0, 0.0);
//! assert_eq!(j2000.day(), 2_451_545.0);
//! assert_eq!(j2000.fraction(), 0.0);
//!
//! // Normalisation moves whole days out of the fraction.
//! let t = JulianDate::new(2_451_545.0, 1.25);
//! assert_eq!(t.day(), 2_451_546.0);
//! assert_eq!(t.fraction(), 0.25);
//! ```

use crate::constants::{JD_J2000, SECONDS_PER_DAY};
use crate::math;

/// A Julian Date stored as an integer day and a fraction in `[0, 1)`.
///
/// The day is the Julian Day Number of the civil day that begins at noon, so
/// `JulianDate::new(2_451_545.0, 0.0)` is noon on 2000-01-01.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct JulianDate {
    day: f64,
    frac: f64,
}

impl JulianDate {
    /// J2000.0 (2000-01-01T12:00:00 TT).
    pub const J2000: JulianDate = JulianDate {
        day: JD_J2000,
        frac: 0.0,
    };

    /// Builds a Julian Date from a day part and a fraction, normalising both.
    ///
    /// Either argument may be out of its nominal range; whole days are moved
    /// from `frac` into `day`. Non-finite input produces non-finite output.
    ///
    /// ```
    /// # use tpt_planetary_core::jd::JulianDate;
    /// let t = JulianDate::new(2_451_544.0, -0.5);
    /// assert_eq!((t.day(), t.fraction()), (2_451_543.0, 0.5));
    /// ```
    pub fn new(day: f64, frac: f64) -> Self {
        normalise(day, frac)
    }

    /// Builds a Julian Date from a single `f64` value.
    ///
    /// Use [`JulianDate::new`] when precision matters; this constructor
    /// rounds to the precision of `f64` at the current epoch.
    pub fn from_f64(jd: f64) -> Self {
        normalise(jd, 0.0)
    }

    /// Returns the integer day part.
    #[inline]
    pub fn day(self) -> f64 {
        self.day
    }

    /// Returns the fraction of a day, in `[0, 1)`.
    #[inline]
    pub fn fraction(self) -> f64 {
        self.frac
    }

    /// Returns the Julian Date as a single `f64`, losing some precision.
    ///
    /// ```
    /// # use tpt_planetary_core::jd::JulianDate;
    /// assert_eq!(JulianDate::new(2_451_545.0, 0.5).to_f64(), 2_451_545.5);
    /// ```
    #[inline]
    pub fn to_f64(self) -> f64 {
        self.day + self.frac
    }

    /// Adds a number of days, normalising the result.
    #[inline]
    pub fn add_days(self, days: f64) -> Self {
        normalise(self.day, self.frac + days)
    }

    /// Adds a number of SI seconds, normalising the result.
    ///
    /// ```
    /// # use tpt_planetary_core::jd::JulianDate;
    /// let t = JulianDate::J2000.add_seconds(86_400.0);
    /// assert_eq!(t.day(), 2_451_546.0);
    /// ```
    #[inline]
    pub fn add_seconds(self, seconds: f64) -> Self {
        normalise(self.day, self.frac + seconds / SECONDS_PER_DAY)
    }

    /// Returns `self - other` in days, computed from the two-part values so
    /// that large common day numbers do not cancel fraction precision.
    ///
    /// ```
    /// # use tpt_planetary_core::jd::JulianDate;
    /// let a = JulianDate::new(2_451_545.0, 0.25);
    /// let b = JulianDate::new(2_451_544.0, 0.75);
    /// assert_eq!(a.days_since(b), 0.5);
    /// ```
    #[inline]
    pub fn days_since(self, other: JulianDate) -> f64 {
        (self.day - other.day) + (self.frac - other.frac)
    }

    /// Returns `self - other` in SI seconds.
    #[inline]
    pub fn seconds_since(self, other: JulianDate) -> f64 {
        self.days_since(other) * SECONDS_PER_DAY
    }

    /// Converts to a Modified Julian Date (`MJD = JD - 2400000.5`).
    ///
    /// ```
    /// # use tpt_planetary_core::jd::JulianDate;
    /// let mjd = JulianDate::new(2_400_000.5, 0.0).to_mjd();
    /// assert_eq!((mjd.day(), mjd.fraction()), (0.0, 0.0));
    /// ```
    pub fn to_mjd(self) -> ModifiedJulianDate {
        // MJD = JD - 2400000.5 = (day - 2400000) + (frac - 0.5).
        let shifted = normalise(self.day - 2_400_000.0, self.frac - 0.5);
        ModifiedJulianDate {
            day: shifted.day,
            frac: shifted.frac,
        }
    }
}

/// A Modified Julian Date stored as an integer day and a fraction in `[0, 1)`.
///
/// `MJD = JD - 2400000.5`, so MJD 0 is 1858-11-17T00:00 (midnight).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct ModifiedJulianDate {
    day: f64,
    frac: f64,
}

impl ModifiedJulianDate {
    /// Builds a Modified Julian Date, normalising both parts.
    pub fn new(day: f64, frac: f64) -> Self {
        let n = normalise(day, frac);
        ModifiedJulianDate {
            day: n.day,
            frac: n.frac,
        }
    }

    /// Returns the integer day part.
    #[inline]
    pub fn day(self) -> f64 {
        self.day
    }

    /// Returns the fraction of a day, in `[0, 1)`.
    #[inline]
    pub fn fraction(self) -> f64 {
        self.frac
    }

    /// Converts back to a Julian Date.
    ///
    /// ```
    /// # use tpt_planetary_core::jd::{JulianDate, ModifiedJulianDate};
    /// let jd = ModifiedJulianDate::new(0.0, 0.0).to_jd();
    /// assert_eq!(jd.to_f64(), 2_400_000.5);
    /// ```
    pub fn to_jd(self) -> JulianDate {
        let n = normalise(self.day + 2_400_000.0, self.frac + 0.5);
        JulianDate {
            day: n.day,
            frac: n.frac,
        }
    }
}

/// Normalises `(day, frac)` so that `day` is integer-valued and `frac` is in
/// `[0, 1)`. Non-finite input is returned as NaN parts.
fn normalise(day: f64, frac: f64) -> JulianDate {
    if !day.is_finite() || !frac.is_finite() {
        return JulianDate {
            day: f64::NAN,
            frac: f64::NAN,
        };
    }
    // Move the integer part of `day` out first, then fold whole days of `frac`.
    let di = math::floor(day);
    let fi = (day - di) + frac;
    let w = math::floor(fi);
    let mut out_day = di + w;
    let mut out_frac = fi - w;
    // Rounding can leave `out_frac` at exactly 1.0 for tiny negative inputs.
    if out_frac >= 1.0 {
        out_frac -= 1.0;
        out_day += 1.0;
    }
    JulianDate {
        day: out_day,
        frac: out_frac,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation_invariants() {
        let cases = [
            (2_451_545.0, 0.0),
            (2_451_545.0, 1.25),
            (2_451_545.0, -0.25),
            (2_451_545.0, -3.5),
            (2_451_545.5, 0.5),
            (-10.0, 0.75),
            (0.0, -1e-18),
        ];
        for (d, f) in cases {
            let t = JulianDate::new(d, f);
            assert_eq!(
                t.day(),
                math::floor(t.day()),
                "day not integral for ({d}, {f})"
            );
            assert!(
                (0.0..1.0).contains(&t.fraction()),
                "frac out of range for ({d}, {f})"
            );
            assert!((t.to_f64() - (d + f)).abs() < 1e-9);
        }
    }

    #[test]
    fn negative_tiny_fraction_does_not_round_to_one() {
        // -1e-18 days is a hair before day 5, so it normalises to day 5 with fraction 0.
        let t = JulianDate::new(5.0, -1e-18);
        assert!(t.fraction() < 1.0);
        assert_eq!((t.day(), t.fraction()), (5.0, 0.0));
    }

    #[test]
    fn j2000_and_mjd_golden_values() {
        // JD 2451545.0 is 2000-01-01T12:00 TT (definition of J2000.0).
        assert_eq!(JulianDate::J2000.to_f64(), 2_451_545.0);
        // MJD 51544.5 is the same instant.
        let mjd = JulianDate::new(2_451_545.0, 0.0).to_mjd();
        assert_eq!((mjd.day(), mjd.fraction()), (51_544.0, 0.5));
        // MJD 0 is 1858-11-17T00:00, JD 2400000.5.
        let zero = ModifiedJulianDate::new(0.0, 0.0).to_jd();
        assert_eq!((zero.day(), zero.fraction()), (2_400_000.0, 0.5));
    }

    #[test]
    fn mjd_round_trip_keeps_fraction_precision() {
        let jd = JulianDate::new(2_451_545.0, 0.123_456_789_012_345);
        let back = jd.to_mjd().to_jd();
        assert_eq!(back.day(), jd.day());
        assert!((back.fraction() - jd.fraction()).abs() < 1e-15);
    }

    #[test]
    fn arithmetic_and_differences() {
        let t = JulianDate::J2000.add_seconds(43_200.0);
        assert_eq!((t.day(), t.fraction()), (2_451_545.0, 0.5));
        assert_eq!(t.seconds_since(JulianDate::J2000), 43_200.0);
        assert_eq!(
            JulianDate::J2000
                .add_days(-1.0)
                .days_since(JulianDate::J2000),
            -1.0
        );
    }

    #[test]
    fn ordering_uses_normalised_parts() {
        let a = JulianDate::new(10.0, 0.9);
        let b = JulianDate::new(11.0, -0.05);
        assert!(a < b);
        assert_eq!(JulianDate::new(10.0, 1.5), JulianDate::new(11.0, 0.5));
    }
}
