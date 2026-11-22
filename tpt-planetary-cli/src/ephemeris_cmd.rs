//! `tptplan ephemeris query`: body position from a DAF/SPK kernel.
//!
//! Bodies are given as NAIF IDs or as one of the standard names below. The
//! astro crate has no name table, so no other names are resolved.

use std::path::Path;

use tpt_planetary_astro::ephemeris::{
    Astrometric, SpkFile, StateVector, apparent_direction, astrometric, et_from_tdb_jd, naif,
    read_file,
};
use tpt_planetary_core::calendar::CalendarSystem;
use tpt_planetary_core::jd::JulianDate;
use tpt_planetary_core::scale::CivilTime;

use crate::iso::{format_timestamp, parse_timestamp};
use crate::json;
use crate::{EXIT_NOT_IMPLEMENTED, EXIT_RUNTIME, EXIT_USAGE, Failure, OutputFormat};

/// Resolves a body argument to a NAIF ID.
///
/// Accepts any integer NAIF ID, or one of `sun` (10), `earth` (399), `moon`
/// (301), `mars` (barycentre 4), `jupiter` (barycentre 5) and `saturn`
/// (barycentre 6).
pub fn resolve_body(arg: &str) -> Result<i32, Failure> {
    if let Ok(id) = arg.parse::<i32>() {
        return Ok(id);
    }
    match arg.to_ascii_lowercase().as_str() {
        "sun" => Ok(naif::SUN),
        "earth" => Ok(naif::EARTH),
        "moon" => Ok(naif::MOON),
        "mars" => Ok(naif::MARS_BARYCENTRE),
        "jupiter" => Ok(naif::JUPITER_BARYCENTRE),
        "saturn" => Ok(naif::SATURN_BARYCENTRE),
        _ => Err(Failure::new(
            EXIT_USAGE,
            format!(
                "unknown body '{arg}': use a NAIF ID or one of sun, earth, moon, mars, \
                 jupiter, saturn"
            ),
        )),
    }
}

/// Short display name for a NAIF ID that this command resolves by name.
fn known_name(id: i32) -> Option<&'static str> {
    match id {
        10 => Some("sun"),
        399 => Some("earth"),
        301 => Some("moon"),
        4 => Some("mars"),
        5 => Some("jupiter"),
        6 => Some("saturn"),
        _ => None,
    }
}

/// Arguments of `ephemeris query`, as parsed by clap.
#[derive(Debug)]
pub struct QueryArgs<'a> {
    /// Target body argument.
    pub body: &'a str,
    /// Epoch, ISO 8601, interpreted as TDB.
    pub epoch: &'a str,
    /// Observer body argument.
    pub observer: &'a str,
    /// Output frame; only `icrs` is supported.
    pub frame: &'a str,
    /// Path to a DAF/SPK file.
    pub kernel: Option<&'a Path>,
    /// Print the astrometric RA and Dec instead of the geometric state.
    pub astrometric: bool,
    /// Print the apparent RA and Dec (light deflection and aberration applied).
    pub apparent: bool,
}

