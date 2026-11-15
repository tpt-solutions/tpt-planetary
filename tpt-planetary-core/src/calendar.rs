//! Calendar dates and Julian Dates.
//!
//! Conversions use the algorithm in Jean Meeus, *Astronomical Algorithms*
//! (2nd ed., 1998), chapter 7, written in exact integer arithmetic. Floating
//! point is used only for the day fraction. The Fliegel-Van Flandern
//! algorithm is not used.
//!
//! Both calendars are **proleptic**: the Gregorian rules are applied to every
//! date, including dates before 1582-10-15, and the Julian rules are applied to
//! every date in the Julian calendar. Use [`CalendarSystem`] to choose.
//!
//! Julian Dates here are the astronomical ones: day numbers start at noon.
//! Midnight of a civil day therefore falls on a `.5` Julian Date.
//!
//! ```
//! use tpt_planetary_core::calendar::{CalendarSystem, CivilDate};
//! use tpt_planetary_core::jd::JulianDate;
//!
//! // 2000-01-01 12:00 in the Gregorian calendar is JD 2451545.0.
//! let jd = JulianDate::from_calendar(
//!     CivilDate::new(2000, 1, 1),
//!     CalendarSystem::Gregorian,
//!     43_200.0,
//! ).unwrap();
//! assert_eq!(jd.to_f64(), 2_451_545.0);
//! ```

use crate::error::Error;
use crate::jd::JulianDate;

/// Which calendar rules to apply, both applied proleptically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CalendarSystem {
    /// The Julian calendar: every fourth year is a leap year.
    Julian,
    /// The Gregorian calendar: century years are leap years only if divisible by 400.
    Gregorian,
}

/// A calendar date (year, month, day) with no time of day.
///
/// The year uses astronomical numbering, so year 0 is 1 BC and year -4712 is
/// 4713 BC. Fields are public; use [`CivilDate::to_jd`] or
/// [`JulianDate::from_calendar`] to convert, which validate the date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilDate {
    /// Astronomical year.
    pub year: i32,
    /// Month, 1 to 12.
    pub month: u8,
    /// Day of month, 1 to the length of the month.
    pub day: u8,
}

impl CivilDate {
    /// Builds a date without validating it. Validation happens at conversion.
    pub const fn new(year: i32, month: u8, day: u8) -> Self {
        CivilDate { year, month, day }
    }

    /// Returns `true` if the date exists in the given calendar system.
    ///
    /// ```
    /// # use tpt_planetary_core::calendar::{CalendarSystem, CivilDate};
    /// assert!(!CivilDate::new(1900, 2, 29).is_valid(CalendarSystem::Gregorian));
    /// assert!(CivilDate::new(1900, 2, 29).is_valid(CalendarSystem::Julian));
    /// ```
    pub fn is_valid(self, cal: CalendarSystem) -> bool {
        (1..=12).contains(&self.month)
            && self.day >= 1
            && self.day <= days_in_month(self.year, self.month, cal)
    }

    /// Returns the Julian Date of midnight at the start of this date.
    ///
    /// ```
    /// # use tpt_planetary_core::calendar::{CalendarSystem, CivilDate};
    /// let jd = CivilDate::new(1970, 1, 1).to_jd(CalendarSystem::Gregorian).unwrap();
    /// assert_eq!(jd.to_f64(), 2_440_587.5);
    /// ```
    pub fn to_jd(self, cal: CalendarSystem) -> Result<JulianDate, Error> {
        JulianDate::from_calendar(self, cal, 0.0)
    }
}

/// Returns the number of days in `month` of `year` in the given calendar.
///
/// # Panics
///
/// Does not panic. For a `month` outside 1 to 12 it returns 0.
pub fn days_in_month(year: i32, month: u8, cal: CalendarSystem) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year, cal) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Returns `true` if `year` is a leap year in the given calendar.
pub fn is_leap_year(year: i32, cal: CalendarSystem) -> bool {
    let div4 = year.rem_euclid(4) == 0;
    match cal {
        CalendarSystem::Julian => div4,
        CalendarSystem::Gregorian => {
            div4 && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0)
        }
    }
}

