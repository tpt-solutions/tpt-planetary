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
//! - `prec`: `t` (Julian centuries of TT from J2000) -> `zeta_A z_A theta_A`
//!   in arcseconds, then the nine elements of the IAU 2006 precession matrix
//!   `P(t)` (row-major, precession only).
//! - `fa`: `t` -> the 14 fundamental arguments F1..F14 in radians.
//! - `nut`: `tt_hi tt_lo` -> `dpsi deps` in radians (IAU 2000A_R06).
//! - `ee`: `tt_hi tt_lo` -> equation of the equinoxes `dpsi cos(eps0)` in radians.
//! - `numat`: `tt_hi tt_lo` -> the nine elements of the nutation matrix `N`
//!   (row-major).
//! - `numat_from`: `dpsi deps eps0` (radians) -> the nine elements of `N` built
//!   from those angles (row-major). Used to check the matrix convention.
//! - `nut_sofa`: `t` -> `dpsi deps` in radians without the out-of-phase rate
//!   terms (the MHB2000/SOFA variant; see the crate documentation).
//! - `gast`: `ut1_hi ut1_lo tt_hi tt_lo` -> GAST in radians, IAU 2006 GMST
//!   plus the classical equation of the equinoxes (no complementary terms).
//! - `tod`: `ra_deg dec_deg tt_hi tt_lo` -> ICRS to true equator of date
//!   `lon_deg lat_deg` (precession, nutation; no frame bias).

use std::io::{self, BufRead, Write};

use tpt_planetary_astro::eop::ARCSEC_TO_RAD;
use tpt_planetary_astro::ephemeris::earth_sun_distance_au_approx;
use tpt_planetary_astro::frames::{Icrs, MeanEcliptic, SkyCoord, polar_motion_matrix};
use tpt_planetary_astro::nutation;
use tpt_planetary_astro::precession;
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
            "prec" => parse_fields(&line, 1).map(|v| {
                let t = v[0];
                let (zeta, z, theta) = precession::precession_angles_arcsec(t);
                let p = precession::precession_matrix(t);
                let mut fields = vec![zeta.to_string(), z.to_string(), theta.to_string()];
                fields.extend(p.iter().flatten().map(f64::to_string));
                fields.join(" ")
            }),
            "fa" => parse_fields(&line, 1).map(|v| {
                nutation::fundamental_arguments(v[0])
                    .iter()
                    .map(f64::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            }),
            "nut" => parse_fields(&line, 2).map(|v| {
                let (dpsi, deps) = nutation::nutation_angles(v[0], v[1]);
                format!("{dpsi} {deps}")
            }),
            "ee" => parse_fields(&line, 2)
                .map(|v| nutation::equation_of_equinoxes(v[0], v[1]).to_string()),
            "numat" => parse_fields(&line, 2).map(|v| {
                nutation::nutation_matrix(v[0], v[1])
                    .iter()
                    .flatten()
                    .map(f64::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            }),
            "numat_from" => parse_fields(&line, 3).map(|v| {
                nutation::nutation_matrix_from_angles(v[0], v[1], v[2])
                    .iter()
                    .flatten()
                    .map(f64::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            }),
            "nut_sofa" => parse_fields(&line, 1).map(|v| {
                let (dpsi, deps) = nutation::nutation_angles_t_without_out_of_phase_rates(v[0]);
                format!("{dpsi} {deps}")
            }),
            "gast" => parse_fields(&line, 4)
                .map(|v| time::gast_iau2006(v[0], v[1], v[2], v[3]).to_string()),
            "tod" => parse_fields(&line, 4).map(|v| {
                let star = SkyCoord::<Icrs>::from_degrees(v[0], v[1]);
                let c = nutation::icrs_to_true_equator_of_date(star, v[2], v[3]);
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
