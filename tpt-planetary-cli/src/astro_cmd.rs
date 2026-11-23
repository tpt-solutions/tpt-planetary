//! `tptplan astro convert`: conversion between UTC, TAI, TT and TDB, and between
//! the ICRS, Galactic and mean J2000 ecliptic frames.
//!
//! Time scales are the ones implemented in `tpt-planetary-core`. Frame
//! conversions use the public `FrameTransform` impls of `tpt-planetary-astro`:
//! ICRS to and from Galactic, and ICRS to and from the mean ecliptic of J2000.
//! Galactic to ecliptic goes through ICRS. Coordinates are `LON,LAT` in
//! degrees. Other frames and time scales are reported as not implemented (exit 3).

use tpt_planetary_astro::eop::{EopError, EopTable, parse_eop_text};
use tpt_planetary_astro::frames::{
    Atmosphere, Galactic, GeodeticObserver, Icrs, MeanEcliptic, SkyCoord, TransformContext,
    icrs_to_altaz_refracted_at,
};
use tpt_planetary_astro::nutation::{
    TrueEclipticOfDate, icrs_to_true_ecliptic_of_date, true_ecliptic_of_date_to_icrs,
};
use tpt_planetary_astro::precession::{
    MeanEclipticOfDate, icrs_to_mean_ecliptic_of_date, mean_ecliptic_of_date_to_icrs,
};
use tpt_planetary_astro::time::{
    tcb_to_tdb, tcg_to_tt, tdb_to_tcb, tdb_to_tt, tt_to_tcg, tt_to_tdb,
};
use tpt_planetary_core::Error as CoreError;
use tpt_planetary_core::jd::JulianDate;
use tpt_planetary_core::leap::EMBEDDED;
use tpt_planetary_core::scale::{CivilTime, Epoch, Tai, Tt, Utc};

use std::path::Path;

use crate::iso::{format_timestamp, parse_timestamp};
use crate::json;
use crate::{EXIT_NOT_IMPLEMENTED, EXIT_RUNTIME, EXIT_USAGE, Failure, OutputFormat};

/// A coordinate frame supported by `astro convert`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameName {
    Icrs,
    Galactic,
    /// Mean ecliptic of J2000.
    Ecliptic,
    /// Mean ecliptic and equinox of the epoch (needs `--epoch`).
    MeanEclipticDate,
    /// True ecliptic and equinox of the epoch, with nutation (needs `--epoch`).
    TrueEclipticDate,
}

impl FrameName {
    fn label(self) -> &'static str {
        match self {
            FrameName::Icrs => "ICRS",
            FrameName::Galactic => "GALACTIC",
            FrameName::Ecliptic => "ECLIPTIC",
            FrameName::MeanEclipticDate => "MEAN_ECLIPTIC_DATE",
            FrameName::TrueEclipticDate => "TRUE_ECLIPTIC_DATE",
        }
    }

    /// True for frames whose transform depends on the epoch.
    fn needs_epoch(self) -> bool {
        matches!(
            self,
            FrameName::MeanEclipticDate | FrameName::TrueEclipticDate
        )
    }
}

/// The supported frame called `name`, if it is one.
fn frame_name(name: &str) -> Option<FrameName> {
    match name.to_ascii_lowercase().as_str() {
        "icrs" => Some(FrameName::Icrs),
        "galactic" => Some(FrameName::Galactic),
        "ecliptic" => Some(FrameName::Ecliptic),
        "ecliptic-date" => Some(FrameName::MeanEclipticDate),
        "true-ecliptic-date" => Some(FrameName::TrueEclipticDate),
        _ => None,
    }
}

/// Parses `LON,LAT` in degrees.
fn parse_pair(value: &str) -> Result<(f64, f64), Failure> {
    let usage = || {
        Failure::new(
            EXIT_USAGE,
            format!(
                "cannot read coordinates '{value}': expected LON,LAT in degrees, e.g. 10.5,-20"
            ),
        )
    };
    let (lon, lat) = value.split_once(',').ok_or_else(usage)?;
    let lon: f64 = lon.trim().parse().map_err(|_| usage())?;
    let lat: f64 = lat.trim().parse().map_err(|_| usage())?;
    if !lon.is_finite() || !lat.is_finite() {
        return Err(usage());
    }
    Ok((lon, lat))
}

