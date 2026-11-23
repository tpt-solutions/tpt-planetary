//! Round trip of ASCII tables through the crate's FITS writer and reader.

use tpt_planetary_astro::fits::{
    AsciiData, AsciiKind, AsciiTableColumn, FitsError, hdus, write_ascii_table,
};

fn read_back(bytes: &[u8]) -> tpt_planetary_astro::fits::AsciiTable<'_> {
    // The extension is the second HDU; the table borrows from `bytes`.
    let hdu = hdus(bytes).nth(1).expect("extension").expect("valid HDU");
    assert!(hdu.is_ascii_table());
    hdu.ascii_table().expect("ASCII table")
}

#[test]
fn every_field_type_reads_back() {
    let labels = ["alpha", "b", "", "delta"];
    let ints = [-3i64, 0, -999, 12];
    let fixed = [0.5, -1.25, f64::NAN, 1000.125];
    let expo = [1.5e-3, -2.0, 3.25e5, 0.0];
    let cols = [
        AsciiTableColumn {
            name: "label",
            data: AsciiData::Text(&labels, 6),
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
        AsciiTableColumn {
            name: "count",
            data: AsciiData::Int(&ints, 6),
            tscal: 1.0,
            tzero: 0.0,
            tnull: Some(-999),
        },
        AsciiTableColumn {
            name: "fixed",
            data: AsciiData::Fixed(&fixed, 9, 3),
            // Stored values are written; physical = 10 + 0.5 * stored.
            tscal: 0.5,
            tzero: 10.0,
            tnull: None,
        },
        AsciiTableColumn {
            name: "expo",
            data: AsciiData::Exp(&expo, 12, 4),
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
    ];
    let bytes = write_ascii_table(&cols, &[]).unwrap();
    assert_eq!(bytes.len() % 2880, 0, "whole FITS blocks");
    let table = read_back(&bytes);
    assert_eq!(table.rows(), 4);
    assert_eq!(table.column("label").unwrap().kind, AsciiKind::Char);
    assert_eq!(
        table.read_strings("label").unwrap(),
        ["alpha", "b", "", "delta"]
    );
    let count = table.read_f64("count").unwrap();
    assert_eq!(&count[..2], &[-3.0, 0.0]);
    assert!(count[2].is_nan(), "TNULL value is undefined");
    assert_eq!(count[3], 12.0);
    let fx = table.read_f64("fixed").unwrap();
    assert!((fx[0] - 10.25).abs() < 1e-12, "{}", fx[0]);
    assert!((fx[1] - 9.375).abs() < 1e-12);
    assert!(fx[2].is_nan(), "a non-finite stored value is a blank field");
    assert!((fx[3] - (10.0 + 0.5 * 1000.125)).abs() < 1e-9);
    let ex = table.read_f64("expo").unwrap();
    for (got, want) in ex.iter().zip(expo) {
        assert!(
            (got - want).abs() <= 1e-4 * want.abs().max(1e-3),
            "{got} vs {want}"
        );
    }
}

#[test]
fn exponent_form_is_fortran_style() {
    let values = [1234.5f64];
    let cols = [AsciiTableColumn {
        name: "x",
        data: AsciiData::Exp(&values, 12, 4),
        tscal: 1.0,
        tzero: 0.0,
        tnull: None,
    }];
    let bytes = write_ascii_table(&cols, &[]).unwrap();
    let row = &bytes[2880 * 2..2880 * 2 + 12];
    assert_eq!(std::str::from_utf8(row).unwrap(), "  1.2345E+03");
}

#[test]
fn a_value_wider_than_its_field_is_rejected() {
    let values = [123456.0f64];
    let cols = [AsciiTableColumn {
        name: "x",
        data: AsciiData::Fixed(&values, 5, 2),
        tscal: 1.0,
        tzero: 0.0,
        tnull: None,
    }];
    assert!(matches!(
        write_ascii_table(&cols, &[]),
        Err(FitsError::ShapeMismatch)
    ));
}

#[test]
fn a_long_string_is_rejected() {
    let labels = ["far too long"];
    let cols = [AsciiTableColumn {
        name: "s",
        data: AsciiData::Text(&labels, 4),
        tscal: 1.0,
        tzero: 0.0,
        tnull: None,
    }];
    assert!(matches!(
        write_ascii_table(&cols, &[]),
        Err(FitsError::ShapeMismatch)
    ));
}

#[test]
fn mismatched_row_counts_are_rejected() {
    let a = [1i64, 2, 3];
    let b = [1.0f64, 2.0];
    let cols = [
        AsciiTableColumn {
            name: "a",
            data: AsciiData::Int(&a, 4),
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
        AsciiTableColumn {
            name: "b",
            data: AsciiData::Fixed(&b, 8, 2),
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
    ];
    assert!(matches!(
        write_ascii_table(&cols, &[]),
        Err(FitsError::ShapeMismatch)
    ));
}

/// Writes the test table to the path in `TPT_FITS_ASCII_OUT`, for an external
/// reader (astropy). Does nothing when the variable is unset.
#[test]
fn writes_a_file_for_external_checks() {
    let Ok(path) = std::env::var("TPT_FITS_ASCII_OUT") else {
        return;
    };
    let labels = ["alpha", "b", "", "delta"];
    let ints = [-3i64, 0, -999, 12];
    let fixed = [0.5, -1.25, f64::NAN, 1000.125];
    let expo = [1.5e-3, -2.0, 3.25e5, 0.0];
    let cols = [
        AsciiTableColumn {
            name: "label",
            data: AsciiData::Text(&labels, 6),
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
        AsciiTableColumn {
            name: "count",
            data: AsciiData::Int(&ints, 6),
            tscal: 1.0,
            tzero: 0.0,
            tnull: Some(-999),
        },
        AsciiTableColumn {
            name: "fixed",
            data: AsciiData::Fixed(&fixed, 9, 3),
            tscal: 0.5,
            tzero: 10.0,
            tnull: None,
        },
        AsciiTableColumn {
            name: "expo",
            data: AsciiData::Exp(&expo, 12, 4),
            tscal: 1.0,
            tzero: 0.0,
            tnull: None,
        },
    ];
    let bytes = write_ascii_table(&cols, &[]).unwrap();
    std::fs::write(path, bytes).unwrap();
}
