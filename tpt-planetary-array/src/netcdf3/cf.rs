//! CF-convention helpers for time coordinates.
//!
//! A time variable carries a `units` attribute of the form
//! `"<unit> since <YYYY-MM-DD>[ <HH:MM[:SS[.fff]]>]"` and optionally a
//! `calendar` attribute. This module parses those strings and converts
//! numeric offsets to calendar dates, and the reverse.
//!
//! Supported units: `days`/`day`/`d`, `hours`/`hour`/`hr`/`h`,
//! `minutes`/`minute`/`min`, `seconds`/`second`/`sec`/`s`.
//! Supported calendars: `standard` and `gregorian` (Julian before
//! 1582-10-15, Gregorian after), `proleptic_gregorian`, `julian`, `noleap` and
//! `365_day`, `all_leap` and `366_day`, and `360_day` (twelve 30-day months).
//! Units such as months or years (which are not fixed lengths in CF) are
//! rejected. Time zone suffixes are rejected. Reference dates must be on or
//! after year -4700.
//!
//! The date arithmetic is private (Julian day numbers), so this module does
//! not depend on `tpt-planetary-core`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::Reader;
use crate::error::{Error, Result};

/// A calendar supported for CF time coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Calendar {
    /// Julian before 1582-10-15, Gregorian after (CF `standard`/`gregorian`).
    Standard,
    /// Gregorian rules at all dates (CF `proleptic_gregorian`).
    ProlepticGregorian,
    /// Every year has 365 days (CF `noleap`/`365_day`).
    NoLeap,
    /// Every year has 366 days (CF `all_leap`/`366_day`).
    AllLeap,
    /// Every year has twelve months of 30 days (CF `360_day`).
    Days360,
    /// Julian rules at all dates (CF `julian`).
    Julian,
}

impl Calendar {
    /// Parses a CF `calendar` attribute value.
    pub fn from_name(name: &str) -> Result<Self> {
        match name {
            "standard" | "gregorian" => Ok(Calendar::Standard),
            "proleptic_gregorian" => Ok(Calendar::ProlepticGregorian),
            "noleap" | "365_day" => Ok(Calendar::NoLeap),
            "all_leap" | "366_day" => Ok(Calendar::AllLeap),
            "360_day" => Ok(Calendar::Days360),
            "julian" => Ok(Calendar::Julian),
            other => Err(Error::Unsupported(format!("CF calendar '{other}'"))),
        }
    }
}

/// The unit of a CF time coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeUnit {
    /// Days of 86400 seconds.
    Days,
    /// Hours of 3600 seconds.
    Hours,
    /// Minutes of 60 seconds.
    Minutes,
    /// Seconds.
    Seconds,
}

impl TimeUnit {
    /// Parses the unit word of a `units` string (case-insensitive).
    pub fn from_name(name: &str) -> Option<Self> {
        let is = |options: &[&str]| options.iter().any(|o| name.eq_ignore_ascii_case(o));
        if is(&["days", "day", "d"]) {
            Some(TimeUnit::Days)
        } else if is(&["hours", "hour", "hr", "h"]) {
            Some(TimeUnit::Hours)
        } else if is(&["minutes", "minute", "min"]) {
            Some(TimeUnit::Minutes)
        } else if is(&["seconds", "second", "sec", "s"]) {
            Some(TimeUnit::Seconds)
        } else {
            None
        }
    }

    /// Length of one unit in seconds.
    pub fn seconds(self) -> f64 {
        match self {
            TimeUnit::Days => 86_400.0,
            TimeUnit::Hours => 3_600.0,
            TimeUnit::Minutes => 60.0,
            TimeUnit::Seconds => 1.0,
        }
    }
}

/// A date and time of day in a CF calendar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CfDateTime {
    /// Year (astronomical numbering; year 0 exists).
    pub year: i32,
    /// Month, 1 to 12.
    pub month: u8,
    /// Day of month, 1-based.
    pub day: u8,
    /// Hour, 0 to 23.
    pub hour: u8,
    /// Minute, 0 to 59.
    pub minute: u8,
    /// Second, in `[0, 60)`, with any fraction.
    pub second: f64,
}

impl CfDateTime {
    /// Seconds since midnight.
    pub fn seconds_of_day(&self) -> f64 {
        f64::from(self.hour) * 3600.0 + f64::from(self.minute) * 60.0 + self.second
    }
}

/// Parsed `"<unit> since <reference>"` time units with their calendar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeUnits {
    unit: TimeUnit,
    calendar: Calendar,
    reference: CfDateTime,
}