/// Runs the query and returns the text to print (without a final newline).
pub fn query(args: &QueryArgs<'_>, format: OutputFormat) -> Result<String, Failure> {
    let target = resolve_body(args.body)?;
    let observer = resolve_body(args.observer)?;
    let ct = parse_timestamp(args.epoch).map_err(|m| Failure::new(EXIT_USAGE, m))?;
    if !args.frame.eq_ignore_ascii_case("icrs") {
        return Err(Failure::new(
            EXIT_NOT_IMPLEMENTED,
            format!(
                "frame '{}' is not implemented yet; ephemeris query supports icrs only",
                args.frame
            ),
        ));
    }
    let Some(kernel) = args.kernel else {
        return Err(Failure::new(
            EXIT_NOT_IMPLEMENTED,
            "ephemeris query needs --kernel <path> to a DAF/SPK file; no kernel catalogue or \
             download is available in this build"
                .to_string(),
        ));
    };

    let secs = f64::from(ct.hour) * 3600.0 + f64::from(ct.minute) * 60.0 + ct.second;
    let jd = JulianDate::from_calendar(ct.date, CalendarSystem::Gregorian, secs)
        .map_err(|e| Failure::new(EXIT_RUNTIME, format!("invalid epoch: {e}")))?;

    let bytes = read_file(kernel).map_err(|e| {
        Failure::new(
            EXIT_RUNTIME,
            format!("cannot read kernel '{}': {e}", kernel.display()),
        )
    })?;
    let spk = SpkFile::new(&bytes).map_err(|e| {
        Failure::new(
            EXIT_RUNTIME,
            format!("cannot open kernel '{}': {e}", kernel.display()),
        )
    })?;
    let et = et_from_tdb_jd(jd.day(), jd.fraction());
    if args.apparent {
        let (direction, light_time_s) =
            apparent_direction(&spk, target, observer, et).map_err(|e| {
                Failure::new(
                    EXIT_RUNTIME,
                    format!(
                        "cannot compute the apparent direction of {target} from {observer}: {e}"
                    ),
                )
            })?;
        let ra = direction.lon();
        let dec = direction.lat();
        return Ok(render_apparent(
            target,
            observer,
            &ct,
            ra,
            dec,
            light_time_s,
            format,
        ));
    }
    if args.astrometric {
        let a = astrometric(&spk, target, observer, et).map_err(|e| {
            Failure::new(
                EXIT_RUNTIME,
                format!(
                    "cannot compute the astrometric direction of {target} from {observer}: {e}"
                ),
            )
        })?;
        return Ok(render_astrometric(target, observer, &ct, &a, format));
    }
    let state = spk.state(target, observer, et).map_err(|e| {
        Failure::new(
            EXIT_RUNTIME,
            format!("cannot evaluate body {target} relative to {observer}: {e}"),
        )
    })?;

    Ok(render(target, observer, &ct, &state, format))
}

fn render_apparent(
    target: i32,
    observer: i32,
    ct: &CivilTime,
    ra: f64,
    dec: f64,
    light_time_s: f64,
    format: OutputFormat,
) -> String {
    let epoch = format_timestamp(ct);
    let ra_deg = ra.to_degrees();
    let dec_deg = dec.to_degrees();
    match format {
        OutputFormat::Table => {
            let label = |id: i32| known_name(id).map_or(String::new(), |n| format!(" ({n})"));
            format!(
                "target: {target}{}\nobserver: {observer}{}\nframe: icrs (apparent)\n\
                 epoch: {epoch} TDB\nright ascension deg: {ra_deg}\ndeclination deg: {dec_deg}\n\
                 light time s: {light_time_s}",
                label(target),
                label(observer),
            )
        }
        OutputFormat::Json => json::object(&[
            ("target_naif_id", target.to_string()),
            ("observer_naif_id", observer.to_string()),
            ("frame", json::string("icrs")),
            ("correction", json::string("apparent")),
            ("epoch_tdb", json::string(&epoch)),
            ("ra_deg", ra_deg.to_string()),
            ("dec_deg", dec_deg.to_string()),
            ("light_time_s", light_time_s.to_string()),
        ]),
    }
}

fn render_astrometric(
    target: i32,
    observer: i32,
    ct: &CivilTime,
    a: &Astrometric,
    format: OutputFormat,
) -> String {
    let epoch = format_timestamp(ct);
    let ra_deg = a.ra_rad.to_degrees();
    let dec_deg = a.dec_rad.to_degrees();
    match format {
        OutputFormat::Table => {
            let label = |id: i32| known_name(id).map_or(String::new(), |n| format!(" ({n})"));
            format!(
                "target: {target}{}\nobserver: {observer}{}\nframe: icrs (astrometric)\n\
                 epoch: {epoch} TDB\nright ascension deg: {ra_deg}\ndeclination deg: {dec_deg}\n\
                 distance km: {}\nlight time s: {}",
                label(target),
                label(observer),
                a.distance_km,
                a.light_time_s
            )
        }
        OutputFormat::Json => json::object(&[
            ("target_naif_id", target.to_string()),
            ("observer_naif_id", observer.to_string()),
            ("frame", json::string("icrs")),
            ("correction", json::string("astrometric")),
            ("epoch_tdb", json::string(&epoch)),
            ("ra_deg", ra_deg.to_string()),
            ("dec_deg", dec_deg.to_string()),
            ("distance_km", a.distance_km.to_string()),
            ("light_time_s", a.light_time_s.to_string()),
        ]),
    }
}

