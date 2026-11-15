//! Leap-second table and UTC <-> TAI conversion.
//!
//! The table records the value of TAI - UTC from each listed UTC midnight
//! onward. Only positive one-second leap seconds are supported. The embedded
//! table [`EMBEDDED`] covers 1972-01-01 (TAI - UTC = 10 s) through
//! 2017-01-01 (TAI - UTC = 37 s), which is the complete history since the
//! 1972 redefinition of UTC.
//!
//! # Expiry
//!
//! A leap second can be announced only a few months ahead, so a table is
//! complete only up to its `valid_until` date. Conversions for instants after
//! that date return [`Error::LeapSecondTableExpired`] instead of guessing.
//! Callers can also test a table directly with
//! [`LeapSecondTable::is_expired_at`].
//!
//! # Leap-second day convention
//!
//! A UTC day that ends in a leap second has 86401 SI seconds. UTC Julian Dates
//! in this crate scale the day fraction by the length of the civil day (86400
//! or 86401), so that `fraction * day_length` is the number of seconds since
//! midnight. This is the convention used by SOFA/ERFA. A UTC epoch on a leap
//! day with seconds in `[86400, 86401)` is the leap second `23:59:60`.

use crate::calendar::{
    CalendarSystem, CivilDate, civil_day_and_fraction, date_from_day_number, day_number,
};
use crate::constants::SECONDS_PER_DAY;
use crate::error::Error;
use crate::jd::JulianDate;

/// Residues smaller than this many seconds are treated as exact midnight when
/// splitting a Julian Date into civil days. The representation resolution is
/// about 1e-11 s, so this is far above rounding noise and far below any
/// physical quantity.
const SNAP_SECONDS: f64 = 1e-9;

/// One entry of a leap-second table: TAI - UTC takes the value
/// `tai_minus_utc` from 00:00 UTC on the given Gregorian date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeapSecond {
    /// Gregorian year.
    pub year: i32,
    /// Gregorian month, 1 to 12.
    pub month: u8,
    /// Gregorian day of month.
    pub day: u8,
    /// TAI - UTC in whole seconds, from this date onward.
    pub tai_minus_utc: i32,
}

/// A leap-second table with a version and an expiry date.
///
/// Create a custom table with [`LeapSecondTable::new`], which validates it, or
/// use [`EMBEDDED`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeapSecondTable<'a> {
    entries: &'a [LeapSecond],
    valid_until: CivilDate,
    version: u32,
    source: &'static str,
}

/// The embedded leap-second table (version 1).
///
/// Entries are the TAI - UTC steps of the IERS history from 1972-01-01
/// (10 s) to 2017-01-01 (37 s).
///
/// The entries were checked line by line against the IERS `leap-seconds.list`
/// file (hpiers.obspm.fr, NTP timestamp `#@` line) on 2026-11-16.
///
/// **Expiry:** `valid_until` is 2027-06-28, the expiry date stated in that IERS
/// file. Before a release, re-check it against the current file and update
/// `valid_until` (and `version`) if a newer file has been published.
pub const EMBEDDED: LeapSecondTable<'static> = LeapSecondTable {
    entries: &EMBEDDED_ENTRIES,
    valid_until: CivilDate::new(2027, 6, 28),
    version: 1,
    source: "IERS leap-second history 1972-2017 (embedded)",
};

