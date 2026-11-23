//! Reads the FITS binary table with variable-length (`P` and `Q`) columns that
//! astropy wrote (tools/gen_fits_vla_fixture.py), and compares every row with
//! astropy's.

use std::fs;
use std::path::Path;

use tpt_planetary_astro::fits::{ColumnKind, FitsError, hdus};

fn fixture(name: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/fits")
            .join(name),
    )
    .expect("fixture")
}

/// Rows of a column from the expected file: rows separated by `|`, elements by spaces.
fn expected_rows(name: &str) -> Vec<Vec<f64>> {
    let line = fixture("vla_expected.tsv")
        .lines()
        .find_map(|l| {
            l.strip_prefix(name)
                .and_then(|r| r.strip_prefix('\t'))
                .map(str::to_string)
        })
        .expect("expected line");
    line.split('|')
        .map(|row| {
            row.split(' ')
                .filter(|v| !v.is_empty())
                .map(|v| v.parse().expect("number"))
                .collect()
        })
        .collect()
}

fn bytes() -> Vec<u8> {
    fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fits/vla.fits"))
        .expect("fixture")
}

#[test]
fn variable_columns_have_the_variable_kind() {
    let data = bytes();
    let table = hdus(&data).nth(1).unwrap().unwrap().table().unwrap();
    assert_eq!(table.rows(), 4);
    assert_eq!(
        table.column("ints").unwrap().kind,
        ColumnKind::Variable('J', false)
    );
    assert_eq!(
        table.column("dbls").unwrap().kind,
        ColumnKind::Variable('D', true)
    );
    assert_eq!(
        table.column("bytes").unwrap().kind,
        ColumnKind::Variable('B', false)
    );
}

#[test]
fn every_row_matches_astropy() {
    let data = bytes();
    let table = hdus(&data).nth(1).unwrap().unwrap().table().unwrap();
    for name in ["ints", "dbls", "bytes"] {
        let got = table.read_vla(name).unwrap();
        let want = expected_rows(name);
        assert_eq!(got.len(), want.len(), "column {name}");
        for (r, (g, w)) in got.iter().zip(&want).enumerate() {
            assert_eq!(g, w, "column {name} row {r}");
        }
    }
}

#[test]
fn an_empty_row_is_an_empty_array() {
    let data = bytes();
    let table = hdus(&data).nth(1).unwrap().unwrap().table().unwrap();
    assert!(table.read_vla("dbls").unwrap()[2].is_empty());
    assert!(table.read_vla("ints").unwrap()[1].is_empty());
}

#[test]
fn variable_columns_refuse_the_fixed_readers() {
    let data = bytes();
    let table = hdus(&data).nth(1).unwrap().unwrap().table().unwrap();
    assert!(matches!(
        table.read_f64("ints"),
        Err(FitsError::Unsupported(_))
    ));
    assert!(matches!(
        table.read_vla("missing"),
        Err(FitsError::InvalidHeader(_))
    ));
}
