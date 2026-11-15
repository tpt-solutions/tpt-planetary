//! Time scales as typed markers, and typed epochs.
//!
//! An [`Epoch<S>`] is an instant on time scale `S`, where `S` is one of the
//! zero-sized markers [`Utc`], [`Tai`] or [`Tt`]. The compiler rejects
//! arithmetic or comparisons between epochs on different scales. Conversions go
//! through [`Epoch::to_scale`], which is available for each supported pair.
//!
//! | From \ To | UTC | TAI | TT |
//! |-----------|-----|-----|----|
//! | UTC       | id  | leap-second table | via TAI |
//! | TAI       | leap-second table | id | `+ 32.184 s` |
//! | TT        | via TAI | `- 32.184 s` | id |
//!
//! TDB is not implemented in this module.
//!
//! ```
//! use tpt_planetary_core::leap::EMBEDDED;
//! use tpt_planetary_core::scale::{Epoch, Tai, Tt, Utc};
//! use tpt_planetary_core::calendar::CivilDate;
//!
//! // 2000-01-01 12:00 TT, the J2000.0 epoch.
//! let tt = Epoch::<Tt>::from_jd(tpt_planetary_core::jd::JulianDate::J2000);
//! let tai = tt.to_scale::<Tai>(&EMBEDDED)?;
//! let utc = tai.to_scale::<Utc>(&EMBEDDED)?;
//! assert_eq!(utc.to_civil(&EMBEDDED)?.date, CivilDate::new(2000, 1, 1));
//! # Ok::<(), tpt_planetary_core::Error>(())
//! ```

use core::fmt;
use core::marker::PhantomData;

use crate::calendar::{CalendarSystem, CivilDate};
use crate::constants::{SECONDS_PER_DAY, TT_MINUS_TAI_SECONDS};
use crate::error::Error;
use crate::jd::JulianDate;
use crate::leap::{self, LeapSecondTable};

mod sealed {
    pub trait Sealed {}
}

/// A time scale marker. Implemented only by [`Utc`], [`Tai`] and [`Tt`].
pub trait TimeScale: sealed::Sealed + Copy + fmt::Debug + PartialEq + PartialOrd {
    /// Short name of the scale, for display.
    const NAME: &'static str;
}

/// Coordinated Universal Time. Carries leap seconds; see [`crate::leap`].
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Hash, Default)]
pub struct Utc;

/// International Atomic Time. Continuous; no leap seconds.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Hash, Default)]
pub struct Tai;

/// Terrestrial Time, defined as TAI + 32.184 s.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Hash, Default)]
pub struct Tt;

impl sealed::Sealed for Utc {}
impl sealed::Sealed for Tai {}
impl sealed::Sealed for Tt {}

impl TimeScale for Utc {
    const NAME: &'static str = "UTC";
}
impl TimeScale for Tai {
    const NAME: &'static str = "TAI";
}
impl TimeScale for Tt {
    const NAME: &'static str = "TT";
}

