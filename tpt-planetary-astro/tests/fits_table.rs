//! Reads the FITS binary table written by astropy (tools/gen_fits_table_fixtures.py)
//! and compares every column with astropy's physical values.

use std::fs;
use std::path::Path;

use tpt_planetary_astro::fits::{ColumnKind, FitsError, hdus};

/// One expected column: its name, its numeric values (`None` for undefined),
/// and its text values (for the label column).
struct Expected {
    name: String,
    numbers: Vec<Option<f64>>,
    text: Vec<String>,
}

fn expected() -> Vec<Expected> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fits/expected.tsv");
    fs::read_to_string(path)
        .expect("expected values")
        .lines()
        .map(|line| {
            let (name, rest) = line.split_once('\t').expect("name and values");
            let items: Vec<&str> = rest.split(' ').collect();
            Expected {
                name: name.to_string(),
                numbers: items
                    .iter()
                    .map(|v| {
                        if *v == "nan" {
                            None
                        } else {
                            v.parse::<f64>().ok()
                        }
                    })
                    .collect(),
                text: items
                    .iter()
                    .map(|v| {
                        if *v == "_" {
                            String::new()
                        } else {
                            (*v).to_string()
                        }
                    })
                    .collect(),
            }
        })
        .collect()
}

fn table_bytes() -> Vec<u8> {
    fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fits/table.fits"))
        .expect("fixture")
}

#[test]
fn every_numeric_column_matches_astropy() {
    let bytes = table_bytes();
    let table_hdu = hdus(&bytes).nth(1).expect("extension").expect("valid HDU");
    assert!(table_hdu.is_table());
    let table = table_hdu.table().expect("binary table");
    assert_eq!(table.rows(), 5);
    let mut checked = 0;
    for column in expected() {
        if column.name == "label" {
            continue;
        }
        let got = table
            .read_f64(&column.name)
            .unwrap_or_else(|e| panic!("{}: {e:?}", column.name));
        assert_eq!(got.len(), column.numbers.len(), "{}: length", column.name);
        for (k, (g, w)) in got.iter().zip(&column.numbers).enumerate() {
            match w {
                None => assert!(
                    g.is_nan(),
                    "{}[{k}] should be undefined, got {g}",
                    column.name
                ),
                Some(w) => assert!(
                    (g - w).abs() <= 1e-12 * w.abs().max(1.0),
                    "{}[{k}]: {g} vs {w}",
                    column.name
                ),
            }
        }
        checked += 1;
    }
    assert_eq!(checked, 10);
}

#[test]
fn strings_and_column_layout() {
    let bytes = table_bytes();
    let table = hdus(&bytes).nth(1).unwrap().unwrap().table().unwrap();
    let want = expected()
        .into_iter()
        .find(|c| c.name == "label")
        .expect("labels")
        .text;
    assert_eq!(table.read_strings("label").unwrap(), want);
    let vec = table.column("vec").unwrap();
    assert_eq!(vec.kind, ColumnKind::Float);
    assert_eq!(vec.repeat, 3);
    assert_eq!(table.column("id").unwrap().kind, ColumnKind::Int);
    assert_eq!(table.row_bytes(), 54);
    assert!(
        table.read_f64("label").is_err(),
        "a string column is not numeric"
    );
}

#[test]
fn image_hdu_is_not_a_table() {
    let bytes = table_bytes();
    let primary = hdus(&bytes).next().unwrap().unwrap();
    assert!(!primary.is_table());
    assert!(matches!(primary.table(), Err(FitsError::InvalidHeader(_))));
}
