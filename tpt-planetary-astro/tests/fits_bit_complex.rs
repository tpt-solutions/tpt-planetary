//! Reads the FITS binary table with bit (`X`) and complex (`C`, `M`) columns that
//! astropy wrote (tools/gen_fits_bit_complex_fixture.py), and compares every value
//! with astropy's.

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

fn expected_line(name: &str) -> String {
    fixture("bitcomplex_expected.tsv")
        .lines()
        .find_map(|l| {
            l.strip_prefix(name)
                .and_then(|r| r.strip_prefix('\t'))
                .map(str::to_string)
        })
        .expect("expected line")
}

fn parse_complex(text: &str) -> Vec<(f64, f64)> {
    text.split(' ')
        .map(|pair| {
            let (re, im) = pair.split_once(',').expect("re,im");
            (re.parse().expect("real"), im.parse().expect("imaginary"))
        })
        .collect()
}

#[test]
fn the_columns_have_the_new_kinds() {
    let bytes =
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fits/bitcomplex.fits"))
            .expect("fixture");
    let table = hdus(&bytes).nth(1).unwrap().unwrap().table().unwrap();
    assert_eq!(table.rows(), 3);
    assert_eq!(table.column("flags").unwrap().kind, ColumnKind::Bit);
    assert_eq!(table.column("flags").unwrap().repeat, 12);
    assert_eq!(table.column("pair").unwrap().kind, ColumnKind::ComplexFloat);
    assert_eq!(table.column("z").unwrap().kind, ColumnKind::ComplexDouble);
}

#[test]
fn bits_match_astropy() {
    let bytes =
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fits/bitcomplex.fits"))
            .expect("fixture");
    let table = hdus(&bytes).nth(1).unwrap().unwrap().table().unwrap();
    let got: Vec<u8> = table
        .read_bits("flags")
        .unwrap()
        .into_iter()
        .map(u8::from)
        .collect();
    let want: Vec<u8> = expected_line("flags")
        .split(' ')
        .map(|v| v.parse().expect("bit"))
        .collect();
    assert_eq!(got, want);
}

#[test]
fn complex_values_match_astropy() {
    let bytes =
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fits/bitcomplex.fits"))
            .expect("fixture");
    let table = hdus(&bytes).nth(1).unwrap().unwrap().table().unwrap();
    // Single precision is exact for these values; double precision is exact too.
    assert_eq!(
        table.read_complex("pair").unwrap(),
        parse_complex(&expected_line("pair"))
    );
    assert_eq!(
        table.read_complex("z").unwrap(),
        parse_complex(&expected_line("z"))
    );
}

#[test]
fn bit_and_complex_columns_refuse_the_numeric_readers() {
    let bytes =
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fits/bitcomplex.fits"))
            .expect("fixture");
    let table = hdus(&bytes).nth(1).unwrap().unwrap().table().unwrap();
    assert!(matches!(
        table.read_f64("flags"),
        Err(FitsError::Unsupported(_))
    ));
    assert!(matches!(
        table.read_f64("pair"),
        Err(FitsError::Unsupported(_))
    ));
    assert!(matches!(
        table.read_bits("z"),
        Err(FitsError::InvalidHeader(_))
    ));
    assert!(matches!(
        table.read_complex("flags"),
        Err(FitsError::InvalidHeader(_))
    ));
}
