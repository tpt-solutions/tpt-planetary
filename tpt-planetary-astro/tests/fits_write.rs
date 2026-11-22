//! Round trip of binary tables through the crate's FITS writer and reader.

use tpt_planetary_astro::fits::{ColumnData, ColumnKind, TableColumn, hdus, write_bintable};

/// The values of the test table, one vector per column.
struct Values {
    ints: Vec<i32>,
    doubles: Vec<f64>,
    flags: Vec<bool>,
    labels: Vec<&'static str>,
    shorts: Vec<i16>,
    floats: Vec<f32>,
}

fn columns() -> Values {
    Values {
        ints: vec![-3, 0, 7, 100000],
        doubles: vec![0.5, -1.25, 3.0e10, f64::MIN_POSITIVE],
        flags: vec![true, false, true, true],
        labels: vec!["alpha", "b", "", "delta-epsilon"],
        shorts: vec![-300, 0, 12, 32767],
        floats: vec![0.25, -2.5, 1.0e-3, 7.0],
    }
}

#[test]
fn written_table_reads_back_exactly() {
    let Values {
        ints,
        doubles,
        flags,
        labels,
        shorts,
        floats,
    } = columns();
    let cols = [
        TableColumn {
            name: "n",
            data: ColumnData::Int(&ints),
            repeat: 1,
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
        TableColumn {
            name: "x",
            data: ColumnData::Double(&doubles),
            repeat: 1,
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
        TableColumn {
            name: "ok",
            data: ColumnData::Logical(&flags),
            repeat: 1,
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
        TableColumn {
            name: "label",
            data: ColumnData::Text(&labels, 13),
            repeat: 1,
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
        TableColumn {
            name: "s",
            data: ColumnData::Short(&shorts),
            repeat: 1,
            tscal: 0.5,
            tzero: 10.0,
            tnull: Some(-300),
        },
        TableColumn {
            name: "f",
            data: ColumnData::Float(&floats),
            repeat: 1,
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
    ];
    let bytes = write_bintable(&cols, &[]).unwrap();
    assert_eq!(bytes.len() % 2880, 0, "whole FITS blocks");

    let table_hdu = hdus(&bytes).nth(1).unwrap().unwrap();
    assert!(table_hdu.is_table());
    let table = table_hdu.table().unwrap();
    assert_eq!(table.rows(), 4);
    assert_eq!(table.columns().len(), 6);
    assert_eq!(table.column("n").unwrap().kind, ColumnKind::Int);

    let ints_back: Vec<f64> = ints.iter().map(|&v| f64::from(v)).collect();
    assert_eq!(table.read_f64("n").unwrap(), ints_back);
    assert_eq!(table.read_f64("x").unwrap(), doubles);
    assert_eq!(table.read_f64("ok").unwrap(), vec![1.0, 0.0, 1.0, 1.0]);
    assert_eq!(table.read_strings("label").unwrap(), labels);
    // Scaling: physical = 10 + 0.5 * stored; the stored -300 is undefined.
    let s = table.read_f64("s").unwrap();
    assert!(s[0].is_nan());
    assert_eq!(s[1], 10.0);
    assert_eq!(s[2], 16.0);
    assert_eq!(s[3], 10.0 + 0.5 * 32767.0);
    assert_eq!(
        table.read_f64("f").unwrap(),
        floats.iter().map(|&v| f64::from(v)).collect::<Vec<_>>()
    );
}

#[test]
fn vector_columns_keep_their_repeat_count() {
    let values: Vec<f32> = (0..6).map(|v| v as f32 * 0.5).collect();
    let cols = [TableColumn {
        name: "vec",
        data: ColumnData::Float(&values),
        repeat: 3,
        tscal: 1.0,
        tzero: 0.0,
        tnull: None,
    }];
    let bytes = write_bintable(&cols, &[]).unwrap();
    let table = hdus(&bytes).nth(1).unwrap().unwrap().table().unwrap();
    assert_eq!(table.rows(), 2);
    assert_eq!(table.column("vec").unwrap().repeat, 3);
    assert_eq!(
        table.read_f64("vec").unwrap(),
        values.iter().map(|&v| f64::from(v)).collect::<Vec<_>>()
    );
}

#[test]
fn strings_wider_than_their_column_are_rejected() {
    let labels = ["far too long for four"];
    let cols = [TableColumn {
        name: "s",
        data: ColumnData::Text(&labels, 4),
        repeat: 1,
        tscal: 1.0,
        tzero: 0.0,
        tnull: None,
    }];
    assert!(write_bintable(&cols, &[]).is_err());
}

#[test]
fn mismatched_row_counts_are_rejected() {
    let a = [1i32, 2, 3];
    let b = [1.0f64, 2.0];
    let cols = [
        TableColumn {
            name: "a",
            data: ColumnData::Int(&a),
            repeat: 1,
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
        TableColumn {
            name: "b",
            data: ColumnData::Double(&b),
            repeat: 1,
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
    ];
    assert!(write_bintable(&cols, &[]).is_err());
}

/// Writes the test table to the path in `TPT_FITS_WRITE_OUT`, for an external
/// reader (astropy, in the Python validation). Does nothing when the variable is unset.
#[test]
fn writes_a_file_for_external_checks() {
    let Ok(path) = std::env::var("TPT_FITS_WRITE_OUT") else {
        return;
    };
    let Values {
        ints,
        doubles,
        flags,
        labels,
        shorts,
        floats,
    } = columns();
    let cols = [
        TableColumn {
            name: "n",
            data: ColumnData::Int(&ints),
            repeat: 1,
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
        TableColumn {
            name: "x",
            data: ColumnData::Double(&doubles),
            repeat: 1,
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
        TableColumn {
            name: "ok",
            data: ColumnData::Logical(&flags),
            repeat: 1,
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
        TableColumn {
            name: "label",
            data: ColumnData::Text(&labels, 13),
            repeat: 1,
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
        TableColumn {
            name: "s",
            data: ColumnData::Short(&shorts),
            repeat: 1,
            tscal: 0.5,
            tzero: 10.0,
            tnull: Some(-300),
        },
        TableColumn {
            name: "f",
            data: ColumnData::Float(&floats),
            repeat: 1,
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
    ];
    let bytes = write_bintable(&cols, &[]).unwrap();
    std::fs::write(path, bytes).unwrap();
}