/// The TT epoch as a two-part Julian date `(day, fraction)`, for frames that
/// depend on time.
fn require_epoch(epoch: Option<&str>) -> Result<(f64, f64), Failure> {
    let text = epoch.ok_or_else(|| {
        Failure::new(
            EXIT_USAGE,
            "this frame depends on time: give the epoch with --epoch (TT, ISO 8601)".to_string(),
        )
    })?;
    let ct = parse_timestamp(text).map_err(|m| Failure::new(EXIT_USAGE, m))?;
    let tt = Epoch::<Tt>::from_civil(ct).map_err(|e| core_failure("invalid --epoch", e))?;
    let jd = tt.jd();
    Ok((jd.day(), jd.fraction()))
}

/// Converts a `LON,LAT` pair between two supported frames. `epoch` is the TT
/// epoch as a two-part Julian date, needed by the frames that depend on time.
fn frame_convert(
    value: &str,
    from: FrameName,
    to: FrameName,
    epoch: Option<(f64, f64)>,
    format: OutputFormat,
) -> Result<String, Failure> {
    let (lon, lat) = parse_pair(value)?;
    let at = || {
        epoch.ok_or_else(|| {
            Failure::new(
                EXIT_USAGE,
                "this frame depends on time: give the epoch with --epoch (TT, ISO 8601)"
                    .to_string(),
            )
        })
    };
    let icrs: SkyCoord<Icrs> = match from {
        FrameName::Icrs => SkyCoord::from_degrees(lon, lat),
        FrameName::Galactic => SkyCoord::<Galactic>::from_degrees(lon, lat).transform_to::<Icrs>(),
        FrameName::Ecliptic => {
            SkyCoord::<MeanEcliptic>::from_degrees(lon, lat).transform_to::<Icrs>()
        }
        FrameName::MeanEclipticDate => {
            let (hi, lo) = at()?;
            mean_ecliptic_of_date_to_icrs(
                SkyCoord::<MeanEclipticOfDate>::from_degrees(lon, lat),
                hi,
                lo,
            )
        }
        FrameName::TrueEclipticDate => {
            let (hi, lo) = at()?;
            true_ecliptic_of_date_to_icrs(
                SkyCoord::<TrueEclipticOfDate>::from_degrees(lon, lat),
                hi,
                lo,
            )
        }
    };
    let (out_lon, out_lat) = match to {
        FrameName::Icrs => (icrs.lon_deg(), icrs.lat_deg()),
        FrameName::Galactic => {
            let c = icrs.transform_to::<Galactic>();
            (c.lon_deg(), c.lat_deg())
        }
        FrameName::Ecliptic => {
            let c = icrs.transform_to::<MeanEcliptic>();
            (c.lon_deg(), c.lat_deg())
        }
        FrameName::MeanEclipticDate => {
            let (hi, lo) = at()?;
            let c = icrs_to_mean_ecliptic_of_date(icrs, hi, lo);
            (c.lon_deg(), c.lat_deg())
        }
        FrameName::TrueEclipticDate => {
            let (hi, lo) = at()?;
            let c = icrs_to_true_ecliptic_of_date(icrs, hi, lo);
            (c.lon_deg(), c.lat_deg())
        }
    };
    Ok(match format {
        OutputFormat::Table => format!("{out_lon},{out_lat} {}", to.label()),
        OutputFormat::Json => json::object(&[
            ("frame", json::string(to.label())),
            ("lon_deg", out_lon.to_string()),
            ("lat_deg", out_lat.to_string()),
        ]),
    })
}