impl JulianDate {
    /// Builds the Julian Date of a calendar date plus a time of day.
    ///
    /// `seconds_of_day` must be in `[0, 86400)`. Leap seconds are not
    /// represented here; for UTC with leap seconds use the `Epoch` API.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidDate`] if the date does not exist in `cal`, and
    /// [`Error::InvalidTime`] if `seconds_of_day` is outside `[0, 86400)`.
    ///
    /// ```
    /// # use tpt_planetary_core::calendar::{CalendarSystem, CivilDate};
    /// # use tpt_planetary_core::jd::JulianDate;
    /// let jd = JulianDate::from_calendar(CivilDate::new(2000, 1, 1), CalendarSystem::Gregorian, 43_200.0)?;
    /// assert_eq!(jd, JulianDate::J2000);
    /// # Ok::<(), tpt_planetary_core::Error>(())
    /// ```
    pub fn from_calendar(
        date: CivilDate,
        cal: CalendarSystem,
        seconds_of_day: f64,
    ) -> Result<Self, Error> {
        let k = day_number(date, cal)?;
        if !(0.0..86_400.0).contains(&seconds_of_day) {
            return Err(Error::InvalidTime);
        }
        Ok(JulianDate::new(k as f64, 0.5 + seconds_of_day / 86_400.0))
    }

    /// Returns the calendar date and the seconds since midnight of the civil
    /// day that contains this Julian Date. The seconds are in `[0, 86400)`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDate`] if the day part is not finite or its
    /// magnitude exceeds 5e11 days (about 1.37 billion years, so the year fits
    /// in `i32`). Beyond that the integer calendar
    /// arithmetic could overflow.
    ///
    /// ```
    /// # use tpt_planetary_core::calendar::{CalendarSystem, CivilDate};
    /// # use tpt_planetary_core::jd::JulianDate;
    /// let (date, secs) = JulianDate::new(2_451_545.0, 0.0)
    ///     .to_calendar(CalendarSystem::Gregorian)
    ///     .unwrap();
    /// assert_eq!(date, CivilDate::new(2000, 1, 1));
    /// assert_eq!(secs, 43_200.0);
    /// ```
    pub fn to_calendar(self, cal: CalendarSystem) -> Result<(CivilDate, f64), Error> {
        let day = self.day().abs();
        if day.is_nan() || day > MAX_CALENDAR_DAY {
            return Err(Error::InvalidDate);
        }
        let (k, frac_day) = civil_day_and_fraction(self);
        Ok((date_from_day_number(k, cal), frac_day * 86_400.0))
    }
}

/// Splits a Julian Date into the civil day number `k` and the fraction of
/// that civil day in `[0, 1)`.
///
/// `k` is the integer part of the Julian Date of midnight that starts the
/// civil day, so the civil day `k` runs from `k + 0.5` to `k + 1.5`.
/// Largest day magnitude accepted by [`JulianDate::to_calendar`]; keeps the
/// integer calendar arithmetic far from `i64` overflow.
pub(crate) const MAX_CALENDAR_DAY: f64 = 5.0e11;

pub(crate) fn civil_day_and_fraction(jd: JulianDate) -> (i64, f64) {
    let whole = jd.day() as i64;
    let f = jd.fraction();
    if f >= 0.5 {
        (whole, f - 0.5)
    } else {
        (whole - 1, f + 0.5)
    }
}

/// Returns the civil day number `k` (the integer Julian Date of midnight) for
/// a calendar date, using Meeus chapter 7 in integer arithmetic.
pub(crate) fn day_number(date: CivilDate, cal: CalendarSystem) -> Result<i64, Error> {
    if !date.is_valid(cal) {
        return Err(Error::InvalidDate);
    }
    let mut y = i64::from(date.year);
    let mut m = i64::from(date.month);
    let d = i64::from(date.day);
    if m <= 2 {
        y -= 1;
        m += 12;
    }
    let b = match cal {
        CalendarSystem::Gregorian => {
            let a = y.div_euclid(100);
            2 - a + a.div_euclid(4)
        }
        CalendarSystem::Julian => 0,
    };
    // floor(365.25 * (y + 4716)) and floor(30.6001 * (m + 1)) in integers.
    let x = (1461 * (y + 4716)).div_euclid(4) + (306_001 * (m + 1)).div_euclid(10_000) + d + b;
    // JD of midnight = x - 1524.5 = (x - 1525) + 0.5.
    Ok(x - 1525)
}

