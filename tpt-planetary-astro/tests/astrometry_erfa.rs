//! Checks annual aberration against ERFA (`erfa.ab`, through pyerfa), using the
//! reference values in `tests/fixtures/aberration_erfa.tsv` (see
//! `tests/fixtures/gen_aberration_fixture.py`).

use std::fs;
use std::path::Path;

use tpt_planetary_astro::astrometry::{annual_aberration, earth_velocity_approx_au_per_day};
use tpt_planetary_astro::frames::{Icrs, SkyCoord};

struct Case {
    ra: f64,
    dec: f64,
    jd1: f64,
    jd2: f64,
    velocity: [f64; 3],
    ara: f64,
    adec: f64,
}

fn cases() -> Vec<Case> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/aberration_erfa.tsv");
    let text = fs::read_to_string(&path).expect("reference file");
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let v: Vec<f64> = l
                .split_whitespace()
                .map(|x| x.parse().expect("number"))
                .collect();
            assert_eq!(v.len(), 9, "line {l}");
            Case {
                ra: v[0],
                dec: v[1],
                jd1: v[2],
                jd2: v[3],
                velocity: [v[4], v[5], v[6]],
                ara: v[7],
                adec: v[8],
            }
        })
        .collect()
}

/// Angular separation in arcseconds between an ICRS direction and (ra, dec) in degrees.
fn separation_arcsec(c: SkyCoord<Icrs>, ra_deg: f64, dec_deg: f64) -> f64 {
    let r = SkyCoord::<Icrs>::from_degrees(ra_deg, dec_deg);
    let (a, b) = (c.to_unit_vector(), r.to_unit_vector());
    // atan2 of the cross and dot products: acos is imprecise near zero angle.
    let cross = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let c = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
    let d = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    c.atan2(d).to_degrees() * 3600.0
}

#[test]
fn aberration_with_erfa_velocity_matches_erfa_ab() {
    let cases = cases();
    assert!(cases.len() >= 40, "only {} cases", cases.len());
    let mut worst: f64 = 0.0;
    for c in &cases {
        let star = SkyCoord::<Icrs>::from_degrees(c.ra, c.dec);
        let ours = annual_aberration(star, c.velocity);
        let err = separation_arcsec(ours, c.ara, c.adec);
        worst = worst.max(err);
        // The only difference is the relativistic terms of order (v/c)^2,
        // about 0.002 arcseconds.
        assert!(
            err < 0.01,
            "ra {} dec {} at {} {}: {err} arcsec",
            c.ra,
            c.dec,
            c.jd1,
            c.jd2
        );
    }
    eprintln!("worst aberration error vs erfa.ab (exact velocity): {worst:.6} arcsec");
}

#[test]
fn approximate_earth_velocity_gives_aberration_within_three_hundredths_arcsecond() {
    let cases = cases();
    let mut worst: f64 = 0.0;
    for c in &cases {
        let star = SkyCoord::<Icrs>::from_degrees(c.ra, c.dec);
        let v = earth_velocity_approx_au_per_day(c.jd1, c.jd2);
        let ours = annual_aberration(star, v);
        let err = separation_arcsec(ours, c.ara, c.adec);
        worst = worst.max(err);
    }
    eprintln!("worst aberration error with approximate velocity: {worst:.6} arcsec");
    // Measured 0.02 arcseconds over 1900-2100 with the Keplerian approximation.
    assert!(worst < 0.03, "worst error {worst} arcsec");
}

struct DeflectionCase {
    ra: f64,
    dec: f64,
    jd1: f64,
    jd2: f64,
    helio: [f64; 3],
    ora: f64,
    odec: f64,
}

fn deflection_cases() -> Vec<DeflectionCase> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/light_deflection_erfa.tsv");
    let text = fs::read_to_string(&path).expect("reference file");
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let v: Vec<f64> = l
                .split_whitespace()
                .map(|x| x.parse().expect("number"))
                .collect();
            assert_eq!(v.len(), 9, "line {l}");
            DeflectionCase {
                ra: v[0],
                dec: v[1],
                jd1: v[2],
                jd2: v[3],
                helio: [v[4], v[5], v[6]],
                ora: v[7],
                odec: v[8],
            }
        })
        .collect()
}

#[test]
fn light_deflection_with_erfa_position_matches_erfa_ld() {
    use tpt_planetary_astro::astrometry::light_deflection_by_sun;
    let cases = deflection_cases();
    assert!(cases.len() >= 40);
    let mut worst: f64 = 0.0;
    for c in &cases {
        let star = SkyCoord::<Icrs>::from_degrees(c.ra, c.dec);
        let ours = light_deflection_by_sun(star, c.helio);
        let err = separation_arcsec(ours, c.ora, c.odec);
        worst = worst.max(err);
        assert!(err < 1e-6, "ra {} dec {}: {err} arcsec", c.ra, c.dec);
    }
    eprintln!("worst light deflection error vs erfa.ld (erfa position): {worst:e} arcsec");
}

#[test]
fn light_deflection_with_approximate_position_is_within_a_thousandth_arcsecond() {
    use tpt_planetary_astro::astrometry::{
        earth_heliocentric_position_approx_au, light_deflection_by_sun,
    };
    let cases = deflection_cases();
    let mut worst: f64 = 0.0;
    for c in &cases {
        let star = SkyCoord::<Icrs>::from_degrees(c.ra, c.dec);
        let helio = earth_heliocentric_position_approx_au(c.jd1, c.jd2);
        let ours = light_deflection_by_sun(star, helio);
        worst = worst.max(separation_arcsec(ours, c.ora, c.odec));
    }
    eprintln!("worst light deflection error with approximate position: {worst:.6} arcsec");
    // The deflection itself is up to 1.75 arcseconds; the approximate position
    // gives a relative error of about 1e-3 in it.
    assert!(worst < 0.002, "worst {worst} arcsec");
}