/// Names of scales and frames that are known but not implemented yet.
const UNIMPLEMENTED: &[&str] = &[
    "icrs", "fk5", "fk4", "galactic", "ecliptic", "gcrs", "itrs", "altaz",
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
    /// Barycentric Dynamical Time.
    Tdb,
    /// Geocentric Coordinate Time.
    Tcg,
    /// Barycentric Coordinate Time.
    Tcb,
    /// Universal Time, UT1 (needs an IERS EOP file for UT1 - UTC).
    Ut1,
}

impl Scale {
    fn label(self) -> &'static str {
        match self {
            Scale::Utc => "utc",
            Scale::Tai => "tai",
            Scale::Tt => "tt",
            Scale::Tdb => "tdb",
            Scale::Tcg => "tcg",
            Scale::Tcb => "tcb",
            Scale::Ut1 => "ut1",
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
        "tdb" => Ok(Scale::Tdb),
        "tcg" => Ok(Scale::Tcg),
        "tcb" => Ok(Scale::Tcb),
        "ut1" => Ok(Scale::Ut1),
        other if UNIMPLEMENTED.contains(&other) => Err(Failure::new(
            EXIT_NOT_IMPLEMENTED,
            format!(
                "scale or frame '{name}' is not implemented yet; astro convert supports \
                 the time scales utc, tai, tt, tdb and ut1 only"
            ),
        )),
        _ => Err(Failure::new(
            EXIT_USAGE,
            format!("unknown time scale '{name}': expected utc, tai, tt, tdb or ut1"),
        )),
    }
}

/// A timestamp on one of the supported scales.
#[derive(Debug, Clone, Copy)]
enum AnyEpoch {
    Utc(Epoch<Utc>),
    Tai(Epoch<Tai>),
    Tt(Epoch<Tt>),
    /// TDB, held as a Julian date (the core crate has no TDB scale type).
    Tdb(JulianDate),
    /// TCG, held as a Julian date.
    Tcg(JulianDate),
    /// TCB, held as a Julian date.
    Tcb(JulianDate),
}

impl AnyEpoch {
    fn jd(self) -> JulianDate {
        match self {
            AnyEpoch::Utc(e) => e.jd(),
            AnyEpoch::Tai(e) => e.jd(),
            AnyEpoch::Tt(e) => e.jd(),
            AnyEpoch::Tdb(j) | AnyEpoch::Tcg(j) | AnyEpoch::Tcb(j) => j,
        }
    }

    fn civil(self) -> Result<CivilTime, CoreError> {
        match self {
            AnyEpoch::Utc(e) => e.to_civil(&EMBEDDED),
            AnyEpoch::Tai(e) => e.to_civil(),
            AnyEpoch::Tt(e) => e.to_civil(),
            AnyEpoch::Tdb(j) | AnyEpoch::Tcg(j) | AnyEpoch::Tcb(j) => {
                Epoch::<Tt>::from_jd(j).to_civil()
            }
        }
    }
}

fn to_tai(e: AnyEpoch) -> Result<Epoch<Tai>, CoreError> {
    match e {
        AnyEpoch::Utc(u) => u.to_scale(&EMBEDDED),
        AnyEpoch::Tai(t) => Ok(t),
        AnyEpoch::Tt(t) => t.to_scale(&EMBEDDED),
        AnyEpoch::Tdb(_) | AnyEpoch::Tcg(_) | AnyEpoch::Tcb(_) => to_tt(e)?.to_scale(&EMBEDDED),
    }
}

fn to_tt(e: AnyEpoch) -> Result<Epoch<Tt>, CoreError> {
    match e {
        AnyEpoch::Utc(u) => u.to_scale(&EMBEDDED),
        AnyEpoch::Tai(t) => t.to_scale(&EMBEDDED),
        AnyEpoch::Tt(t) => Ok(t),
        AnyEpoch::Tdb(j) => {
            let (hi, lo) = tdb_to_tt(j.day(), j.fraction());
            Ok(Epoch::from_jd(JulianDate::new(hi, lo)))
        }
        AnyEpoch::Tcg(j) => {
            let (hi, lo) = tcg_to_tt(j.day(), j.fraction());
            Ok(Epoch::from_jd(JulianDate::new(hi, lo)))
        }
        AnyEpoch::Tcb(j) => {
            let (hi, lo) = tcb_to_tdb(j.day(), j.fraction());
            let (hi, lo) = tdb_to_tt(hi, lo);
            Ok(Epoch::from_jd(JulianDate::new(hi, lo)))
        }
    }
}