const EMBEDDED_ENTRIES: [LeapSecond; 28] = [
    LeapSecond {
        year: 1972,
        month: 1,
        day: 1,
        tai_minus_utc: 10,
    },
    LeapSecond {
        year: 1972,
        month: 7,
        day: 1,
        tai_minus_utc: 11,
    },
    LeapSecond {
        year: 1973,
        month: 1,
        day: 1,
        tai_minus_utc: 12,
    },
    LeapSecond {
        year: 1974,
        month: 1,
        day: 1,
        tai_minus_utc: 13,
    },
    LeapSecond {
        year: 1975,
        month: 1,
        day: 1,
        tai_minus_utc: 14,
    },
    LeapSecond {
        year: 1976,
        month: 1,
        day: 1,
        tai_minus_utc: 15,
    },
    LeapSecond {
        year: 1977,
        month: 1,
        day: 1,
        tai_minus_utc: 16,
    },
    LeapSecond {
        year: 1978,
        month: 1,
        day: 1,
        tai_minus_utc: 17,
    },
    LeapSecond {
        year: 1979,
        month: 1,
        day: 1,
        tai_minus_utc: 18,
    },
    LeapSecond {
        year: 1980,
        month: 1,
        day: 1,
        tai_minus_utc: 19,
    },
    LeapSecond {
        year: 1981,
        month: 7,
        day: 1,
        tai_minus_utc: 20,
    },
    LeapSecond {
        year: 1982,
        month: 7,
        day: 1,
        tai_minus_utc: 21,
    },
    LeapSecond {
        year: 1983,
        month: 7,
        day: 1,
        tai_minus_utc: 22,
    },
    LeapSecond {
        year: 1985,
        month: 7,
        day: 1,
        tai_minus_utc: 23,
    },
    LeapSecond {
        year: 1988,
        month: 1,
        day: 1,
        tai_minus_utc: 24,
    },
    LeapSecond {
        year: 1990,
        month: 1,
        day: 1,
        tai_minus_utc: 25,
    },
    LeapSecond {
        year: 1991,
        month: 1,
        day: 1,
        tai_minus_utc: 26,
    },
    LeapSecond {
        year: 1992,
        month: 7,
        day: 1,
        tai_minus_utc: 27,
    },
    LeapSecond {
        year: 1993,
        month: 7,
        day: 1,
        tai_minus_utc: 28,
    },
    LeapSecond {
        year: 1994,
        month: 7,
        day: 1,
        tai_minus_utc: 29,
    },
    LeapSecond {
        year: 1996,
        month: 1,
        day: 1,
        tai_minus_utc: 30,
    },
    LeapSecond {
        year: 1997,
        month: 7,
        day: 1,
        tai_minus_utc: 31,
    },
    LeapSecond {
        year: 1999,
        month: 1,
        day: 1,
        tai_minus_utc: 32,
    },
    LeapSecond {
        year: 2006,
        month: 1,
        day: 1,
        tai_minus_utc: 33,
    },
    LeapSecond {
        year: 2009,
        month: 1,
        day: 1,
        tai_minus_utc: 34,
    },
    LeapSecond {
        year: 2012,
        month: 7,
        day: 1,
        tai_minus_utc: 35,
    },
    LeapSecond {
        year: 2015,
        month: 7,
        day: 1,
        tai_minus_utc: 36,
    },
    LeapSecond {
        year: 2017,
        month: 1,
        day: 1,
        tai_minus_utc: 37,
    },
];

impl<'a> LeapSecondTable<'a> {
    /// Creates and validates a custom table.
    ///
    /// Entries must be non-empty, have valid Gregorian dates, be strictly
    /// increasing by date, and each step must be exactly +1 s after the first.
    /// `valid_until` must be a valid date on or after the last entry.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidLeapSecondTable`] if any rule above is broken, or
    /// [`Error::InvalidDate`] if a date is not a valid Gregorian date.
    ///
    /// ```
    /// use tpt_planetary_core::calendar::CivilDate;
    /// use tpt_planetary_core::leap::{LeapSecond, LeapSecondTable};
    ///
    /// let entries = [
    ///     LeapSecond { year: 2015, month: 7, day: 1, tai_minus_utc: 36 },
    ///     LeapSecond { year: 2017, month: 1, day: 1, tai_minus_utc: 37 },
    /// ];
    /// let table = LeapSecondTable::new(&entries, CivilDate::new(2018, 1, 1), 2, "example").unwrap();
    /// assert_eq!(table.version(), 2);
    /// ```
    pub fn new(
        entries: &'a [LeapSecond],
        valid_until: CivilDate,
        version: u32,
        source: &'static str,
    ) -> Result<Self, Error> {
        let table = LeapSecondTable {
            entries,
            valid_until,
            version,
            source,
        };
        table.validate()?;
        Ok(table)
    }

    fn validate(&self) -> Result<(), Error> {
        if self.entries.is_empty() {
            return Err(Error::InvalidLeapSecondTable);
        }
        let mut prev: Option<(i64, i32)> = None;
        for e in self.entries {
            let k = day_number(
                CivilDate::new(e.year, e.month, e.day),
                CalendarSystem::Gregorian,
            )?;
            if let Some((pk, pv)) = prev
                && (k <= pk || e.tai_minus_utc != pv + 1)
            {
                return Err(Error::InvalidLeapSecondTable);
            }
            prev = Some((k, e.tai_minus_utc));
        }
        let last = prev.map(|(k, _)| k).unwrap_or(i64::MIN);
        let until = day_number(self.valid_until, CalendarSystem::Gregorian)?;
        if until < last {
            return Err(Error::InvalidLeapSecondTable);
        }
        Ok(())
    }