impl TimeUnits {
    /// Parses a CF `units` string such as `"days since 2000-01-01"` for the
    /// given calendar.
    pub fn parse(units: &str, calendar: Calendar) -> Result<Self> {
        let mut words = units.split_whitespace();
        let unit_word = words
            .next()
            .ok_or_else(|| Error::InvalidMetadata(String::from("empty time units")))?;
        let unit = TimeUnit::from_name(unit_word).ok_or_else(|| {
            Error::Unsupported(format!(
                "time unit '{unit_word}' (only days, hours, minutes and seconds)"
            ))
        })?;
        if words.next() != Some("since") {
            return Err(Error::InvalidMetadata(format!(
                "time units '{units}' lack 'since'"
            )));
        }
        let rest: Vec<&str> = words.collect();
        let (date, time) = match rest.as_slice() {
            [date] => match date.split_once('T') {
                Some((d, t)) => (d, Some(t)),
                None => (*date, None),
            },
            [date, time] => (*date, Some(*time)),
            _ => {
                return Err(Error::InvalidMetadata(format!(
                    "cannot parse reference time in '{units}'"
                )));
            }
        };
        let (year, month, day) = parse_date(date)?;
        let (hour, minute, second) = match time {
            Some(t) => parse_time(t)?,
            None => (0, 0, 0.0),
        };
        validate_date(year, month, day, calendar)?;
        Ok(Self {
            unit,
            calendar,
            reference: CfDateTime {
                year,
                month,
                day,
                hour,
                minute,
                second,
            },
        })
    }

    /// The time unit.
    pub fn unit(&self) -> TimeUnit {
        self.unit
    }

    /// The calendar.
    pub fn calendar(&self) -> Calendar {
        self.calendar
    }

    /// The reference date and time of day.
    pub fn reference(&self) -> CfDateTime {
        self.reference
    }

    /// Converts a numeric offset in this unit to a date and time.
    ///
    /// Results are rounded to the nearest microsecond. Fails for non-finite
    /// offsets, offsets that move the result beyond about 30,000 years of the
    /// reference, and results before year -4700.
    pub fn decode(&self, offset: f64) -> Result<CfDateTime> {
        if !offset.is_finite() {
            return Err(Error::InvalidMetadata(format!(
                "time offset {offset} is not finite"
            )));
        }
        let total = round_micro(self.reference.seconds_of_day() + offset * self.unit.seconds());
        if !(total > -1e12 && total < 1e12) {
            return Err(Error::Unsupported(format!(
                "time offset {offset} is too far from the reference"
            )));
        }
        let mut shift = floor(total / 86_400.0) as i64;
        let mut sod = total - shift as f64 * 86_400.0;
        if sod >= 86_400.0 {
            sod -= 86_400.0;
            shift += 1;
        } else if sod < 0.0 {
            sod += 86_400.0;
            shift -= 1;
        }
        let base = self.calendar.to_count(&self.reference);
        let (year, month, day) = self.calendar.date_of_count(base + shift)?;
        let hour = floor(sod / 3600.0);
        let rem = sod - hour * 3600.0;
        let minute = floor(rem / 60.0);
        let second = rem - minute * 60.0;
        Ok(CfDateTime {
            year: i32::try_from(year)
                .map_err(|_| Error::Unsupported(format!("year {year} out of range")))?,
            month,
            day,
            hour: hour as u8,
            minute: minute as u8,
            second,
        })
    }

    /// Converts a date and time to a numeric offset in this unit. This is the
    /// inverse of [`TimeUnits::decode`] for dates valid in the calendar.
    pub fn encode(&self, time: CfDateTime) -> Result<f64> {
        validate_date(time.year, time.month, time.day, self.calendar)?;
        if time.hour > 23 || time.minute > 59 || !(0.0..60.0).contains(&time.second) {
            return Err(Error::InvalidMetadata(String::from("invalid time of day")));
        }
        let days = self.calendar.to_count(&time) - self.calendar.to_count(&self.reference);
        let seconds =
            days as f64 * 86_400.0 + (time.seconds_of_day() - self.reference.seconds_of_day());
        Ok(seconds / self.unit.seconds())
    }
}

/// Parses the time units of variable `variable` from its `units` and
/// `calendar` attributes. The calendar defaults to `standard`.
pub fn time_units(reader: &Reader<'_>, variable: &str) -> Result<TimeUnits> {
    let var = reader
        .variable(variable)
        .ok_or_else(|| Error::NotFound(format!("variable '{variable}'")))?;
    let units = var
        .attribute("units")
        .and_then(|a| a.values.as_text())
        .ok_or_else(|| {
            Error::InvalidMetadata(format!(
                "variable '{variable}' has no text 'units' attribute"
            ))
        })?;
    let calendar = match var.attribute("calendar").and_then(|a| a.values.as_text()) {
        Some(name) => Calendar::from_name(name)?,
        None => Calendar::Standard,
    };
    TimeUnits::parse(units, calendar)
}