/// Conversion from scale `Self` to scale `To`.
///
/// Implemented for every supported pair. Prefer [`Epoch::to_scale`] in code.
pub trait ScaleConvert<To: TimeScale>: TimeScale {
    /// Converts `epoch` from `Self` to `To`, using `table` for UTC.
    fn convert(epoch: Epoch<Self>, table: &LeapSecondTable<'_>) -> Result<Epoch<To>, Error>;
}

impl<S: TimeScale> ScaleConvert<S> for S {
    fn convert(epoch: Epoch<S>, _table: &LeapSecondTable<'_>) -> Result<Epoch<S>, Error> {
        Ok(epoch)
    }
}

impl ScaleConvert<Tai> for Utc {
    fn convert(epoch: Epoch<Utc>, table: &LeapSecondTable<'_>) -> Result<Epoch<Tai>, Error> {
        Ok(Epoch::from_jd(leap::utc_to_tai(table, epoch.jd)?))
    }
}

impl ScaleConvert<Utc> for Tai {
    fn convert(epoch: Epoch<Tai>, table: &LeapSecondTable<'_>) -> Result<Epoch<Utc>, Error> {
        Ok(Epoch::from_jd(leap::tai_to_utc(table, epoch.jd)?))
    }
}

impl ScaleConvert<Tt> for Tai {
    fn convert(epoch: Epoch<Tai>, _table: &LeapSecondTable<'_>) -> Result<Epoch<Tt>, Error> {
        Ok(Epoch::from_jd(epoch.jd.add_seconds(TT_MINUS_TAI_SECONDS)))
    }
}

impl ScaleConvert<Tai> for Tt {
    fn convert(epoch: Epoch<Tt>, _table: &LeapSecondTable<'_>) -> Result<Epoch<Tai>, Error> {
        Ok(Epoch::from_jd(epoch.jd.add_seconds(-TT_MINUS_TAI_SECONDS)))
    }
}

impl ScaleConvert<Tt> for Utc {
    fn convert(epoch: Epoch<Utc>, table: &LeapSecondTable<'_>) -> Result<Epoch<Tt>, Error> {
        let tai: Epoch<Tai> = epoch.to_scale(table)?;
        tai.to_scale(table)
    }
}

impl ScaleConvert<Utc> for Tt {
    fn convert(epoch: Epoch<Tt>, table: &LeapSecondTable<'_>) -> Result<Epoch<Utc>, Error> {
        let tai: Epoch<Tai> = epoch.to_scale(table)?;
        tai.to_scale(table)
    }
}

/// A civil time of day with calendar date.
///
/// For UTC on a leap day, `second` may be in `[60, 61)` with `hour = 23` and
/// `minute = 59`, which is the leap second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CivilTime {
    /// Gregorian calendar date.
    pub date: CivilDate,
    /// Hour, 0 to 23.
    pub hour: u8,
    /// Minute, 0 to 59.
    pub minute: u8,
    /// Second and fraction of a second.
    pub second: f64,
}

/// An instant on time scale `S`, stored as a two-part Julian Date.
///
/// For [`Utc`], the Julian Date uses the leap-second day convention documented
/// in [`crate::leap`]. Arithmetic is only offered for [`Tai`] and [`Tt`],
/// where seconds are uniform.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Epoch<S: TimeScale> {
    jd: JulianDate,
    scale: PhantomData<S>,
}

impl<S: TimeScale> Epoch<S> {
    /// Wraps a Julian Date that is already on scale `S`.
    #[inline]
    pub fn from_jd(jd: JulianDate) -> Self {
        Epoch {
            jd,
            scale: PhantomData,
        }
    }

    /// Returns the underlying Julian Date (on scale `S`).
    #[inline]
    pub fn jd(self) -> JulianDate {
        self.jd
    }

    /// Returns the time scale's name.
    pub fn scale_name(self) -> &'static str {
        S::NAME
    }

    /// Converts this epoch to scale `T`.
    ///
    /// # Errors
    ///
    /// For conversions involving UTC, returns [`Error::BeforeLeapSecondTable`]
    /// or [`Error::LeapSecondTableExpired`] when the instant is outside the
    /// table's coverage.
    ///
    /// ```
    /// use tpt_planetary_core::leap::EMBEDDED;
    /// use tpt_planetary_core::scale::{Epoch, Tai, Tt};
    /// let tai = Epoch::<Tai>::from_jd(tpt_planetary_core::jd::JulianDate::J2000);
    /// let tt: Epoch<Tt> = tai.to_scale(&EMBEDDED)?;
    /// assert!((tt.jd().seconds_since(tai.jd()) - 32.184).abs() < 1e-9);
    /// # Ok::<(), tpt_planetary_core::Error>(())
    /// ```
    pub fn to_scale<T: TimeScale>(self, table: &LeapSecondTable<'_>) -> Result<Epoch<T>, Error>
    where
        S: ScaleConvert<T>,
    {
        <S as ScaleConvert<T>>::convert(self, table)
    }
}

impl Epoch<Tai> {
    /// Adds SI seconds to this epoch.
    pub fn add_seconds(self, seconds: f64) -> Self {
        Epoch::from_jd(self.jd.add_seconds(seconds))
    }

    /// Returns `self - other` in SI seconds.
    pub fn seconds_since(self, other: Epoch<Tai>) -> f64 {
        self.jd.seconds_since(other.jd)
    }

