//! `tptplan astro convert`: conversion between UTC, TAI and TT.
//!
//! Only the time scales implemented in `tpt-planetary-core` are supported.
//! TDB and coordinate frames are reported as not implemented (exit 3).

use tpt_planetary_core::Error as CoreError;
use tpt_planetary_core::jd::JulianDate;
use tpt_planetary_core::leap::EMBEDDED;
use tpt_planetary_core::scale::{CivilTime, Epoch, Tai, Tt, Utc};

use crate::iso::{format_timestamp, parse_timestamp};
use crate::json;
use crate::{EXIT_NOT_IMPLEMENTED, EXIT_RUNTIME, EXIT_USAGE, Failure, OutputFormat};

/// Names of scales and frames that are known but not implemented yet.
const UNIMPLEMENTED: &[&str] = &[
    "tdb", "tcb", "tcg", "ut1", "icrs", "fk5", "fk4", "galactic", "ecliptic", "gcrs", "itrs",
    "altaz",
];

/// A time scale supported by this command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    /// Coordinated Universal Time.
    Utc,
    /// International Atomic Time.
    Tai,
    /// Terrestrial Time.
    Tt,
}

impl Scale {
    fn label(self) -> &'static str {
        match self {
            Scale::Utc => "utc",
            Scale::Tai => "tai",
            Scale::Tt => "tt",
        }
    }
}

/// Parses a scale name. Known-but-unimplemented names give exit 3, and other
/// unknown names give exit 2.
pub fn parse_scale(name: &str) -> Result<Scale, Failure> {
    match name.to_ascii_lowercase().as_str() {
        "utc" => Ok(Scale::Utc),
        "tai" => Ok(Scale::Tai),
        "tt" => Ok(Scale::Tt),
        other if UNIMPLEMENTED.contains(&other) => Err(Failure::new(
            EXIT_NOT_IMPLEMENTED,
            format!(
                "scale or frame '{name}' is not implemented yet; astro convert supports \
                 the time scales utc, tai and tt only"
            ),
        )),
        _ => Err(Failure::new(
            EXIT_USAGE,
            format!("unknown time scale '{name}': expected utc, tai or tt"),
        )),
    }
}

/// A timestamp on one of the supported scales.
#[derive(Debug, Clone, Copy)]
enum AnyEpoch {
    Utc(Epoch<Utc>),
    Tai(Epoch<Tai>),
    Tt(Epoch<Tt>),
}

impl AnyEpoch {
    fn jd(self) -> JulianDate {
        match self {
            AnyEpoch::Utc(e) => e.jd(),
            AnyEpoch::Tai(e) => e.jd(),
            AnyEpoch::Tt(e) => e.jd(),
        }
    }

    fn civil(self) -> Result<CivilTime, CoreError> {
        match self {
            AnyEpoch::Utc(e) => e.to_civil(&EMBEDDED),
            AnyEpoch::Tai(e) => e.to_civil(),
            AnyEpoch::Tt(e) => e.to_civil(),
        }
    }
}

fn to_tai(e: AnyEpoch) -> Result<Epoch<Tai>, CoreError> {
    match e {
        AnyEpoch::Utc(u) => u.to_scale(&EMBEDDED),
        AnyEpoch::Tai(t) => Ok(t),
        AnyEpoch::Tt(t) => t.to_scale(&EMBEDDED),
    }
}

fn to_tt(e: AnyEpoch) -> Result<Epoch<Tt>, CoreError> {
    match e {
        AnyEpoch::Utc(u) => u.to_scale(&EMBEDDED),
        AnyEpoch::Tai(t) => t.to_scale(&EMBEDDED),
        AnyEpoch::Tt(t) => Ok(t),
    }
}

fn to_utc(e: AnyEpoch) -> Result<Epoch<Utc>, CoreError> {
    match e {
        AnyEpoch::Utc(u) => Ok(u),
        AnyEpoch::Tai(t) => t.to_scale(&EMBEDDED),
        AnyEpoch::Tt(t) => t.to_scale(&EMBEDDED),
    }
}

fn convert(e: AnyEpoch, to: Scale) -> Result<AnyEpoch, CoreError> {
    Ok(match to {
        Scale::Utc => AnyEpoch::Utc(to_utc(e)?),
        Scale::Tai => AnyEpoch::Tai(to_tai(e)?),
        Scale::Tt => AnyEpoch::Tt(to_tt(e)?),
    })
}