/// Inverse of [`day_number`]: returns the calendar date of civil day `k`.
pub(crate) fn date_from_day_number(k: i64, cal: CalendarSystem) -> CivilDate {
    // Meeus uses Z = floor(JD + 0.5), which is k + 1 for midnight of day k.
    let z = k + 1;
    let a = match cal {
        CalendarSystem::Julian => z,
        CalendarSystem::Gregorian => {
            let alpha = (4 * z - 7_468_865).div_euclid(146_097);
            z + 1 + alpha - alpha.div_euclid(4)
        }
    };
    let b = a + 1524;
    let c = (20 * b - 2442).div_euclid(7305);
    let d = (1461 * c).div_euclid(4);
    let e = ((b - d) * 10_000).div_euclid(306_001);
    let day = b - d - (306_001 * e).div_euclid(10_000);
    let month = if e < 14 { e - 1 } else { e - 13 };
    let year = if month > 2 { c - 4716 } else { c - 4715 };
    CivilDate {
        year: year as i32,
        month: month as u8,
        day: day as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CalendarSystem::{Gregorian, Julian};

    fn jd_of(y: i32, m: u8, d: u8, cal: CalendarSystem) -> f64 {
        CivilDate::new(y, m, d).to_jd(cal).unwrap().to_f64()
    }

    #[test]
    fn golden_gregorian_values() {
        // J2000.0: 2000-01-01T12:00 (JD 2451545.0), certain by definition.
        assert_eq!(
            JulianDate::from_calendar(CivilDate::new(2000, 1, 1), Gregorian, 43_200.0)
                .unwrap()
                .to_f64(),
            2_451_545.0
        );
        // Unix epoch 1970-01-01T00:00 UTC is JD 2440587.5.
        assert_eq!(jd_of(1970, 1, 1, Gregorian), 2_440_587.5);
        // 1972-01-01 (start of the leap-second table) is JD 2441317.5.
        assert_eq!(jd_of(1972, 1, 1, Gregorian), 2_441_317.5);
        // 2017-01-01 is JD 2457754.5.
        assert_eq!(jd_of(2017, 1, 1, Gregorian), 2_457_754.5);
        // MJD 0 is 1858-11-17 00:00, JD 2400000.5.
        assert_eq!(jd_of(1858, 11, 17, Gregorian), 2_400_000.5);
    }

    #[test]
    fn golden_julian_and_reform_values() {
        // JD 0.0 is noon on 4713 BC January 1 in the Julian calendar (year -4712).
        assert_eq!(
            JulianDate::from_calendar(CivilDate::new(-4712, 1, 1), Julian, 43_200.0)
                .unwrap()
                .to_f64(),
            0.0
        );
        // Gregorian reform: the day after 1582-10-04 (Julian) is 1582-10-15 (Gregorian).
        assert_eq!(jd_of(1582, 10, 4, Julian), 2_299_159.5);
        assert_eq!(jd_of(1582, 10, 15, Gregorian), 2_299_160.5);
    }

    #[test]
    fn leap_year_rules() {
        assert!(is_leap_year(2000, Gregorian));
        assert!(!is_leap_year(1900, Gregorian));
        assert!(is_leap_year(1900, Julian));
        assert!(is_leap_year(2024, Gregorian));
        assert!(!is_leap_year(2023, Gregorian));
    }

    #[test]
    fn invalid_dates_and_times_are_rejected() {
        assert_eq!(
            day_number(CivilDate::new(2001, 2, 29), Gregorian),
            Err(Error::InvalidDate)
        );
        assert_eq!(
            day_number(CivilDate::new(2001, 13, 1), Gregorian),
            Err(Error::InvalidDate)
        );
        assert_eq!(
            day_number(CivilDate::new(2001, 4, 31), Gregorian),
            Err(Error::InvalidDate)
        );
        assert_eq!(
            JulianDate::from_calendar(CivilDate::new(2001, 1, 1), Gregorian, 86_400.0),
            Err(Error::InvalidTime)
        );
    }

    #[test]
    fn round_trip_over_several_centuries_both_calendars() {
        for cal in [Julian, Gregorian] {
            for k in (-300_000_i64..=900_000).step_by(37) {
                let date = date_from_day_number(k, cal);
                assert!(date.is_valid(cal), "{date:?} invalid in {cal:?}");
                assert_eq!(day_number(date, cal), Ok(k), "k = {k}, {cal:?}");
            }
        }
    }

    #[test]
    fn to_calendar_round_trips_time_of_day() {
        let jd =
            JulianDate::from_calendar(CivilDate::new(1999, 12, 31), Gregorian, 86_399.0).unwrap();
        let (date, secs) = jd.to_calendar(Gregorian).unwrap();
        assert_eq!(date, CivilDate::new(1999, 12, 31));
        assert!((secs - 86_399.0).abs() < 1e-6);
    }
}

#[cfg(test)]
mod range_tests {
    use super::*;
    use crate::jd::JulianDate;

    #[test]
    fn to_calendar_rejects_out_of_range_and_nan() {
        for day in [1.0e13, -1.0e13, f64::INFINITY, f64::NAN] {
            assert_eq!(
                JulianDate::new(day, 0.0).to_calendar(CalendarSystem::Gregorian),
                Err(Error::InvalidDate),
                "day = {day}"
            );
        }
    }

    #[test]
    fn to_calendar_accepts_the_range_edge() {
        let (date, _) = JulianDate::new(MAX_CALENDAR_DAY, 0.5)
            .to_calendar(CalendarSystem::Gregorian)
            .expect("edge of range");
        assert!(date.year > 1_000_000_000);
    }
}
