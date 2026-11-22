//! Reads an excerpt of the IERS finals2000A file (tools/gen_eop_fixture.py) and
//! checks the interpolated UT1-UTC and polar motion against astropy's IERS-B table.

use std::fs;
use std::path::Path;

use tpt_planetary_astro::eop::{EopError, EopTable, FINALS2000A_COLUMNS, parse_finals2000a};

fn fixture(name: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/eop")
            .join(name),
    )
    .expect("fixture")
}

/// Expected (mjd, UT1-UTC seconds, xp arcsec, yp arcsec) rows.
fn expected() -> Vec<(f64, f64, f64, f64)> {
    fixture("expected.tsv")
        .lines()
        .skip(1)
        .map(|l| {
            let f: Vec<f64> = l.split('\t').map(|v| v.parse().expect("number")).collect();
            (f[0], f[1], f[2], f[3])
        })
        .collect()
}

#[test]
fn excerpt_rows_parse_with_the_readme_columns() {
    let records = parse_finals2000a(&fixture("finals_excerpt.all"), &FINALS2000A_COLUMNS).unwrap();
    assert_eq!(records.len(), 14);
    assert_eq!(records[0].mjd, 58000.0);
    assert!((records[0].dut1_seconds - 0.3357915).abs() < 1e-12);
}

#[test]
fn ut1_minus_utc_interpolates_within_the_expected_difference() {
    let records = parse_finals2000a(&fixture("finals_excerpt.all"), &FINALS2000A_COLUMNS).unwrap();
    let table = EopTable::new(&records).unwrap();
    for (mjd, dut1, _, _) in expected() {
        let got = table.ut1_minus_utc(mjd).unwrap();
        // The finals rows and astropy's IERS-B series differ by up to about 30 us
        // at these epochs (two products, both interpolated linearly).
        assert!((got - dut1).abs() < 50e-6, "MJD {mjd}: {got} vs {dut1}");
    }
}

#[test]
fn polar_motion_interpolates_within_a_milliarcsecond() {
    let records = parse_finals2000a(&fixture("finals_excerpt.all"), &FINALS2000A_COLUMNS).unwrap();
    let table = EopTable::new(&records).unwrap();
    for (mjd, _, xp, yp) in expected() {
        let (gx, gy) = table.polar_motion_arcsec(mjd).unwrap();
        assert!((gx - xp).abs() < 1e-3, "MJD {mjd}: xp {gx} vs {xp}");
        assert!((gy - yp).abs() < 1e-3, "MJD {mjd}: yp {gy} vs {yp}");
    }
}

#[test]
fn epochs_outside_the_excerpt_are_errors() {
    let records = parse_finals2000a(&fixture("finals_excerpt.all"), &FINALS2000A_COLUMNS).unwrap();
    let table = EopTable::new(&records).unwrap();
    assert!(matches!(
        table.ut1_minus_utc(50000.0),
        Err(EopError::OutOfRange { .. })
    ));
    assert!(
        matches!(table.ut1_minus_utc(58250.0), Err(EopError::Gap { .. })),
        "gap between windows"
    );
}