fn core_failure(context: &str, e: CoreError) -> Failure {
    Failure::new(EXIT_RUNTIME, format!("{context}: {e}"))
}

/// Runs the conversion and returns the text to print (without a final newline).
pub fn convert_command(
    value: &str,
    from: &str,
    to: &str,
    format: OutputFormat,
) -> Result<String, Failure> {
    let from = parse_scale(from)?;
    let to = parse_scale(to)?;
    let ct = parse_timestamp(value).map_err(|m| Failure::new(EXIT_USAGE, m))?;

    let input = match from {
        Scale::Utc => Epoch::<Utc>::from_civil(ct, &EMBEDDED).map(AnyEpoch::Utc),
        Scale::Tai => Epoch::<Tai>::from_civil(ct).map(AnyEpoch::Tai),
        Scale::Tt => Epoch::<Tt>::from_civil(ct).map(AnyEpoch::Tt),
    }
    .map_err(|e| core_failure("invalid timestamp", e))?;

    let output = convert(input, to).map_err(|e| core_failure("cannot convert", e))?;
    let out_ct = output
        .civil()
        .map_err(|e| core_failure("cannot format result", e))?;
    let timestamp = format_timestamp(&out_ct);
    let jd = output.jd();

    Ok(match format {
        OutputFormat::Table => format!(
            "{timestamp} {}\njd1 {}\njd2 {}",
            to.label().to_ascii_uppercase(),
            jd.day(),
            jd.fraction()
        ),
        OutputFormat::Json => json::object(&[
            ("scale", json::string(&to.label().to_ascii_uppercase())),
            ("timestamp", json::string(&timestamp)),
            ("jd1", jd.day().to_string()),
            ("jd2", jd.fraction().to_string()),
        ]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(value: &str, from: &str, to: &str) -> String {
        convert_command(value, from, to, OutputFormat::Table).unwrap()
    }

    #[test]
    fn utc_to_tai_adds_leap_seconds() {
        assert!(run("2017-01-01T00:00:00", "utc", "tai").starts_with("2017-01-01T00:00:37 TAI"));
    }

    #[test]
    fn tai_to_utc_subtracts_leap_seconds() {
        // Before the 2017 leap second TAI - UTC was 36 s, so TAI 00:00:00 is UTC 23:59:24.
        assert!(run("2017-01-01T00:00:00", "tai", "utc").starts_with("2016-12-31T23:59:24 UTC"));
        assert!(run("2017-01-01T00:00:37", "tai", "utc").starts_with("2017-01-01T00:00:00 UTC"));
    }

    #[test]
    fn utc_to_tt_adds_tai_offset() {
        assert!(run("2017-01-01T00:00:00", "utc", "tt").starts_with("2017-01-01T00:01:09.184 TT"));
    }

    #[test]
    fn scale_names_are_case_insensitive() {
        assert_eq!(parse_scale("TAI").unwrap(), Scale::Tai);
    }

    #[test]
    fn unimplemented_and_unknown_scales_have_distinct_codes() {
        assert_eq!(parse_scale("icrs").unwrap_err().code, EXIT_NOT_IMPLEMENTED);
        assert_eq!(parse_scale("tdb").unwrap_err().code, EXIT_NOT_IMPLEMENTED);
        assert_eq!(parse_scale("utx").unwrap_err().code, EXIT_USAGE);
    }

    #[test]
    fn json_output_has_fixed_members() {
        let s = convert_command("2017-01-01T00:00:00", "utc", "tai", OutputFormat::Json).unwrap();
        assert!(s.starts_with(r#"{"scale": "TAI", "timestamp": "2017-01-01T00:00:37""#));
    }

    #[test]
    fn leap_second_on_ordinary_day_is_runtime_error() {
        let e =
            convert_command("2016-12-30T23:59:60", "utc", "tai", OutputFormat::Table).unwrap_err();
        assert_eq!(e.code, EXIT_RUNTIME);
    }

    #[test]
    fn before_1972_is_runtime_error() {
        let e =
            convert_command("1960-01-01T00:00:00", "utc", "tai", OutputFormat::Table).unwrap_err();
        assert_eq!(e.code, EXIT_RUNTIME);
    }
}