fn to_tcg(e: AnyEpoch) -> Result<JulianDate, CoreError> {
    let tt = to_tt(e)?.jd();
    let (hi, lo) = tt_to_tcg(tt.day(), tt.fraction());
    Ok(JulianDate::new(hi, lo))
}

fn to_tcb(e: AnyEpoch) -> Result<JulianDate, CoreError> {
    let tdb = to_tdb(e)?;
    let (hi, lo) = tdb_to_tcb(tdb.day(), tdb.fraction());
    Ok(JulianDate::new(hi, lo))
}

fn to_tdb(e: AnyEpoch) -> Result<JulianDate, CoreError> {
    match e {
        AnyEpoch::Tdb(j) => Ok(j),
        AnyEpoch::Tcb(j) => {
            let (hi, lo) = tcb_to_tdb(j.day(), j.fraction());
            Ok(JulianDate::new(hi, lo))
        }
        other => {
            let tt = to_tt(other)?.jd();
            let (hi, lo) = tt_to_tdb(tt.day(), tt.fraction());
            Ok(JulianDate::new(hi, lo))
        }
    }
}

fn to_utc(e: AnyEpoch) -> Result<Epoch<Utc>, CoreError> {
    match e {
        AnyEpoch::Utc(u) => Ok(u),
        AnyEpoch::Tai(t) => t.to_scale(&EMBEDDED),
        AnyEpoch::Tt(t) => t.to_scale(&EMBEDDED),
        AnyEpoch::Tdb(_) | AnyEpoch::Tcg(_) | AnyEpoch::Tcb(_) => to_tt(e)?.to_scale(&EMBEDDED),
    }
}

fn convert(e: AnyEpoch, to: Scale) -> Result<AnyEpoch, CoreError> {
    Ok(match to {
        Scale::Utc => AnyEpoch::Utc(to_utc(e)?),
        Scale::Tai => AnyEpoch::Tai(to_tai(e)?),
        Scale::Tt => AnyEpoch::Tt(to_tt(e)?),
        Scale::Tdb => AnyEpoch::Tdb(to_tdb(e)?),
        Scale::Tcg => AnyEpoch::Tcg(to_tcg(e)?),
        Scale::Tcb => AnyEpoch::Tcb(to_tcb(e)?),
        Scale::Ut1 => unreachable!("UT1 conversions are routed through convert_ut1"),
    })
}

fn core_failure(context: &str, e: CoreError) -> Failure {
    Failure::new(EXIT_RUNTIME, format!("{context}: {e}"))
}

/// Runs the conversion and returns the text to print (without a final newline).
#[cfg(test)]
pub fn convert_command(
    value: &str,
    from: &str,
    to: &str,
    format: OutputFormat,
) -> Result<String, Failure> {
    convert_command_at(value, from, to, None, None, format)
}