    /// Returns the entries of the table, in date order.
    pub fn entries(&self) -> &'a [LeapSecond] {
        self.entries
    }

    /// Returns the last civil date for which this table is asserted complete.
    pub fn valid_until(&self) -> CivilDate {
        self.valid_until
    }

    /// Returns the table's version number.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Returns a human-readable description of where the table came from.
    pub fn source(&self) -> &'static str {
        self.source
    }

    /// Returns `true` if `jd` (a UTC Julian Date) falls after the table's
    /// `valid_until` day, so the table may be missing leap seconds.
    ///
    /// ```
    /// use tpt_planetary_core::jd::JulianDate;
    /// use tpt_planetary_core::leap::EMBEDDED;
    ///
    /// assert!(!EMBEDDED.is_expired_at(JulianDate::new(2_451_545.0, 0.0)));
    /// assert!(EMBEDDED.is_expired_at(JulianDate::new(2_500_000.0, 0.0)));
    /// ```
    pub fn is_expired_at(&self, jd: JulianDate) -> bool {
        let until = match day_number(self.valid_until, CalendarSystem::Gregorian) {
            Ok(k) => k,
            Err(_) => return true,
        };
        let (k, _) = civil_day_and_fraction(jd);
        k > until
    }

    /// Returns the day number of the first table entry.
    fn first_day(&self) -> Result<i64, Error> {
        entry_day(&self.entries[0])
    }

    /// Returns the day number of `valid_until`.
    fn until_day(&self) -> Result<i64, Error> {
        day_number(self.valid_until, CalendarSystem::Gregorian)
    }

    /// Returns TAI - UTC (seconds) in force on civil day `k`, or an error if
    /// `k` precedes the table.
    fn offset_on_day(&self, k: i64) -> Result<i64, Error> {
        let mut found = None;
        for e in self.entries {
            if entry_day(e)? <= k {
                found = Some(i64::from(e.tai_minus_utc));
            } else {
                break;
            }
        }
        found.ok_or(Error::BeforeLeapSecondTable)
    }

    /// Returns `true` if civil day `k` ends with a leap second.
    fn day_has_leap_second(&self, k: i64) -> Result<bool, Error> {
        for e in self.entries.iter().skip(1) {
            if entry_day(e)? == k + 1 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Returns the length in SI seconds of civil day `k` (86400 or 86401).
    fn day_length(&self, k: i64) -> Result<f64, Error> {
        Ok(if self.day_has_leap_second(k)? {
            SECONDS_PER_DAY + 1.0
        } else {
            SECONDS_PER_DAY
        })
    }

    /// Checks that civil day `k` is inside the table's coverage.
    fn check_day(&self, k: i64) -> Result<(), Error> {
        if k < self.first_day()? {
            return Err(Error::BeforeLeapSecondTable);
        }
        if k > self.until_day()? {
            return Err(Error::LeapSecondTableExpired);
        }
        Ok(())
    }
}

/// Returns the civil day number of a table entry (Gregorian midnight).
fn entry_day(e: &LeapSecond) -> Result<i64, Error> {
    day_number(
        CivilDate::new(e.year, e.month, e.day),
        CalendarSystem::Gregorian,
    )
}

/// Converts a UTC Julian Date (scaled leap-second day convention) to TAI.
pub(crate) fn utc_to_tai(
    table: &LeapSecondTable<'_>,
    utc: JulianDate,
) -> Result<JulianDate, Error> {
    let (k, fs) = civil_day_and_fraction(utc);
    table.check_day(k)?;
    let len = table.day_length(k)?;
    let secs_utc = fs * len;
    let off = table.offset_on_day(k)? as f64;
    Ok(JulianDate::new(
        k as f64,
        0.5 + (secs_utc + off) / SECONDS_PER_DAY,
    ))
}

/// Converts a TAI Julian Date to UTC (scaled leap-second day convention).
pub(crate) fn tai_to_utc(
    table: &LeapSecondTable<'_>,
    tai: JulianDate,
) -> Result<JulianDate, Error> {
    let mut off: Option<f64> = None;
    for (i, e) in table.entries.iter().enumerate() {
        let k = entry_day(e)?;
        let new = f64::from(e.tai_minus_utc);
        // TAI instant at which this entry takes effect (UTC midnight, TAI clock).
        let start = JulianDate::new(k as f64, 0.5).add_seconds(new);
        if i > 0 {
            // The leap second 23:59:60 occupies the TAI second before `start`.
            let leap_begin = start.add_seconds(-1.0);
            if tai >= leap_begin && tai < start {
                let secs = SECONDS_PER_DAY + tai.seconds_since(leap_begin);
                let result = JulianDate::new((k - 1) as f64, 0.5 + secs / (SECONDS_PER_DAY + 1.0));
                table.check_day(k - 1)?;
                return Ok(result);
            }
        }
        if tai >= start {
            off = Some(new);
        } else {
            break;
        }
    }
    let off = off.ok_or(Error::BeforeLeapSecondTable)?;
    let naive = tai.add_seconds(-off);
    let (mut k, mut fd) = civil_day_and_fraction(naive);
    // Snap residues within SNAP_SECONDS of midnight onto the next midnight, so
    // that rounding cannot place an exact midnight on the previous (leap) day.
    if (1.0 - fd) * SECONDS_PER_DAY < SNAP_SECONDS {
        k += 1;
        fd = 0.0;
    }
    table.check_day(k)?;
    let len = table.day_length(k)?;
    Ok(JulianDate::new(k as f64, 0.5 + fd * SECONDS_PER_DAY / len))
}

/// Returns the UTC civil day number and seconds since midnight for a UTC
/// Julian Date, using the table's day length.
pub(crate) fn utc_day_and_seconds(
    table: &LeapSecondTable<'_>,
    utc: JulianDate,
) -> Result<(CivilDate, f64), Error> {
    let (mut k, fs) = civil_day_and_fraction(utc);
    let len = table.day_length(k)?;
    let mut secs = fs * len;
    if len - secs < SNAP_SECONDS {
        k += 1;
        secs = 0.0;
    }
    table.check_day(k)?;
    Ok((date_from_day_number(k, CalendarSystem::Gregorian), secs))
}

/// Builds a UTC Julian Date from a civil date and seconds since midnight.
///
/// `secs` may be in `[0, 86401)` only on a leap day, and `[0, 86400)`
/// otherwise.
pub(crate) fn utc_from_day_and_seconds(
    table: &LeapSecondTable<'_>,
    date: CivilDate,
    secs: f64,
) -> Result<JulianDate, Error> {
    let k = day_number(date, CalendarSystem::Gregorian)?;
    table.check_day(k)?;
    let len = table.day_length(k)?;
    if !(0.0..len).contains(&secs) {
        return Err(if secs >= SECONDS_PER_DAY {
            Error::LeapSecondNotInTable
        } else {
            Error::InvalidTime
        });
    }
    Ok(JulianDate::new(k as f64, 0.5 + secs / len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jd::JulianDate;

    fn utc_midnight(y: i32, m: u8, d: u8) -> JulianDate {
        JulianDate::from_calendar(CivilDate::new(y, m, d), CalendarSystem::Gregorian, 0.0).unwrap()
    }

    /// Compares with a tolerance of 1e-9 s; the two-part representation resolves about 1e-11 s.
    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    fn tai_offset_at(y: i32, m: u8, d: u8) -> f64 {
        // Round-trip a UTC midnight to TAI and read the offset back.
        let utc = utc_midnight(y, m, d);
        let tai = utc_to_tai(&EMBEDDED, utc).unwrap();
        tai.seconds_since(utc)
    }

    #[test]
    fn embedded_table_is_valid_and_complete() {
        assert!(EMBEDDED.validate().is_ok());
        assert_eq!(EMBEDDED.entries().len(), 28);
        assert_eq!(EMBEDDED.entries()[0].tai_minus_utc, 10);
        assert_eq!(EMBEDDED.entries().last().unwrap().tai_minus_utc, 37);
        assert_eq!(EMBEDDED.version(), 1);
    }

    #[test]
    fn tai_minus_utc_golden_values() {
        assert_close(tai_offset_at(1972, 1, 1), 10.0);
        assert_close(tai_offset_at(1980, 6, 1), 19.0);
        assert_close(tai_offset_at(2000, 1, 1), 32.0);
        // Stated in the task: 37 s after 2017-01-01.
        assert_close(tai_offset_at(2017, 1, 1), 37.0);
        assert_close(tai_offset_at(2020, 6, 1), 37.0);
    }

    #[test]
    fn leap_second_day_is_86401_seconds() {
        // 2016-12-31 is the leap-second day (23:59:60 is followed by 2017-01-01).
        assert_eq!(
            EMBEDDED
                .day_length(
                    day_number(CivilDate::new(2016, 12, 31), CalendarSystem::Gregorian).unwrap()
                )
                .unwrap(),
            86_401.0
        );
        assert_eq!(
            EMBEDDED
                .day_length(
                    day_number(CivilDate::new(2016, 12, 30), CalendarSystem::Gregorian).unwrap()
                )
                .unwrap(),
            86_400.0
        );
    }

    #[test]
    fn leap_second_instant_maps_into_tai_gap() {
        // UTC 2016-12-31 23:59:60.5 is TAI 2017-01-01 00:00:36.5.
        let (k, _) = civil_day_and_fraction(utc_midnight(2016, 12, 31));
        let secs_leap = SECONDS_PER_DAY + 0.5;
        let utc = JulianDate::new(k as f64, 0.5 + secs_leap / (SECONDS_PER_DAY + 1.0));
        let tai = utc_to_tai(&EMBEDDED, utc).unwrap();
        let expected = utc_midnight(2017, 1, 1).add_seconds(36.5);
        assert!((tai.seconds_since(expected)).abs() < 1e-6);
        // And back again.
        let back = tai_to_utc(&EMBEDDED, tai).unwrap();
        assert!((back.days_since(utc)).abs() < 1e-12);
    }

    #[test]
    fn utc_tai_round_trip_across_all_entries() {
        for e in EMBEDDED.entries() {
            let k = entry_day(e).unwrap();
            for offset_secs in [-1.0, 0.0, 0.25, 3_600.0, 86_399.0, 86_400.5] {
                let utc = JulianDate::new(k as f64, 0.5).add_seconds(offset_secs);
                let Ok(tai) = utc_to_tai(&EMBEDDED, utc) else {
                    // Only the first entry can step back before the table.
                    assert!(offset_secs < 0.0 && e == &EMBEDDED.entries()[0]);
                    continue;
                };
                let back = tai_to_utc(&EMBEDDED, tai).unwrap();
                assert!(
                    back.days_since(utc).abs() < 1e-9,
                    "round trip failed at {e:?} +{offset_secs}"
                );
            }
        }
    }

    #[test]
    fn before_table_and_after_expiry_are_errors() {
        assert_eq!(
            utc_to_tai(&EMBEDDED, utc_midnight(1971, 12, 31)),
            Err(Error::BeforeLeapSecondTable)
        );
        assert_eq!(
            utc_to_tai(&EMBEDDED, utc_midnight(2030, 1, 1)),
            Err(Error::LeapSecondTableExpired)
        );
        assert!(EMBEDDED.is_expired_at(utc_midnight(2030, 1, 1)));
        assert!(!EMBEDDED.is_expired_at(utc_midnight(2017, 1, 1)));
    }

    #[test]
    fn custom_table_validation_rejects_bad_input() {
        let unsorted = [
            LeapSecond {
                year: 2017,
                month: 1,
                day: 1,
                tai_minus_utc: 37,
            },
            LeapSecond {
                year: 2015,
                month: 7,
                day: 1,
                tai_minus_utc: 36,
            },
        ];
        assert_eq!(
            LeapSecondTable::new(&unsorted, CivilDate::new(2018, 1, 1), 1, "x"),
            Err(Error::InvalidLeapSecondTable)
        );
        let gap = [
            LeapSecond {
                year: 2015,
                month: 7,
                day: 1,
                tai_minus_utc: 36,
            },
            LeapSecond {
                year: 2017,
                month: 1,
                day: 1,
                tai_minus_utc: 38,
            },
        ];
        assert_eq!(
            LeapSecondTable::new(&gap, CivilDate::new(2018, 1, 1), 1, "x"),
            Err(Error::InvalidLeapSecondTable)
        );
        let early_expiry = [LeapSecond {
            year: 2017,
            month: 1,
            day: 1,
            tai_minus_utc: 37,
        }];
        assert_eq!(
            LeapSecondTable::new(&early_expiry, CivilDate::new(2016, 1, 1), 1, "x"),
            Err(Error::InvalidLeapSecondTable)
        );
    }
}