/// Calendar month labels (0 for January to 11 for December) of time offsets,
/// for use as group labels with `Expr::group_mean` (the labels are per index
/// along the time axis, so the result has one entry per offset).
pub fn calendar_month_labels(units: &TimeUnits, offsets: &[f64]) -> Result<Vec<u32>> {
    offsets
        .iter()
        .map(|&o| units.decode(o).map(|t| u32::from(t.month) - 1))
        .collect()
}

/// Reads variable `variable` and decodes each value as a CF time.
pub fn decode_times(reader: &Reader<'_>, variable: &str) -> Result<Vec<CfDateTime>> {
    let units = time_units(reader, variable)?;
    reader
        .read_f64(variable)?
        .into_iter()
        .map(|offset| units.decode(offset))
        .collect()
}

/// Days from 1 January 4713 BC (Julian) of the Gregorian-family calendars,
/// for the first day of the Gregorian reform.
const GREGORIAN_CUTOVER_JDN: i64 = 2_299_161;
/// Earliest year accepted, keeping the day-number formulas non-negative.
const MIN_YEAR: i64 = -4700;
/// Days before the first day of each month in a non-leap year.
const CUMULATIVE_DAYS: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
/// Days before the first day of each month in a leap year.
const CUMULATIVE_DAYS_LEAP: [i64; 12] = [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335];

fn gregorian_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn is_leap(year: i64, calendar: Calendar) -> bool {
    match calendar {
        Calendar::NoLeap | Calendar::Days360 => false,
        Calendar::AllLeap => true,
        Calendar::Julian => year.rem_euclid(4) == 0,
        Calendar::ProlepticGregorian => gregorian_leap(year),
        Calendar::Standard => {
            if year < 1582 {
                year.rem_euclid(4) == 0
            } else {
                gregorian_leap(year)
            }
        }
    }
}