/// As `convert_command`, with an optional `epoch` (an ISO 8601 TT timestamp)
/// for the frames that depend on time.
pub fn convert_command_at(
    value: &str,
    from: &str,
    to: &str,
    epoch: Option<&str>,
    eop: Option<&Path>,
    format: OutputFormat,
) -> Result<String, Failure> {
    match (frame_name(from), frame_name(to)) {
        (Some(f), Some(t)) => {
            let at = if f.needs_epoch() || t.needs_epoch() {
                Some(require_epoch(epoch)?)
            } else {
                None
            };
            return frame_convert(value, f, t, at, format);
        }
        (Some(_), None) | (None, Some(_)) => {
            // Mixing a frame with a time scale: report the other side's error
            // if it is unknown or unimplemented, otherwise the mismatch.
            let other = if frame_name(from).is_some() { to } else { from };
            parse_scale(other)?;
            return Err(Failure::new(
                EXIT_USAGE,
                format!("cannot convert between the coordinate frame and the time scale '{other}'"),
            ));
        }
        (None, None) => {}
    }
    let from = parse_scale(from)?;
    let to = parse_scale(to)?;
    let ct = parse_timestamp(value).map_err(|m| Failure::new(EXIT_USAGE, m))?;
    if from == Scale::Ut1 || to == Scale::Ut1 {
        return convert_ut1(&ct, from, to, eop, format);
    }

    let input = match from {
        Scale::Utc => Epoch::<Utc>::from_civil(ct, &EMBEDDED).map(AnyEpoch::Utc),
        Scale::Tai => Epoch::<Tai>::from_civil(ct).map(AnyEpoch::Tai),
        Scale::Tt => Epoch::<Tt>::from_civil(ct).map(AnyEpoch::Tt),
        // The calendar reading of a TDB timestamp is the same Julian date.
        Scale::Tdb => Epoch::<Tt>::from_civil(ct).map(|t| AnyEpoch::Tdb(t.jd())),
        Scale::Tcg => Epoch::<Tt>::from_civil(ct).map(|t| AnyEpoch::Tcg(t.jd())),
        Scale::Tcb => Epoch::<Tt>::from_civil(ct).map(|t| AnyEpoch::Tcb(t.jd())),
        Scale::Ut1 => unreachable!("UT1 inputs are routed through convert_ut1"),
    }
    .map_err(|e| core_failure("invalid timestamp", e))?;

    let output = convert(input, to).map_err(|e| core_failure("cannot convert", e))?;
    let out_ct = output
        .civil()
        .map_err(|e| core_failure("cannot format result", e))?;
    Ok(render_time(&out_ct, output.jd(), to, format))
}

/// Formats a converted time: the calendar form and the two-part Julian date.
fn render_time(ct: &CivilTime, jd: JulianDate, to: Scale, format: OutputFormat) -> String {
    let timestamp = format_timestamp(ct);
    match format {
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
    }
}

