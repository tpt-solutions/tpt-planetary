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
//! - `xy`: `tt_hi tt_lo` -> CIP `X Y` in radians (IAU 2006/2000A, GCRS).
//! - `s`: `tt_hi tt_lo` -> CIO locator `s` in radians.
//! - `sp`: `tt_hi tt_lo` -> TIO locator `s'` in radians.
//! - `c2i`: `tt_hi tt_lo` -> the nine elements of the ICRS to CIRS matrix
//!   (row-major).
//! - `bias`: any one field -> the nine elements of the frame bias matrix.
//! - `eect`: `tt_hi tt_lo` -> EO complementary terms in radians (Table 5.2e).
//! - `era`: `ut1_hi ut1_lo` -> Earth rotation angle in radians.
//! - `c2t`: `tt_hi tt_lo ut1_hi ut1_lo xp yp` -> the nine elements of the ICRS
//!   to ITRS matrix, with `xp`, `yp` in radians (row-major).
//! - `altaz`: `ra_deg dec_deg lon_deg lat_deg tt_hi tt_lo ut1_hi ut1_lo xp_arcsec yp_arcsec`
//!   -> `az_deg alt_deg` for an ICRS direction seen from a WGS84 geodetic observer
//!   (no refraction, aberration or parallax; see `frames::icrs_to_altaz_at`).
//! - `altaz_refracted`: the fields of `altaz` followed by `pressure_hPa temp_C
//!   humidity wavelength_um` -> `az_deg alt_deg`, with the refraction of
//!   `frames::icrs_to_altaz_refracted_at` (still no aberration or parallax).
//! - `c2t_xys`: `x y s era sp xp yp` (radians) -> the nine elements of the
//!   chain matrix built from those components (row-major). Used with ERFA's
//!   own `xys06a`, `era00` and `sp00` to isolate the matrix algebra.

use std::io::{self, BufRead, Write};

use tpt_planetary_astro::cio;
use tpt_planetary_astro::eop::ARCSEC_TO_RAD;
use tpt_planetary_astro::ephemeris::earth_sun_distance_au_approx;
use tpt_planetary_astro::frames::{
    Atmosphere, GeodeticObserver, Icrs, MeanEcliptic, SkyCoord, TransformContext, icrs_to_altaz_at,
    icrs_to_altaz_refracted_at, polar_motion_matrix,
};
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
        let result =
            match mode.as_str() {
                "tdb" => parse_fields(&line, 2)
                    .map(|v| time::tdb_minus_tt_seconds(v[0], v[1]).to_string()),
                "obl" => parse_fields(&line, 2)
                    .map(|v| time::mean_obliquity_iau2006_arcsec(v[0], v[1]).to_string()),
                "ecl" => parse_fields(&line, 2).map(|v| {
                    let c =
                        SkyCoord::<Icrs>::from_degrees(v[0], v[1]).transform_to::<MeanEcliptic>();
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
                "esd" => parse_fields(&line, 2)
                    .map(|v| earth_sun_distance_au_approx(v[0], v[1]).to_string()),
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
                "xy" => parse_fields(&line, 2).map(|v| {
                    let (x, y) = cio::cip_xy(v[0], v[1]);
                    format!("{x} {y}")
                }),
                "s" => parse_fields(&line, 2).map(|v| cio::cio_locator(v[0], v[1]).to_string()),
                "sp" => parse_fields(&line, 2).map(|v| cio::tio_locator(v[0], v[1]).to_string()),
                "c2i" => parse_fields(&line, 2).map(|v| {
                    cio::icrs_to_cirs(v[0], v[1])
                        .iter()
                        .flatten()
                        .map(f64::to_string)
                        .collect::<Vec<_>>()
                        .join(" ")
                }),
                "bias" => parse_fields(&line, 1).map(|_| {
                    cio::frame_bias_matrix()
                        .iter()
                        .flatten()
                        .map(f64::to_string)
                        .collect::<Vec<_>>()
                        .join(" ")
                }),
                "eect" => parse_fields(&line, 2)
                    .map(|v| cio::eo_complementary_terms(v[0], v[1]).to_string()),
                "era" => parse_fields(&line, 2)
                    .map(|v| time::earth_rotation_angle(v[0], v[1]).to_string()),
                "c2t" => parse_fields(&line, 6).map(|v| {
                    cio::icrs_to_itrs(v[0], v[1], v[2], v[3], v[4], v[5])
                        .iter()
                        .flatten()
                        .map(f64::to_string)
                        .collect::<Vec<_>>()
                        .join(" ")
                }),
                "c2t_xys" => parse_fields(&line, 7).map(|v| {
                    cio::itrs_from_components(v[0], v[1], v[2], v[3], v[4], v[5], v[6])
                        .iter()
                        .flatten()
                        .map(f64::to_string)
                        .collect::<Vec<_>>()
                        .join(" ")
                }),
                "altaz" => parse_fields(&line, 10).map(|v| {
                    let star = SkyCoord::<Icrs>::from_degrees(v[0], v[1]);
                    let obs = GeodeticObserver::from_degrees(v[2], v[3], 0.0);
                    let ctx = TransformContext::new((v[4], v[5]), (v[6], v[7]), v[8], v[9]);
                    let a = icrs_to_altaz_at(star, obs, &ctx);
                    format!("{} {}", a.lon_deg(), a.lat_deg())
                }),
                "altaz_refracted" => parse_fields(&line, 14).map(|v| {
                    let star = SkyCoord::<Icrs>::from_degrees(v[0], v[1]);
                    let obs = GeodeticObserver::from_degrees(v[2], v[3], 0.0);
                    let ctx = TransformContext::new((v[4], v[5]), (v[6], v[7]), v[8], v[9]);
                    let atmosphere = Atmosphere {
                        pressure_hpa: v[10],
                        temperature_c: v[11],
                        relative_humidity: v[12],
                        wavelength_um: v[13],
                    };
                    let a = icrs_to_altaz_refracted_at(star, obs, &ctx, atmosphere);
                    format!("{} {}", a.lon_deg(), a.lat_deg())
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
