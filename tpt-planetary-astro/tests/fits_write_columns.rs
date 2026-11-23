//! Round trip of bit, complex and variable-length binary table columns through
//! the crate's writer and reader. With TPT_FITS_COLUMNS_OUT set, the test also
//! writes the table to that path for an external reader (astropy).

use tpt_planetary_astro::fits::{
    ColumnData, ColumnKind, FitsError, TableColumn, VarElements, hdus, write_bintable,
};

fn plain<'a>(name: &'a str, data: ColumnData<'a>, repeat: usize) -> TableColumn<'a> {
    TableColumn {
        name,
        data,
        repeat,
        tscal: 1.0,
        tzero: 0.0,
        tnull: None,
    }
}

/// The columns of the test table, shared by the round trip and the external file.
struct Values {
    bits: Vec<bool>,
    cf: Vec<[f32; 2]>,
    cd: Vec<[f64; 2]>,
    ints: Vec<Vec<i32>>,
    doubles: Vec<Vec<f64>>,
}

fn values() -> Values {
    Values {
        // Four rows of 12 bits.
        bits: (0..48).map(|k| (k * 7 + 3) % 5 == 1).collect(),
        // Four rows of two complex single values.
        cf: vec![
            [1.5, -2.0],
            [0.25, 3.0],
            [-4.0, 0.5],
            [7.0, -1.0],
            [2.0, 2.0],
            [0.5, -0.5],
            [-8.0, 4.0],
            [1.0, 0.0],
        ],
        // One complex double per row, four rows.
        cd: vec![[1.0e-7, -2.5], [-3.0, 1.0e6], [0.0, 0.0], [5.0, 5.0]],
        // Variable-length: rows of different lengths, one empty.
        ints: vec![vec![1, -2, 3], vec![], vec![40], vec![-5, 6]],
        doubles: vec![vec![0.5], vec![1.5, -2.25, 1.0e-3], vec![], vec![7.0, 8.0]],
    }
}

/// The row slices of the variable-length columns, borrowed from `v`.
fn rows(v: &Values) -> (Vec<&[i32]>, Vec<&[f64]>) {
    (
        v.ints.iter().map(Vec::as_slice).collect(),
        v.doubles.iter().map(Vec::as_slice).collect(),
    )
}

fn table<'a>(
    v: &'a Values,
    ints: &'a [&'a [i32]],
    doubles: &'a [&'a [f64]],
) -> Vec<TableColumn<'a>> {
    vec![
        plain("flags", ColumnData::Bit(&v.bits), 12),
        plain("cf", ColumnData::ComplexFloat(&v.cf), 2),
        plain("cd", ColumnData::ComplexDouble(&v.cd), 1),
        plain(
            "ints",
            ColumnData::Variable(VarElements::Int(ints), false),
            1,
        ),
        plain(
            "doubles",
            ColumnData::Variable(VarElements::Double(doubles), true),
            1,
        ),
    ]
}

#[test]
fn every_column_reads_back() {
    let v = values();
    let (ints, doubles) = rows(&v);
    let cols = table(&v, &ints, &doubles);
    let bytes = write_bintable(&cols, &[]).unwrap();
    assert_eq!(bytes.len() % 2880, 0, "whole FITS blocks");
    let table = hdus(&bytes).nth(1).unwrap().unwrap().table().unwrap();
    assert_eq!(table.rows(), 4);
    assert_eq!(table.column("flags").unwrap().kind, ColumnKind::Bit);
    assert_eq!(table.column("cf").unwrap().kind, ColumnKind::ComplexFloat);
    assert_eq!(table.column("cd").unwrap().kind, ColumnKind::ComplexDouble);
    assert_eq!(
        table.column("ints").unwrap().kind,
        ColumnKind::Variable('J', false)
    );
    assert_eq!(
        table.column("doubles").unwrap().kind,
        ColumnKind::Variable('D', true)
    );

    assert_eq!(table.read_bits("flags").unwrap(), v.bits);
    let cf: Vec<(f64, f64)> =
        v.cf.iter()
            .map(|z| (f64::from(z[0]), f64::from(z[1])))
            .collect();
    assert_eq!(table.read_complex("cf").unwrap(), cf);
    let cd: Vec<(f64, f64)> = v.cd.iter().map(|z| (z[0], z[1])).collect();
    assert_eq!(table.read_complex("cd").unwrap(), cd);
    let ints: Vec<Vec<f64>> = v
        .ints
        .iter()
        .map(|r| r.iter().map(|&x| f64::from(x)).collect())
        .collect();
    assert_eq!(table.read_vla("ints").unwrap(), ints);
    assert_eq!(table.read_vla("doubles").unwrap(), v.doubles);
}

#[test]
fn a_bit_column_packs_the_row_in_bytes() {
    // Twelve bits per row take two bytes; the first bit is the most significant.
    let bits = [
        true, false, false, false, false, false, false, false, true, true, false, false,
    ];
    let cols = [plain("b", ColumnData::Bit(&bits), 12)];
    let bytes = write_bintable(&cols, &[]).unwrap();
    let row = &bytes[2880 * 2..2880 * 2 + 2];
    assert_eq!(row, &[0b1000_0000, 0b1100_0000]);
}

#[test]
fn a_bit_column_needs_whole_rows() {
    let bits = [true, false, true];
    let cols = [plain("b", ColumnData::Bit(&bits), 2)];
    assert!(matches!(
        write_bintable(&cols, &[]),
        Err(FitsError::ShapeMismatch)
    ));
}

#[test]
fn variable_columns_with_a_different_row_count_are_rejected() {
    let a: Vec<&[i32]> = vec![&[1], &[2]];
    let b: Vec<f64> = vec![1.0];
    let cols = [
        plain("a", ColumnData::Variable(VarElements::Int(&a), false), 1),
        plain("b", ColumnData::Double(&b), 1),
    ];
    assert!(matches!(
        write_bintable(&cols, &[]),
        Err(FitsError::ShapeMismatch)
    ));
}

/// Writes the test table to the path in `TPT_FITS_COLUMNS_OUT`, for astropy.
#[test]
fn writes_a_file_for_external_checks() {
    let Ok(path) = std::env::var("TPT_FITS_COLUMNS_OUT") else {
        return;
    };
    let v = values();
    let (ints, doubles) = rows(&v);
    let cols = table(&v, &ints, &doubles);
    std::fs::write(path, write_bintable(&cols, &[]).unwrap()).unwrap();
}