fn render(
    target: i32,
    observer: i32,
    ct: &CivilTime,
    state: &StateVector,
    format: OutputFormat,
) -> String {
    let epoch = format_timestamp(ct);
    let [x, y, z] = state.position;
    let [vx, vy, vz] = state.velocity;
    match format {
        OutputFormat::Table => {
            let label = |id: i32| known_name(id).map_or(String::new(), |n| format!(" ({n})"));
            format!(
                "target: {target}{}\nobserver: {observer}{}\nframe: icrs\nepoch: {epoch} TDB\n\
                 position km: {x} {y} {z}\nvelocity km/s: {vx} {vy} {vz}",
                label(target),
                label(observer)
            )
        }
        OutputFormat::Json => json::object(&[
            ("target_naif_id", target.to_string()),
            ("observer_naif_id", observer.to_string()),
            ("frame", json::string("icrs")),
            ("epoch_tdb", json::string(&epoch)),
            ("position_km", json::f64_array(&state.position)),
            ("velocity_km_s", json::f64_array(&state.velocity)),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args<'a>(body: &'a str, kernel: Option<&'a Path>) -> QueryArgs<'a> {
        QueryArgs {
            body,
            epoch: "2017-01-01T00:00:00",
            observer: "earth",
            frame: "icrs",
            kernel,
            astrometric: false,
            apparent: false,
        }
    }

    #[test]
    fn resolves_standard_names_and_ids() {
        assert_eq!(resolve_body("sun").unwrap(), 10);
        assert_eq!(resolve_body("EARTH").unwrap(), 399);
        assert_eq!(resolve_body("moon").unwrap(), 301);
        assert_eq!(resolve_body("mars").unwrap(), 4);
        assert_eq!(resolve_body("jupiter").unwrap(), 5);
        assert_eq!(resolve_body("saturn").unwrap(), 6);
        assert_eq!(resolve_body("1000012").unwrap(), 1_000_012);
        assert_eq!(resolve_body("pluto").unwrap_err().code, EXIT_USAGE);
    }

    #[test]
    fn missing_kernel_is_not_implemented() {
        let e = query(&args("jupiter", None), OutputFormat::Table).unwrap_err();
        assert_eq!(e.code, EXIT_NOT_IMPLEMENTED);
        assert!(e.message.contains("--kernel"));
    }

    #[test]
    fn non_icrs_frame_is_not_implemented() {
        let mut a = args("jupiter", None);
        a.frame = "galactic";
        assert_eq!(
            query(&a, OutputFormat::Table).unwrap_err().code,
            EXIT_NOT_IMPLEMENTED
        );
    }

    #[test]
    fn unreadable_kernel_is_runtime_error() {
        let p = Path::new("definitely-missing-kernel.bsp");
        let e = query(&args("jupiter", Some(p)), OutputFormat::Table).unwrap_err();
        assert_eq!(e.code, EXIT_RUNTIME);
    }

    #[test]
    fn non_spk_file_is_runtime_error() {
        let dir = std::env::temp_dir().join(format!("tptplan-ephem-unit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("not-a-kernel.bsp");
        std::fs::write(&p, vec![0u8; 2048]).unwrap();
        let result = query(&args("jupiter", Some(&p)), OutputFormat::Table);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(result.unwrap_err().code, EXIT_RUNTIME);
    }
}
