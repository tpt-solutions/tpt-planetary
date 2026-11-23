//! Integration tests for `tptplan astro convert` with the coordinate time scales
//! TCG and TCB. The values are checked against pyerfa in
//! bindings/tpt-planetary-py/tests/validation/test_tcg_tcb.py; these tests check
//! the command-line wiring and the round trips.

use std::process::{Command, Output};

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

fn convert(value: &str, from: &str, to: &str) -> Output {
    tptplan(&[
        "--format", "json", "astro", "convert", value, "--from", from, "--to", to,
    ])
}

#[test]
fn tcg_minus_tt_at_j2000_is_the_linear_term() {
    // TCG - TT = L_G (JD - 2443144.5003725) days, 0.5058 s at JD 2451545.
    let out = convert("2000-01-01T12:00:00", "tt", "tcg");
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let offset = seconds_of_day(&timestamp(&out)) - 12.0 * 3600.0;
    assert!((offset - 0.5058333).abs() < 1e-6, "offset {offset}");
}

#[test]
fn tcg_round_trips_to_tt() {
    let forward = convert("2017-09-05T12:00:00", "tt", "tcg");
    assert_eq!(code(&forward), 0);
    let tcg = timestamp(&forward);
    let back = convert(&tcg, "tcg", "tt");
    assert_eq!(code(&back), 0, "{}", String::from_utf8_lossy(&back.stderr));
    let t = seconds_of_day(&timestamp(&back));
    assert!((t - 12.0 * 3600.0).abs() < 1e-5, "round trip gave {t}");
}

#[test]
fn tcb_round_trips_to_tdb() {
    let forward = convert("2000-01-01T12:00:00", "tdb", "tcb");
    assert_eq!(code(&forward), 0);
    // TCB - TDB at J2000 is 11.2537 s (the L_B term plus TDB0).
    let offset = seconds_of_day(&timestamp(&forward)) - 12.0 * 3600.0;
    assert!((offset - 11.25373).abs() < 1e-4, "offset {offset}");
    let back = convert(&timestamp(&forward), "tcb", "tdb");
    assert_eq!(code(&back), 0);
    let t = seconds_of_day(&timestamp(&back));
    assert!((t - 12.0 * 3600.0).abs() < 1e-5, "round trip gave {t}");
}

#[test]
fn utc_converts_to_tcg_through_tt() {
    // erfa.utctai, taitt and tttcg from 2017-09-05 12:00 UTC give TCG - UTC =
    // 70.0786394377662 s (TT - UTC is 69.184 s, TCG - TT is 0.8946394 s).
    let out = convert("2017-09-05T12:00:00", "utc", "tcg");
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let offset = seconds_of_day(&timestamp(&out)) - 12.0 * 3600.0;
    assert!((offset - 70.0786394377662).abs() < 1e-6, "offset {offset}");
}

#[test]
fn tcg_and_tcb_are_accepted_scales_not_unimplemented() {
    for name in ["tcg", "tcb", "TCG", "TCB"] {
        let out = convert("2000-01-01T12:00:00", name, "tt");
        assert_eq!(
            code(&out),
            0,
            "scale {name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
