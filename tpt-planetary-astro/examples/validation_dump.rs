//! Prints quantities that the Python bindings do not expose, so that the
//! validation tests in `bindings/tpt-planetary-py/tests/validation` can compare
//! them with ERFA and Astropy. Not part of the public API.
//!
//! Usage (stdin is one whitespace-separated record per line):
//!
//! - `tdb`: `tt_hi tt_lo` -> TDB - TT in seconds.
//! - `obl`: `tt_hi tt_lo` -> IAU 2006 mean obliquity in arcseconds.
//! - `ecl`: `ra_deg dec_deg` -> J2000 mean ecliptic `lon_deg lat_deg`.
//! - `pm`: `xp_arcsec yp_arcsec` -> the nine elements of the polar-motion
//!   matrix `W` (ITRS -> TIRS, row-major), with s' = 0.
//! - `esd`: `tt_hi tt_lo` -> low-precision Earth-Sun distance in AU.

use std::io::{self, BufRead, Write};

use tpt_planetary_astro::eop::ARCSEC_TO_RAD;
use tpt_planetary_astro::ephemeris::earth_sun_distance_au_approx;
use tpt_planetary_astro::frames::{Icrs, MeanEcliptic, SkyCoord, polar_motion_matrix};
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
            "pm" => parse_fields(&line, 2).map(|v| {
                let w = polar_motion_matrix(v[0] * ARCSEC_TO_RAD, v[1] * ARCSEC_TO_RAD);
                w.iter()
                    .flatten()
                    .map(f64::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            }),
            "esd" => {
                parse_fields(&line, 2).map(|v| earth_sun_distance_au_approx(v[0], v[1]).to_string())
            }
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