fn days_in_month(year: i64, month: u8, calendar: Calendar) -> u8 {
    if calendar == Calendar::Days360 {
        return 30;
    }
    match month {
        2 if is_leap(year, calendar) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Checks that a civil date exists in `calendar`.
fn validate_date(year: i32, month: u8, day: u8, calendar: Calendar) -> Result<()> {
    let year = i64::from(year);
    if year < MIN_YEAR {
        return Err(Error::Unsupported(format!(
            "reference year {year} is before {MIN_YEAR}"
        )));
    }
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month, calendar) {
        return Err(Error::InvalidMetadata(format!(
            "invalid date {year:04}-{month:02}-{day:02}"
        )));
    }
    if calendar == Calendar::Standard && year == 1582 && month == 10 && (5..=14).contains(&day) {
        return Err(Error::InvalidMetadata(String::from(
            "1582-10-05 to 1582-10-14 do not exist in the standard calendar",
        )));
    }
    Ok(())
}

impl Calendar {
    /// Day number of a validated date. Gregorian-family calendars use Julian
    /// day numbers; `NoLeap` counts days from 0000-01-01.
    fn to_count(self, dt: &CfDateTime) -> i64 {
        let year = i64::from(dt.year);
        let month = i64::from(dt.month);
        let day = i64::from(dt.day);
        match self {
            Calendar::NoLeap => year * 365 + CUMULATIVE_DAYS[(month - 1) as usize] + day - 1,
            Calendar::AllLeap => year * 366 + CUMULATIVE_DAYS_LEAP[(month - 1) as usize] + day - 1,
            Calendar::Days360 => year * 360 + (month - 1) * 30 + day - 1,
            Calendar::Julian => jdn_julian(year, month, day),
            Calendar::ProlepticGregorian => jdn_gregorian(year, month, day),
            Calendar::Standard => {
                if (dt.year, dt.month, dt.day) >= (1582, 10, 15) {
                    jdn_gregorian(year, month, day)
                } else {
                    jdn_julian(year, month, day)
                }
            }
        }
    }

    /// Civil date (year, month, day) of a day number from [`Calendar::to_count`].
    fn date_of_count(self, count: i64) -> Result<(i64, u8, u8)> {
        match self {
            Calendar::NoLeap => {
                let year = count.div_euclid(365);
                let doy = count.rem_euclid(365);
                let mut month = 1usize;
                while month < 12 && CUMULATIVE_DAYS[month] <= doy {
                    month += 1;
                }
                let day = doy - CUMULATIVE_DAYS[month - 1] + 1;
                Ok((year, month as u8, day as u8))
            }
            Calendar::AllLeap => {
                let year = count.div_euclid(366);
                let doy = count.rem_euclid(366);
                let mut month = 1usize;
                while month < 12 && CUMULATIVE_DAYS_LEAP[month] <= doy {
                    month += 1;
                }
                let day = doy - CUMULATIVE_DAYS_LEAP[month - 1] + 1;
                Ok((year, month as u8, day as u8))
            }
            Calendar::Days360 => {
                let year = count.div_euclid(360);
                let doy = count.rem_euclid(360);
                let month = doy / 30 + 1;
                let day = doy % 30 + 1;
                Ok((year, month as u8, day as u8))
            }
            Calendar::Julian => julian_from_jdn(count),
            Calendar::ProlepticGregorian => gregorian_from_jdn(count),
            Calendar::Standard => {
                if count >= GREGORIAN_CUTOVER_JDN {
                    gregorian_from_jdn(count)
                } else {
                    julian_from_jdn(count)
                }
            }
        }
    }
}

/// Julian day number of a Gregorian date (integer arithmetic, `year >= -4700`).
fn jdn_gregorian(year: i64, month: i64, day: i64) -> i64 {
    let a = (14 - month) / 12;
    let y = year + 4800 - a;
    let m = month + 12 * a - 3;
    day + (153 * m + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32_045
}

/// Julian day number of a Julian-calendar date (`year >= -4700`).
fn jdn_julian(year: i64, month: i64, day: i64) -> i64 {
    let a = (14 - month) / 12;
    let y = year + 4800 - a;
    let m = month + 12 * a - 3;
    day + (153 * m + 2) / 5 + 365 * y + y / 4 - 32_083
}

fn gregorian_from_jdn(jdn: i64) -> Result<(i64, u8, u8)> {
    if jdn < 0 {
        return Err(Error::Unsupported(String::from(
            "date before the supported range (year -4700)",
        )));
    }
    let a = jdn + 32_044;
    let b = (4 * a + 3) / 146_097;
    let c = a - 146_097 * b / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - 1461 * d / 4;
    let m = (5 * e + 2) / 153;
    let day = e - (153 * m + 2) / 5 + 1;
    let month = m + 3 - 12 * (m / 10);
    let year = 100 * b + d - 4800 + m / 10;
    Ok((year, month as u8, day as u8))
}

fn julian_from_jdn(jdn: i64) -> Result<(i64, u8, u8)> {
    if jdn < 0 {
        return Err(Error::Unsupported(String::from(
            "date before the supported range (year -4700)",
        )));
    }
    let c = jdn + 32_082;
    let d = (4 * c + 3) / 1461;
    let e = c - 1461 * d / 4;
    let m = (5 * e + 2) / 153;
    let day = e - (153 * m + 2) / 5 + 1;
    let month = m + 3 - 12 * (m / 10);
    let year = d - 4800 + m / 10;
    Ok((year, month as u8, day as u8))
}

/// Parses `YYYY-MM-DD`, with an optional leading minus sign on the year.
fn parse_date(text: &str) -> Result<(i32, u8, u8)> {
    let bad = || Error::InvalidMetadata(format!("cannot parse reference date '{text}'"));
    let (negative, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let mut parts = body.splitn(3, '-');
    let year: i64 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(bad)?;
    let month: u8 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(bad)?;
    let day: u8 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(bad)?;
    let year = if negative { -year } else { year };
    let year = i32::try_from(year).map_err(|_| bad())?;
    Ok((year, month, day))
}

/// Parses `HH:MM` or `HH:MM:SS[.fff]`.
fn parse_time(text: &str) -> Result<(u8, u8, f64)> {
    let bad = || Error::InvalidMetadata(format!("cannot parse reference time '{text}'"));
    let mut parts = text.splitn(3, ':');
    let hour: u8 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(bad)?;
    let minute: u8 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(bad)?;
    let second: f64 = match parts.next() {
        Some(s) => s.parse().map_err(|_| bad())?,
        None => 0.0,
    };
    if hour > 23 || minute > 59 || !(0.0..60.0).contains(&second) {
        return Err(bad());
    }
    Ok((hour, minute, second))
}

/// Largest integer not greater than `x`, for `|x| < 2^62`.
fn floor(x: f64) -> f64 {
    let truncated = x as i64 as f64;
    if truncated > x {
        truncated - 1.0
    } else {
        truncated
    }
}

/// Rounds to the nearest microsecond. Only used for `|x| < 1e12`.
fn round_micro(x: f64) -> f64 {
    floor(x * 1e6 + 0.5) / 1e6
}
