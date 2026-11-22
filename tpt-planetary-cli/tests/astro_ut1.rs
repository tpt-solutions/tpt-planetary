//! Integration tests for `tptplan astro convert` with UT1, using an excerpt of
//! the IERS finals2000A file (the array crate's fixtures are not shared, so the
//! excerpt is read from the astro crate's test fixtures).

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

/// The `timestamp` member of the JSON output.
fn timestamp(out: &Output) -> String {
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let key = "\"timestamp\": \"";
    let start = text.find(key).expect("timestamp") + key.len();
    let end = start + text[start..].find('"').expect("closing quote");
    text[start..end].to_string()
}

/// Seconds of the day of a `YYYY-MM-DDTHH:MM:SS.fraction` timestamp.
fn seconds_of_day(ts: &str) -> f64 {
    let time = ts.split('T').nth(1).expect("time part");
    let h: f64 = time[0..2].parse().unwrap();
    let m: f64 = time[3..5].parse().unwrap();
    let s: f64 = time[6..].parse().unwrap();
    h * 3600.0 + m * 60.0 + s
}

#[test]
fn utc_to_ut1_adds_the_table_offset() {
    let eop = eop_file();
    let out = tptplan(&[
        "--format",
        "json",
        "astro",
        "convert",
        "2017-09-05T12:00:00",
        "--from",
        "utc",
        "--to",
        "ut1",
        "--eop",
        eop.to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    // astropy's IERS-B offset at this epoch is 0.335089 s; the finals rows give
    // 0.335111 s. Both come from the same series, within 50 microseconds.
    let offset = seconds_of_day(&timestamp(&out)) - 12.0 * 3600.0;
    assert!((offset - 0.335089).abs() < 50e-6, "offset {offset}");
}

#[test]
fn ut1_back_to_utc_recovers_the_input() {
    let eop = eop_file();
    let forward = tptplan(&[
        "--format",
        "json",
        "astro",
        "convert",
        "2017-09-05T12:00:00",
        "--from",
        "utc",
        "--to",
        "ut1",
        "--eop",
        eop.to_str().unwrap(),
    ]);
    let ut1 = timestamp(&forward);
    let back = tptplan(&[
        "--format",
        "json",
        "astro",
        "convert",
        &ut1,
        "--from",
        "ut1",
        "--to",
        "utc",
        "--eop",
        eop.to_str().unwrap(),
    ]);
    assert_eq!(code(&back), 0, "{}", String::from_utf8_lossy(&back.stderr));
    let t = seconds_of_day(&timestamp(&back));
    assert!((t - 12.0 * 3600.0).abs() < 1e-5, "round trip gave {t}");
}

#[test]
fn ut1_without_an_eop_file_is_a_usage_error() {
    let out = tptplan(&[
        "astro",
        "convert",
        "2017-09-05T12:00:00",
        "--from",
        "utc",
        "--to",
        "ut1",
    ]);
    assert_eq!(code(&out), 2);
}

#[test]
fn an_epoch_in_a_gap_of_the_table_is_a_runtime_error() {
    let eop = eop_file();
    // 2017-10-12 lies between two excerpt windows.
    let out = tptplan(&[
        "astro",
        "convert",
        "2017-10-12T12:00:00",
        "--from",
        "utc",
        "--to",
        "ut1",
        "--eop",
        eop.to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 1);
}

#[test]
fn a_missing_eop_file_is_a_runtime_error() {
    let out = tptplan(&[
        "astro",
        "convert",
        "2017-09-05T12:00:00",
        "--from",
        "utc",
        "--to",
        "ut1",
        "--eop",
        "no-such-finals.all",
    ]);
    assert_eq!(code(&out), 1);
}