/// Converts to or from UT1. UT1 is bridged through UTC: a UT1 input becomes
/// UTC with the offset UT1 - UTC from the IERS table, and a UT1 output is UTC
/// plus that offset. The table comes from an IERS finals2000A or C04 file given by
/// `--eop`; its rows must be daily around the epoch.
fn convert_ut1(
    ct: &CivilTime,
    from: Scale,
    to: Scale,
    eop: Option<&Path>,
    format: OutputFormat,
) -> Result<String, Failure> {
    let path = eop.ok_or_else(|| {
        Failure::new(
            EXIT_USAGE,
            "UT1 needs --eop FILE, an IERS finals2000A or C04 file, for UT1 - UTC".to_string(),
        )
    })?;
    let text = std::fs::read_to_string(path).map_err(|e| {
        Failure::new(
            EXIT_RUNTIME,
            format!("cannot read EOP file '{}': {e}", path.display()),
        )
    })?;
    let records = parse_eop_text(&text).map_err(|e| {
        Failure::new(
            EXIT_RUNTIME,
            format!("cannot parse EOP file '{}': {e}", path.display()),
        )
    })?;
    let table = EopTable::new(&records)
        .map_err(|e| Failure::new(EXIT_RUNTIME, format!("bad EOP table: {e}")))?;
    let eop_err = |e: EopError| Failure::new(EXIT_RUNTIME, format!("UT1 - UTC: {e}"));

    // The UTC epoch of the input.
    let utc: Epoch<Utc> = match from {
        Scale::Ut1 => {
            // The calendar reading of a UT1 timestamp. The offset is taken at the
            // UT1 epoch; the difference from the UTC epoch is far below a
            // millisecond, since UT1 - UTC changes by about a millisecond a day.
            let j = Epoch::<Tt>::from_civil(*ct)
                .map_err(|e| core_failure("invalid timestamp", e))?
                .jd();
            let dut1 = table.ut1_minus_utc(mjd_of(j)).map_err(eop_err)?;
            Epoch::<Utc>::from_jd(Epoch::<Tt>::from_jd(j).add_seconds(-dut1).jd())
        }
        Scale::Utc => Epoch::<Utc>::from_civil(*ct, &EMBEDDED)
            .map_err(|e| core_failure("invalid timestamp", e))?,
        other => {
            let input = match other {
                Scale::Tai => AnyEpoch::Tai(
                    Epoch::<Tai>::from_civil(*ct)
                        .map_err(|e| core_failure("invalid timestamp", e))?,
                ),
                Scale::Tt => AnyEpoch::Tt(
                    Epoch::<Tt>::from_civil(*ct)
                        .map_err(|e| core_failure("invalid timestamp", e))?,
                ),
                Scale::Tdb => AnyEpoch::Tdb(
                    Epoch::<Tt>::from_civil(*ct)
                        .map_err(|e| core_failure("invalid timestamp", e))?
                        .jd(),
                ),
                Scale::Tcg => AnyEpoch::Tcg(
                    Epoch::<Tt>::from_civil(*ct)
                        .map_err(|e| core_failure("invalid timestamp", e))?
                        .jd(),
                ),
                Scale::Tcb => AnyEpoch::Tcb(
                    Epoch::<Tt>::from_civil(*ct)
                        .map_err(|e| core_failure("invalid timestamp", e))?
                        .jd(),
                ),
                Scale::Utc | Scale::Ut1 => unreachable!("handled above"),
            };
            to_utc(input).map_err(|e| core_failure("cannot convert", e))?
        }
    };

    if to == Scale::Ut1 {
        let j = utc.jd();
        let (hi, lo) = table.utc_to_ut1(j.day(), j.fraction()).map_err(eop_err)?;
        let out = Epoch::<Tt>::from_jd(JulianDate::new(hi, lo));
        let out_ct = out
            .to_civil()
            .map_err(|e| core_failure("cannot format result", e))?;
        return Ok(render_time(&out_ct, out.jd(), to, format));
    }
    let output = convert(AnyEpoch::Utc(utc), to).map_err(|e| core_failure("cannot convert", e))?;
    let out_ct = output
        .civil()
        .map_err(|e| core_failure("cannot format result", e))?;
    Ok(render_time(&out_ct, output.jd(), to, format))
}

/// Modified Julian Date of a two-part Julian date.
fn mjd_of(j: JulianDate) -> f64 {
    (j.day() - 2_400_000.5) + j.fraction()
}

/// Parses an observer given as `LON,LAT,HEIGHT`: WGS84 longitude (east, degrees),
/// geodetic latitude (degrees) and height (metres).
fn parse_observer(text: &str) -> Result<(f64, f64, f64), Failure> {
    let usage = |msg: &str| Failure::new(EXIT_USAGE, format!("--observer: {msg}"));
    let parts: Vec<&str> = text.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        return Err(usage("expected LON,LAT,HEIGHT"));
    }
    let mut values = [0.0f64; 3];
    for (slot, part) in values.iter_mut().zip(&parts) {
        *slot = part
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| usage(&format!("'{part}' is not a number")))?;
    }
    if !(-90.0..=90.0).contains(&values[1]) {
        return Err(usage("latitude must be between -90 and 90 degrees"));
    }
    Ok((values[0], values[1], values[2]))
}