    /// Builds a TAI epoch from a civil date and time (seconds in `[0, 60)`).
    ///
    /// ```
    /// use tpt_planetary_core::calendar::CivilDate;
    /// use tpt_planetary_core::scale::{CivilTime, Epoch, Tai};
    /// let t = Epoch::<Tai>::from_civil(CivilTime {
    ///     date: CivilDate::new(2000, 1, 1), hour: 11, minute: 59, second: 27.816,
    /// })?;
    /// // TT 12:00:00 is TAI 11:59:27.816, so this TAI reading is 32.184 s before J2000.0.
    /// let expected = 2_451_545.0 - 32.184 / 86_400.0;
    /// assert!((t.jd().to_f64() - expected).abs() < 1e-9);
    /// # Ok::<(), tpt_planetary_core::Error>(())
    /// ```
    pub fn from_civil(ct: CivilTime) -> Result<Self, Error> {
        Ok(Epoch::from_jd(civil_to_jd(ct)?))
    }

    /// Returns the civil date and time of this epoch on the TAI clock.
    pub fn to_civil(self) -> Result<CivilTime, Error> {
        jd_to_civil(self.jd)
    }
}

impl Epoch<Tt> {
    /// Adds SI seconds to this epoch.
    pub fn add_seconds(self, seconds: f64) -> Self {
        Epoch::from_jd(self.jd.add_seconds(seconds))
    }

    /// Returns `self - other` in SI seconds.
    pub fn seconds_since(self, other: Epoch<Tt>) -> f64 {
        self.jd.seconds_since(other.jd)
    }

    /// Builds a TT epoch from a civil date and time (seconds in `[0, 60)`).
    pub fn from_civil(ct: CivilTime) -> Result<Self, Error> {
        Ok(Epoch::from_jd(civil_to_jd(ct)?))
    }

    /// Returns the civil date and time of this epoch on the TT clock.
    pub fn to_civil(self) -> Result<CivilTime, Error> {
        jd_to_civil(self.jd)
    }
}

impl Epoch<Utc> {
    /// Builds a UTC epoch from a civil date and time.
    ///
    /// A `second` in `[60, 61)` is accepted only as `23:59:6x` on a day that
    /// ends with a leap second according to `table`.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidDate`] or [`Error::InvalidTime`] for an invalid field,
    /// [`Error::LeapSecondNotInTable`] for a leap second that the table does
    /// not contain, and the table coverage errors of [`Epoch::to_scale`].
    ///
    /// ```
    /// use tpt_planetary_core::calendar::CivilDate;
    /// use tpt_planetary_core::leap::EMBEDDED;
    /// use tpt_planetary_core::scale::{CivilTime, Epoch, Tai, Utc};
    ///
    /// let leap = Epoch::<Utc>::from_civil(
    ///     CivilTime { date: CivilDate::new(2016, 12, 31), hour: 23, minute: 59, second: 60.5 },
    ///     &EMBEDDED,
    /// )?;
    /// let tai: Epoch<Tai> = leap.to_scale(&EMBEDDED)?;
    /// let next_day = Epoch::<Utc>::from_civil(
    ///     CivilTime { date: CivilDate::new(2017, 1, 1), hour: 0, minute: 0, second: 0.0 },
    ///     &EMBEDDED,
    /// )?;
    /// let gap: Epoch<Tai> = next_day.to_scale(&EMBEDDED)?;
    /// assert!((gap.seconds_since(tai) - 0.5).abs() < 1e-6);
    /// # Ok::<(), tpt_planetary_core::Error>(())
    /// ```
    pub fn from_civil(ct: CivilTime, table: &LeapSecondTable<'_>) -> Result<Self, Error> {
        if !(ct.hour < 24 && ct.minute < 60 && (0.0..61.0).contains(&ct.second)) {
            return Err(Error::InvalidTime);
        }
        if !ct.date.is_valid(CalendarSystem::Gregorian) {
            return Err(Error::InvalidDate);
        }
        if ct.second >= 60.0 && (ct.hour != 23 || ct.minute != 59) {
            return Err(Error::InvalidTime);
        }
        let secs = f64::from(ct.hour) * 3600.0 + f64::from(ct.minute) * 60.0 + ct.second;
        Ok(Epoch::from_jd(leap::utc_from_day_and_seconds(
            table, ct.date, secs,
        )?))
    }

