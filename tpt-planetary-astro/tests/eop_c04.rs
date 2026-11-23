//! Reads an excerpt of the IERS C04 series (`eopc04.1962-now`, fetched from
//! hpiers.obspm.fr on 2026-11-23) and checks the parsed rows, the format
//! detection, and agreement with the finals2000A excerpt over the shared dates.

use std::fs;
use std::path::Path;

use tpt_planetary_astro::eop::{
    EopError, EopTable, FINALS2000A_COLUMNS, parse_c04, parse_eop_text, parse_finals2000a,
};

fn fixture(name: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/eop")
            .join(name),
    )
    .expect("fixture")
}

#[test]
fn c04_rows_parse_with_the_readme_columns() {
    let records = parse_c04(&fixture("c04_excerpt.txt")).unwrap();
    assert_eq!(records.len(), 19);
    // 2017-09-05, the MJD 58001 row of the series.
    let r = records.iter().find(|r| r.mjd == 58001.0).expect("row");
    assert!((r.xp_arcsec - 0.238065).abs() < 1e-12);
    assert!((r.yp_arcsec - 0.349979).abs() < 1e-12);
    assert!((r.dut1_seconds - 0.3353744).abs() < 1e-12);
    // LOD is 0.0004771 s in the file; it is returned in milliseconds.
    assert!((r.lod_ms.unwrap() - 0.4771).abs() < 1e-9);
}

#[test]
fn c04_rows_are_daily_and_ascending() {
    let records = parse_c04(&fixture("c04_excerpt.txt")).unwrap();
    for pair in records.windows(2) {
        assert_eq!(pair[1].mjd - pair[0].mjd, 1.0);
    }
}

#[test]
fn format_is_detected_from_the_header() {
    let c04 = parse_eop_text(&fixture("c04_excerpt.txt")).unwrap();
    assert_eq!(c04.len(), 19);
    let finals = parse_eop_text(&fixture("finals_excerpt.all")).unwrap();
    assert_eq!(finals.len(), 14);
    assert_eq!(
        finals,
        parse_finals2000a(&fixture("finals_excerpt.all"), &FINALS2000A_COLUMNS).unwrap()
    );
}

#[test]
fn c04_and_finals_agree_on_the_shared_dates() {
    let c04 = parse_c04(&fixture("c04_excerpt.txt")).unwrap();
    let finals = parse_finals2000a(&fixture("finals_excerpt.all"), &FINALS2000A_COLUMNS).unwrap();
    let mut shared = 0;
    for f in &finals {
        if let Some(c) = c04.iter().find(|c| c.mjd == f.mjd) {
            shared += 1;
            // Two products of the same series: UT1-UTC within 0.5 ms and polar
            // motion within 1 mas (0.001 arcsec).
            assert!(
                (c.dut1_seconds - f.dut1_seconds).abs() < 0.5e-3,
                "mjd {}",
                f.mjd
            );
            assert!((c.xp_arcsec - f.xp_arcsec).abs() < 1e-3, "mjd {}", f.mjd);
            assert!((c.yp_arcsec - f.yp_arcsec).abs() < 1e-3, "mjd {}", f.mjd);
        }
    }
    // The finals excerpt has four dates in the C04 window (2017-09-04 to 09-07).
    assert_eq!(shared, 4);
}

#[test]
fn c04_table_interpolates() {
    let records = parse_c04(&fixture("c04_excerpt.txt")).unwrap();
    let table = EopTable::new(&records).unwrap();
    let a = table.ut1_minus_utc(58001.0).unwrap();
    let b = table.ut1_minus_utc(58002.0).unwrap();
    let mid = table.ut1_minus_utc(58001.5).unwrap();
    assert!((mid - (a + b) / 2.0).abs() < 1e-12);
}

#[test]
fn a_malformed_c04_row_reports_its_line() {
    let mut text = fixture("c04_excerpt.txt");
    // Blank the MJD of the first data row (line 7 of the excerpt).
    let bad = text
        .lines()
        .enumerate()
        .map(|(i, l)| {
            if i == 6 {
                format!("{}          {}", &l[..16], &l[26..])
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    text = bad;
    match parse_c04(&text) {
        Err(EopError::BadLine { line, field }) => {
            assert_eq!(line, 7);
            assert_eq!(field, "mjd");
        }
        other => panic!("expected BadLine, got {other:?}"),
    }
}
