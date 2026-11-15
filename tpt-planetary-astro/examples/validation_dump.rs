//! Prints quantities that the Python bindings do not expose, so that the
//! validation tests in `bindings/tpt-planetary-py/tests/validation` can compare
//! them with ERFA and Astropy. Not part of the public API.
//!
//! Usage (stdin is one whitespace-separated record per line):
//!
//! - `tdb`: `tt_hi tt_lo` -> TDB - TT in seconds.
//! - `obl`: `tt_hi tt_lo` -> IAU 2006 mean obliquity in arcseconds.
//! - `ecl`: `ra_deg dec_deg` -> J2000 mean ecliptic `lon_deg lat_deg`.

use std::io::{self, BufRead, Write};

use tpt_planetary_astro::frames::{Icrs, MeanEcliptic, SkyCoord};
use tpt_planetary_astro::time;

fn parse_fields(line: &str, n: usize) -> Option<Vec<f64>> {
    let vals: Result<Vec<f64>, _> = line.split_whitespace().map(str::parse).collect();
    match vals {
        Ok(v) if v.len() == n => Some(v),
        _ => None,
    }
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let line = line.expect("stdin readable");
        if line.trim().is_empty() {
            continue;
        }
        let result = match mode.as_str() {
            "tdb" => {
                parse_fields(&line, 2).map(|v| time::tdb_minus_tt_seconds(v[0], v[1]).to_string())
            }
            "obl" => parse_fields(&line, 2)
                .map(|v| time::mean_obliquity_iau2006_arcsec(v[0], v[1]).to_string()),
            "ecl" => parse_fields(&line, 2).map(|v| {
                let c = SkyCoord::<Icrs>::from_degrees(v[0], v[1]).transform_to::<MeanEcliptic>();
                format!("{} {}", c.lon_deg(), c.lat_deg())
            }),
            _ => None,
        };
        match result {
            Some(s) => writeln!(out, "{s}").expect("stdout writable"),
            None => {
                eprintln!("validation_dump: bad mode or input line: {line:?}");
                std::process::exit(2);
            }
        }
    }
}
