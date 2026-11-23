//! Reads the FITS ASCII table written by astropy (tools/gen_fits_ascii_fixture.py)
//! and compares every column with astropy's physical values, with undefined
//! values as NaN.

use std::fs;
use std::path::Path;

use tpt_planetary_astro::fits::{AsciiKind, FitsError, hdus};

fn fixture_bytes() -> Vec<u8> {
    fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fits/ascii.fits"))
        .expect("fixture")
}

/// (name, values) from ascii_expected.tsv; `nan` is NaN and `_` is an empty string.
fn expected() -> Vec<(String, Vec<String>)> {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fits/ascii_expected.tsv"),
    )
    .expect("expected values")
    .lines()
    .map(|line| {
        let (name, rest) = line.split_once('\t').expect("name and values");
        (
            name.to_string(),
            rest.split(' ').map(str::to_string).collect(),
        )
    })
    .collect()
}

fn same_number(got: f64, want: &str) -> bool {
    if want == "nan" {
        got.is_nan()
    } else {
        let w: f64 = want.parse().expect("number");
        (got - w).abs() <= 1e-12 * w.abs().max(1.0)
    }
}

#[test]
fn the_extension_is_an_ascii_table() {
    let bytes = fixture_bytes();
    let hdu = hdus(&bytes).nth(1).expect("extension").expect("valid HDU");
    assert!(hdu.is_ascii_table());
    assert!(!hdu.is_table());
    let table = hdu.ascii_table().unwrap();
    assert_eq!(table.rows(), 4);
    let kinds: Vec<AsciiKind> = table.columns().iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [
            AsciiKind::Char,
            AsciiKind::Int,
            AsciiKind::Float,
            AsciiKind::Float,
            AsciiKind::Float
        ]
    );
    let names: Vec<&str> = table.columns().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["label", "count", "fixed", "expo", "dbl"]);
}

#[test]
fn every_column_matches_astropy() {
    let bytes = fixture_bytes();
    let hdu = hdus(&bytes).nth(1).expect("extension").expect("valid HDU");
    let table = hdu.ascii_table().unwrap();
    for (name, values) in expected() {
        let col = table.column(&name).expect("column");
        if col.kind == AsciiKind::Char {
            let got = table.read_strings(&name).unwrap();
            let want: Vec<String> = values
                .iter()
                .map(|v| if v == "_" { String::new() } else { v.clone() })
                .collect();
            assert_eq!(got, want, "column {name}");
        } else {
            let got = table.read_f64(&name).unwrap();
            assert_eq!(got.len(), values.len(), "column {name}");
            for (i, (g, w)) in got.iter().zip(&values).enumerate() {
                assert!(
                    same_number(*g, w),
                    "column {name} row {i}: got {g}, want {w}"
                );
            }
        }
    }
}

#[test]
fn an_undefined_integer_field_is_nan_and_the_scaling_applies() {
    let bytes = fixture_bytes();
    let table = hdus(&bytes).nth(1).unwrap().unwrap().ascii_table().unwrap();
    let count = table.read_f64("count").unwrap();
    assert!(count[2].is_nan(), "TNULL row is undefined");
    // The F column stores (physical - 10) / 0.5 as text: -19.000 is 0.5.
    let fixed = table.read_f64("fixed").unwrap();
    assert!((fixed[0] - 0.5).abs() < 1e-12);
    assert!((fixed[3] - 1000.125).abs() < 1e-9);
}

#[test]
fn a_binary_table_is_not_read_as_ascii() {
    // The binary table fixture is a BINTABLE, so the ASCII reader refuses it.
    let bytes =
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fits/table.fits"))
            .unwrap();
    let hdu = hdus(&bytes).nth(1).unwrap().unwrap();
    assert!(matches!(
        hdu.ascii_table(),
        Err(FitsError::InvalidHeader(_))
    ));
}
