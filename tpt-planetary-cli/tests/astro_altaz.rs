//! Integration tests for `tptplan astro altaz`, with an excerpt of the IERS
//! finals2000A file for UT1 - UTC and the polar motion.

use std::path::PathBuf;
use std::process::{Command, Output};

fn eop_file() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tpt-planetary-astro")
        .join("tests")
        .join("fixtures")
        .join("eop")
        .join("finals_excerpt.all")
}

fn tptplan(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tptplan"))
        .args(args)
        .output()
        .expect("failed to run tptplan")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("process exited with a code")
}

/// A numeric member of the JSON output.
fn number(out: &Output, key: &str) -> f64 {
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let pattern = format!("\"{key}\": ");
    let start = text.find(&pattern).expect("member") + pattern.len();
    let rest = &text[start..];
    let end = rest.find([',', '}']).expect("end of value");
    rest[..end].trim().parse().expect("number")
}

/// Runs `altaz` for the direction (10, -10) from Chile at 2017-09-05 12:00 UTC.
fn altaz(extra: &[&str]) -> Output {
    let eop = eop_file();
    let mut args = vec![
        "--format",
        "json",
        "astro",
        "altaz",
        "10",
        "-10",
        "--observer=-70.6,-33.5,2400",
        "--epoch",
        "2017-09-05T12:00:00",
        "--eop",
    ];
    let eop_text = eop.to_str().unwrap().to_string();
    args.push(&eop_text);
    args.extend_from_slice(extra);
    tptplan(&args)
}

#[test]
fn geometric_altaz_is_a_valid_direction() {
    let out = altaz(&[]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let az = number(&out, "azimuth_deg");
    let alt = number(&out, "altitude_deg");
    assert!((0.0..360.0).contains(&az), "azimuth {az}");
    assert!((-90.0..=90.0).contains(&alt), "altitude {alt}");
    // The direction is 10.4 degrees above the horizon for this epoch and site.
    assert!((alt - 10.446).abs() < 0.01, "altitude {alt}");
}

#[test]
fn refraction_raises_the_altitude_and_keeps_the_azimuth() {
    let geometric = altaz(&[]);
    let refracted = altaz(&["--pressure", "1013.25"]);
    assert_eq!(code(&refracted), 0);
    let az0 = number(&geometric, "azimuth_deg");
    let alt0 = number(&geometric, "altitude_deg");
    let az1 = number(&refracted, "azimuth_deg");
    let alt1 = number(&refracted, "altitude_deg");
    assert_eq!(az0, az1, "refraction changes only the altitude");
    // At 10 degrees the refraction is about 5.3 arcminutes (0.088 degrees).
    let shift = alt1 - alt0;
    assert!(shift > 0.0 && shift < 0.1, "shift {shift}");
}

#[test]
fn table_output_names_both_angles() {
    let eop = eop_file();
    let out = tptplan(&[
        "astro",
        "altaz",
        "10",
        "-10",
        "--observer=-70.6,-33.5,2400",
        "--epoch",
        "2017-09-05T12:00:00",
        "--eop",
        eop.to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("azimuth "), "{text}");
    assert!(text.contains("altitude "), "{text}");
}

#[test]
fn a_latitude_out_of_range_is_a_usage_error() {
    let eop = eop_file();
    let out = tptplan(&[
        "astro",
        "altaz",
        "10",
        "-10",
        "--observer=0,95,0",
        "--epoch",
        "2017-09-05T12:00:00",
        "--eop",
        eop.to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 2);
}

#[test]
fn a_missing_eop_argument_is_a_usage_error() {
    let out = tptplan(&[
        "astro",
        "altaz",
        "10",
        "-10",
        "--observer=0,0,0",
        "--epoch",
        "2017-09-05T12:00:00",
    ]);
    assert_eq!(code(&out), 2);
}

#[test]
fn an_epoch_in_a_gap_of_the_eop_table_is_a_runtime_error() {
    let eop = eop_file();
    // 2017-10-12 lies between two excerpt windows.
    let out = tptplan(&[
        "astro",
        "altaz",
        "10",
        "-10",
        "--observer=-70.6,-33.5,2400",
        "--epoch",
        "2017-10-12T12:00:00",
        "--eop",
        eop.to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 1);
}
