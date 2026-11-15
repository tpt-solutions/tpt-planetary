//! Minimal ISO 8601 handling for `YYYY-MM-DDTHH:MM:SS[.fraction]`.
//!
//! No time-zone designator is accepted: the scale of the timestamp is always
//! given separately on the command line. Parsing is done by hand so that the
//! CLI needs no date-time crate.

use tpt_planetary_core::calendar::CivilDate;
use tpt_planetary_core::scale::CivilTime;

/// Parses `YYYY-MM-DDTHH:MM:SS` with an optional fraction of seconds.
///
/// Field ranges (month, day, leap seconds) are checked later by
/// `tpt-planetary-core`, so this function only checks the syntax.
pub fn parse_timestamp(text: &str) -> Result<CivilTime, String> {
    let b = text.as_bytes();
    let shape = "YYYY-MM-DDTHH:MM:SS[.fraction]";
    let bad =
        || format!("invalid timestamp '{text}': expected {shape} with no time-zone designator");
    if b.len() < 19 {
        return Err(bad());
    }
    let expect = |i: usize, c: u8| if b[i] == c { Ok(()) } else { Err(bad()) };
    expect(4, b'-')?;
    expect(7, b'-')?;
    expect(10, b'T')?;
    expect(13, b':')?;
    expect(16, b':')?;
    let num = |range: std::ops::Range<usize>| -> Result<u32, String> {
        let digits = &b[range];
        if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
            return Err(bad());
        }
        let mut v: u32 = 0;
        for d in digits {
            v = v * 10 + u32::from(d - b'0');
        }
        Ok(v)
    };
    let year = i32::try_from(num(0..4)?).map_err(|_| bad())?;
    let month = num(5..7)?;
    let day = num(8..10)?;
    let hour = num(11..13)?;
    let minute = num(14..16)?;
    let whole = num(17..19)?;
    let mut second = f64::from(whole);
    let rest = &text[19..];
    if !rest.is_empty() {
        let frac = rest.strip_prefix('.').ok_or_else(bad)?;
        if frac.is_empty() || !frac.bytes().all(|c| c.is_ascii_digit()) {
            return Err(bad());
        }
        let parsed: f64 = format!("0.{frac}").parse().map_err(|_| bad())?;
        second += parsed;
    }
    let to_u8 = |v: u32| u8::try_from(v).map_err(|_| bad());
    Ok(CivilTime {
        date: CivilDate::new(year, to_u8(month)?, to_u8(day)?),
        hour: to_u8(hour)?,
        minute: to_u8(minute)?,
        second,
    })
}

/// Formats a civil time as `YYYY-MM-DDTHH:MM:SS`, adding up to nine fractional
/// digits (trailing zeros removed) when the seconds are not whole.
pub fn format_timestamp(ct: &CivilTime) -> String {
    let total_ns = (ct.second * 1e9).round() as i64;
    let whole = total_ns / 1_000_000_000;
    let nanos = total_ns % 1_000_000_000;
    let mut out = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        ct.date.year, ct.date.month, ct.date.day, ct.hour, ct.minute, whole
    );
    if nanos != 0 {
        let digits = format!("{nanos:09}");
        out.push('.');
        out.push_str(digits.trim_end_matches('0'));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_whole_and_fractional_seconds() {
        let t = parse_timestamp("2017-01-01T00:00:00").unwrap();
        assert_eq!(t.date, CivilDate::new(2017, 1, 1));
        assert_eq!((t.hour, t.minute, t.second), (0, 0, 0.0));
        let f = parse_timestamp("2000-06-15T12:30:45.25").unwrap();
        assert_eq!((f.hour, f.minute), (12, 30));
        assert!((f.second - 45.25).abs() < 1e-12);
    }

    #[test]
    fn rejects_bad_syntax() {
        for bad in [
            "2017-01-01",
            "2017-01-01 00:00:00",
            "2017-01-01T00:00:00Z",
            "2017-01-01T00:00:00.",
            "2017-1-01T00:00:00",
            "abcd-01-01T00:00:00",
            "2017-01-01T00:00:00.5x",
        ] {
            assert!(parse_timestamp(bad).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn formats_without_spurious_fraction() {
        let t = CivilTime {
            date: CivilDate::new(2017, 1, 1),
            hour: 0,
            minute: 0,
            second: 37.0,
        };
        assert_eq!(format_timestamp(&t), "2017-01-01T00:00:37");
        let f = CivilTime { second: 9.184, ..t };
        assert_eq!(format_timestamp(&f), "2017-01-01T00:00:09.184");
    }
}