    /// Returns the civil date and time of this UTC epoch.
    ///
    /// On a leap-second day the seconds field may be in `[60, 61)` with
    /// `hour = 23` and `minute = 59`.
    pub fn to_civil(self, table: &LeapSecondTable<'_>) -> Result<CivilTime, Error> {
        let (date, secs) = leap::utc_day_and_seconds(table, self.jd)?;
        Ok(seconds_to_civil(date, secs))
    }
}

/// Builds a Julian Date from a civil time on the Gregorian calendar, with no
/// leap-second handling. Seconds must be in `[0, 60)`.
fn civil_to_jd(ct: CivilTime) -> Result<JulianDate, Error> {
    if !(ct.hour < 24 && ct.minute < 60 && (0.0..60.0).contains(&ct.second)) {
        return Err(Error::InvalidTime);
    }
    let secs = f64::from(ct.hour) * 3600.0 + f64::from(ct.minute) * 60.0 + ct.second;
    JulianDate::from_calendar(ct.date, CalendarSystem::Gregorian, secs)
}

/// Returns the civil time of a Julian Date on the uniform (non-leap) clock.
fn jd_to_civil(jd: JulianDate) -> Result<CivilTime, Error> {
    let (date, secs) = jd.to_calendar(CalendarSystem::Gregorian)?;
    Ok(seconds_to_civil(date, secs))
}

/// Splits seconds since midnight into hour, minute and second.
fn seconds_to_civil(date: CivilDate, secs: f64) -> CivilTime {
    let (hour, minute, second) = if secs >= SECONDS_PER_DAY {
        // Leap second: 23:59:60 plus the fraction.
        (23, 59, 60.0 + (secs - SECONDS_PER_DAY))
    } else {
        let h = (secs / 3600.0) as u8;
        let rem = secs - f64::from(h) * 3600.0;
        let m = (rem / 60.0) as u8;
        let s = rem - f64::from(m) * 60.0;
        (h, m, s)
    };
    CivilTime {
        date,
        hour,
        minute,
        second,
    }
}