/// Alt/Az of an ICRS direction for a geodetic observer at a UTC epoch.
///
/// UT1 - UTC and the polar motion come from the EOP file given by `eop`
/// (`TransformContext::from_utc_and_eop`); no aberration or parallax is applied.
/// With a positive pressure the refraction model of ERFA is applied; zero pressure
/// gives the geometric position.
pub fn altaz_command(
    ra_deg: f64,
    dec_deg: f64,
    observer: &str,
    epoch: &str,
    eop: &Path,
    atmosphere: Atmosphere,
    format: OutputFormat,
) -> Result<String, Failure> {
    if !(ra_deg.is_finite() && dec_deg.is_finite() && (-90.0..=90.0).contains(&dec_deg)) {
        return Err(Failure::new(
            EXIT_USAGE,
            "the direction needs a finite RA and a Dec between -90 and 90 degrees".to_string(),
        ));
    }
    let (lon, lat, height) = parse_observer(observer)?;
    let ct = parse_timestamp(epoch)
        .map_err(|e| Failure::new(EXIT_USAGE, format!("invalid epoch '{epoch}': {e}")))?;
    let utc =
        Epoch::<Utc>::from_civil(ct, &EMBEDDED).map_err(|e| core_failure("invalid epoch", e))?;
    let text = std::fs::read_to_string(eop).map_err(|e| {
        Failure::new(
            EXIT_RUNTIME,
            format!("cannot read EOP file '{}': {e}", eop.display()),
        )
    })?;
    let records = parse_eop_text(&text).map_err(|e| {
        Failure::new(
            EXIT_RUNTIME,
            format!("cannot parse EOP file '{}': {e}", eop.display()),
        )
    })?;
    let table = EopTable::new(&records)
        .map_err(|e| Failure::new(EXIT_RUNTIME, format!("bad EOP table: {e}")))?;
    let ctx = TransformContext::from_utc_and_eop(utc, &table, &EMBEDDED)
        .map_err(|e| Failure::new(EXIT_RUNTIME, e.to_string()))?;

    let star = SkyCoord::<Icrs>::from_degrees(ra_deg, dec_deg);
    let site = GeodeticObserver::from_degrees(lon, lat, height);
    let altaz = icrs_to_altaz_refracted_at(star, site, &ctx, atmosphere);
    let (az, alt) = (altaz.lon_deg(), altaz.lat_deg());
    Ok(match format {
        OutputFormat::Table => format!("azimuth {az} deg, altitude {alt} deg"),
        OutputFormat::Json => json::object(&[
            ("azimuth_deg", az.to_string()),
            ("altitude_deg", alt.to_string()),
        ]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_frames_need_an_epoch_and_round_trip() {
        let no_epoch = convert_command("10,20", "icrs", "ecliptic-date", OutputFormat::Table);
        assert_eq!(no_epoch.unwrap_err().code, EXIT_USAGE);
        let at = Some("2024-06-01T12:00:00");
        let date = convert_command_at(
            "10,20",
            "icrs",
            "ecliptic-date",
            at,
            None,
            OutputFormat::Table,
        )
        .unwrap();
        let back = convert_command_at(
            date.split(' ').next().unwrap(),
            "ecliptic-date",
            "icrs",
            at,
            None,
            OutputFormat::Table,
        )
        .unwrap();
        let parse = |s: &str| -> (f64, f64) {
            let mut it = s.split(' ').next().unwrap().split(',');
            (
                it.next().unwrap().parse().unwrap(),
                it.next().unwrap().parse().unwrap(),
            )
        };
        let (l0, b0) = parse("10,20");
        let (l1, b1) = parse(&back);
        assert!((l1 - l0).abs() < 1e-9 && (b1 - b0).abs() < 1e-9, "{back}");
    }

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
    fn tt_to_tdb_and_back_is_the_identity() {
        // TDB - TT is about 1.7 ms at most, so the printed timestamp moves only
        // in its last digits; converting back must give the input again.
        let out = run("2017-01-01T00:01:09.184", "tt", "tdb");
        let first = out.lines().next().unwrap();
        assert!(first.ends_with(" TDB"), "{out}");
        let stamp = first.split(' ').next().unwrap();
        let back = run(stamp, "tdb", "tt");
        let back_first = back.lines().next().unwrap();
        assert!(
            back_first.starts_with("2017-01-01T00:01:09.18") && back_first.ends_with(" TT"),
            "{back}"
        );
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
        assert_eq!(parse_scale("tdb").unwrap(), Scale::Tdb);
        assert_eq!(parse_scale("tcb").unwrap(), Scale::Tcb);
        assert_eq!(parse_scale("fk5").unwrap_err().code, EXIT_NOT_IMPLEMENTED);
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