impl<S: TimeScale> fmt::Display for Epoch<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JD {} {}", self.jd.to_f64(), S::NAME)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leap::EMBEDDED;

    fn civil(y: i32, m: u8, d: u8, h: u8, mi: u8, s: f64) -> CivilTime {
        CivilTime {
            date: CivilDate::new(y, m, d),
            hour: h,
            minute: mi,
            second: s,
        }
    }

    #[test]
    fn tt_is_tai_plus_32_184_seconds() {
        let tai = Epoch::<Tai>::from_civil(civil(2000, 1, 1, 0, 0, 0.0)).unwrap();
        let tt: Epoch<Tt> = tai.to_scale(&EMBEDDED).unwrap();
        assert!((tt.seconds_since(Epoch::from_jd(tai.jd())) - 32.184).abs() < 1e-9);
        let back: Epoch<Tai> = tt.to_scale(&EMBEDDED).unwrap();
        assert!(back.seconds_since(tai).abs() < 1e-9);
    }

    #[test]
    fn j2000_epoch_in_all_scales() {
        // 2000-01-01T12:00:00 TT is JD 2451545.0 (J2000.0).
        let tt = Epoch::<Tt>::from_jd(JulianDate::J2000);
        // TAI = TT - 32.184 s, so TAI reads 11:59:27.816.
        let tai: Epoch<Tai> = tt.to_scale(&EMBEDDED).unwrap();
        let tai_civil = tai.to_civil().unwrap();
        assert_eq!((tai_civil.hour, tai_civil.minute), (11, 59));
        assert!((tai_civil.second - 27.816).abs() < 1e-6);
        // TAI - UTC = 32 s in 2000, so UTC reads 11:58:55.816.
        let utc: Epoch<Utc> = tai.to_scale(&EMBEDDED).unwrap();
        let utc_civil = utc.to_civil(&EMBEDDED).unwrap();
        assert_eq!(utc_civil.date, CivilDate::new(2000, 1, 1));
        assert_eq!((utc_civil.hour, utc_civil.minute), (11, 58));
        assert!((utc_civil.second - 55.816).abs() < 1e-6);
    }

    #[test]
    fn tai_minus_utc_is_37_after_2017() {
        let utc = Epoch::<Utc>::from_civil(civil(2017, 1, 1, 0, 0, 0.0), &EMBEDDED).unwrap();
        let tai: Epoch<Tai> = utc.to_scale(&EMBEDDED).unwrap();
        let expected = Epoch::<Tai>::from_civil(civil(2017, 1, 1, 0, 0, 37.0)).unwrap();
        assert!(tai.seconds_since(expected).abs() < 1e-6);
    }

    #[test]
    fn utc_leap_second_round_trip_through_tai() {
        let leap = Epoch::<Utc>::from_civil(civil(2016, 12, 31, 23, 59, 60.25), &EMBEDDED).unwrap();
        let tai: Epoch<Tai> = leap.to_scale(&EMBEDDED).unwrap();
        // The leap second is TAI 2017-01-01 00:00:36.25.
        let expected = Epoch::<Tai>::from_civil(civil(2017, 1, 1, 0, 0, 36.25)).unwrap();
        assert!(tai.seconds_since(expected).abs() < 1e-6);
        let back = tai.to_civil().unwrap();
        assert_eq!(back.date, CivilDate::new(2017, 1, 1));
        let utc_back: Epoch<Utc> = tai.to_scale(&EMBEDDED).unwrap();
        let c = utc_back.to_civil(&EMBEDDED).unwrap();
        assert_eq!(
            (c.date, c.hour, c.minute),
            (CivilDate::new(2016, 12, 31), 23, 59)
        );
        assert!((c.second - 60.25).abs() < 1e-6);
    }

    #[test]
    fn leap_seconds_rejected_on_ordinary_days() {
        assert_eq!(
            Epoch::<Utc>::from_civil(civil(2016, 12, 30, 23, 59, 60.0), &EMBEDDED).err(),
            Some(Error::LeapSecondNotInTable)
        );
        assert_eq!(
            Epoch::<Utc>::from_civil(civil(2016, 12, 31, 12, 0, 60.0), &EMBEDDED).err(),
            Some(Error::InvalidTime)
        );
        assert_eq!(
            Epoch::<Utc>::from_civil(civil(2016, 2, 30, 0, 0, 0.0), &EMBEDDED).err(),
            Some(Error::InvalidDate)
        );
    }

    #[test]
    fn utc_before_1972_is_an_error() {
        let r = Epoch::<Utc>::from_civil(civil(1971, 6, 1, 0, 0, 0.0), &EMBEDDED);
        assert_eq!(r.err(), Some(Error::BeforeLeapSecondTable));
        let jd =
            JulianDate::from_calendar(CivilDate::new(1971, 6, 1), CalendarSystem::Gregorian, 0.0)
                .unwrap();
        let e: Result<Epoch<Tai>, Error> = Epoch::<Utc>::from_jd(jd).to_scale(&EMBEDDED);
        assert_eq!(e.err(), Some(Error::BeforeLeapSecondTable));
    }

    #[test]
    fn utc_to_tt_composes_through_tai() {
        let utc = Epoch::<Utc>::from_civil(civil(2000, 1, 1, 12, 0, 0.0), &EMBEDDED).unwrap();
        let tt: Epoch<Tt> = utc.to_scale(&EMBEDDED).unwrap();
        // 12:00:00 UTC is 12:01:04.184 TT in 2000 (TT - UTC = 32.184 + 32).
        let expected = Epoch::<Tt>::from_civil(civil(2000, 1, 1, 12, 1, 4.184)).unwrap();
        assert!(tt.seconds_since(expected).abs() < 1e-6);
        let back: Epoch<Utc> = tt.to_scale(&EMBEDDED).unwrap();
        assert!(back.jd().days_since(utc.jd()).abs() < 1e-12);
    }

    #[test]
    fn identity_conversion_is_exact() {
        let t = Epoch::<Tai>::from_jd(JulianDate::new(2_451_545.0, 0.25));
        let same: Epoch<Tai> = t.to_scale(&EMBEDDED).unwrap();
        assert_eq!(same, t);
    }

    #[test]
    fn scale_names_and_display() {
        assert_eq!(Epoch::<Tt>::from_jd(JulianDate::J2000).scale_name(), "TT");
        assert_eq!(Utc::NAME, "UTC");
        let midnight_2000 = JulianDate::new(2_451_544.0, 0.5);
        assert_eq!(
            midnight_2000
                .to_calendar(CalendarSystem::Gregorian)
                .unwrap()
                .0,
            CivilDate::new(2000, 1, 1)
        );
    }
}
