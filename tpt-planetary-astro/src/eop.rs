//! IERS Earth Orientation Parameters (EOP): parsing of the `finals2000A`
//! fixed-width format, interpolation of UT1-UTC and polar motion, and the
//! UT1 time scale.
//!
//! This module does not use trigonometry and works without `std` and without
//! allocation. Only [`parse_finals2000a`] needs `std`, because it returns a
//! `Vec`.
//!
//! # Column layout
//!
//! The fixed column positions are described by [`ColumnMap`]. The default
//! map, [`FINALS2000A_COLUMNS`], was checked against the IERS readme for
//! `finals2000A.data` (column table as published at
//! <https://maia.usno.navy.mil/ser7/readme.finals2000A>, retrieved 2026-11-16).
//! The unit tests build most lines from the map itself, so they check the
//! parser's arithmetic. The test `parses_readme_layout_line` uses a line laid
//! out by hand from the readme table, which checks the map against that table.
//!
//! # Interpolation
//!
//! UT1-UTC and polar motion are interpolated linearly in UTC Modified Julian
//! Date between the two bracketing rows. Linear interpolation is not valid
//! across a leap second, where UT1-UTC steps by 1 s; the table is not checked
//! for leap seconds. Epochs outside the table range return an error. Nothing
//! is extrapolated.

use core::fmt;

use tpt_planetary_core::constants::{MJD_OFFSET, SECONDS_PER_DAY};

/// Arcseconds to radians, `pi / (180 * 3600)`, exact by definition.
pub const ARCSEC_TO_RAD: f64 = core::f64::consts::PI / (180.0 * 3600.0);

/// A half-open byte range `[start, end)` within a line, zero-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// First byte of the field (zero-based, inclusive).
    pub start: usize,
    /// One past the last byte of the field (zero-based, exclusive).
    pub end: usize,
}

impl Span {
    /// Creates a span from zero-based inclusive `start` and exclusive `end`.
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }
}

/// Zero-based byte positions of the fields read from a `finals2000A` line.
///
/// Positions are zero-based. A readme column `c` (one-based, as IERS writes
/// them) corresponds to index `c - 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnMap {
    /// UTC Modified Julian Date of the row.
    pub mjd: Span,
    /// Bulletin A polar motion flag (`I` observed, `P` predicted).
    pub pm_flag: usize,
    /// Bulletin A polar motion x, in arcseconds.
    pub pm_x: Span,
    /// Bulletin A polar motion y, in arcseconds.
    pub pm_y: Span,
    /// Bulletin A UT1-UTC flag (`I` observed, `P` predicted).
    pub ut1_flag: usize,
    /// Bulletin A UT1-UTC, in seconds.
    pub ut1_utc: Span,
    /// Flag for the LOD column. `None` means LOD is read whenever it is non-blank.
    pub lod_flag: Option<usize>,
    /// Bulletin A length-of-day, in milliseconds. `None` means LOD is not read.
    pub lod: Option<Span>,
}

/// Default column map for `finals2000A.data`.
///
/// Source: IERS readme for `finals2000A` (`readme.finals2000A`, retrieved
/// 2026-11-16 from <https://maia.usno.navy.mil/ser7/readme.finals2000A>).
/// One-based readme columns used: MJD 8-15, PM flag 17, PM-x 19-27, PM-y 38-46,
/// UT1 flag 58, UT1-UTC 59-68, LOD 80-86. Column 79 is blank in the readme, and
/// the readme gives no LOD flag, so `lod_flag` is `None` and LOD is read
/// whenever it is non-blank (the readme says it is "NOT ALWAYS FILLED").
pub const FINALS2000A_COLUMNS: ColumnMap = ColumnMap {
    mjd: Span::new(7, 15),
    pm_flag: 16,
    pm_x: Span::new(18, 27),
    pm_y: Span::new(37, 46),
    ut1_flag: 57,
    ut1_utc: Span::new(58, 68),
    lod_flag: None,
    lod: Some(Span::new(79, 86)),
};

/// One observed (`I`) row of Earth orientation data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EopRecord {
    /// UTC Modified Julian Date of the row.
    pub mjd: f64,
    /// Polar motion x, in arcseconds.
    pub xp_arcsec: f64,
    /// Polar motion y, in arcseconds.
    pub yp_arcsec: f64,
    /// UT1 - UTC, in seconds.
    pub dut1_seconds: f64,
    /// Length of day, in milliseconds, if present in the source row.
    pub lod_ms: Option<f64>,
}

/// Errors from EOP parsing, table construction and interpolation.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum EopError {
    /// A required field is missing, too short, or not a finite number.
    /// `line` is the one-based line number when parsed from text, or `0` when
    /// a single line was parsed with [`parse_finals2000a_line`].
    BadLine {
        /// One-based line number, or `0` if unknown.
        line: usize,
        /// Name of the field that failed.
        field: &'static str,
    },
    /// The table has no rows.
    EmptyTable,
    /// A row is not finite, or its MJD is not strictly greater than the
    /// previous row's MJD. `index` is the zero-based row index.
    InvalidRecord {
        /// Zero-based index of the offending row.
        index: usize,
    },
    /// The requested epoch lies outside the table's MJD range.
    OutOfRange {
        /// The MJD that was requested.
        mjd: f64,
    },
}

impl fmt::Display for EopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EopError::BadLine { line, field } => {
                write!(f, "EOP line {line}: field `{field}` is missing or invalid")
            }
            EopError::EmptyTable => f.write_str("EOP table has no rows"),
            EopError::InvalidRecord { index } => {
                write!(
                    f,
                    "EOP row {index} is not finite or not in increasing MJD order"
                )
            }
            EopError::OutOfRange { mjd } => {
                write!(f, "MJD {mjd} lies outside the EOP table range")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for EopError {}

fn field_f64(line: &str, span: Span, name: &'static str) -> Result<f64, EopError> {
    let bad = EopError::BadLine {
        line: 0,
        field: name,
    };
    line.get(span.start..span.end)
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .ok_or(bad)
}

fn flag_at(line: &str, index: usize, name: &'static str) -> Result<u8, EopError> {
    line.as_bytes()
        .get(index)
        .copied()
        .ok_or(EopError::BadLine {
            line: 0,
            field: name,
        })
}

/// Parses one line of `finals2000A` data.
///
/// Returns `Ok(None)` for blank lines and for rows whose polar motion or
/// UT1-UTC flag is not `I`. Returns `Ok(Some(record))` for observed rows.
///
/// # Errors
///
/// [`EopError::BadLine`] if a required field is short, blank or not a finite
/// number. The `line` field of the error is `0`.
pub fn parse_finals2000a_line(line: &str, cols: &ColumnMap) -> Result<Option<EopRecord>, EopError> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    if flag_at(line, cols.pm_flag, "pm_flag")? != b'I'
        || flag_at(line, cols.ut1_flag, "ut1_flag")? != b'I'
    {
        return Ok(None);
    }
    let lod_wanted = match cols.lod_flag {
        Some(idx) => flag_at(line, idx, "lod_flag")? == b'I',
        None => true,
    };
    let lod_ms = match cols.lod {
        Some(span) if lod_wanted => line
            .get(span.start..span.end)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|v| v.is_finite()),
        _ => None,
    };
    Ok(Some(EopRecord {
        mjd: field_f64(line, cols.mjd, "mjd")?,
        xp_arcsec: field_f64(line, cols.pm_x, "pm_x")?,
        yp_arcsec: field_f64(line, cols.pm_y, "pm_y")?,
        dut1_seconds: field_f64(line, cols.ut1_utc, "ut1_utc")?,
        lod_ms,
    }))
}

/// Parses the observed rows of a whole `finals2000A` text.
///
/// Rows that are not observed (`P` flags) and blank lines are skipped. Errors
/// report the one-based line number.
///
/// # Errors
///
/// [`EopError::BadLine`] with the line number of the first malformed observed
/// row.
#[cfg(feature = "std")]
pub fn parse_finals2000a(text: &str, cols: &ColumnMap) -> Result<Vec<EopRecord>, EopError> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        match parse_finals2000a_line(line, cols) {
            Ok(Some(rec)) => out.push(rec),
            Ok(None) => {}
            Err(EopError::BadLine { field, .. }) => {
                return Err(EopError::BadLine { line: i + 1, field });
            }
            Err(other) => return Err(other),
        }
    }
    Ok(out)
}

/// A validated, read-only table of EOP rows sorted by MJD.
///
/// The table borrows its rows, so it needs no allocation. Build the rows with
/// [`parse_finals2000a`] (with `std`) or [`parse_finals2000a_line`], store
/// them in a slice, and wrap the slice here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EopTable<'a> {
    records: &'a [EopRecord],
}

impl<'a> EopTable<'a> {
    /// Validates the rows and wraps them.
    ///
    /// # Errors
    ///
    /// [`EopError::EmptyTable`] for an empty slice, or
    /// [`EopError::InvalidRecord`] if any row has a non-finite value or an MJD
    /// that is not strictly greater than the previous row's.
    pub fn new(records: &'a [EopRecord]) -> Result<Self, EopError> {
        if records.is_empty() {
            return Err(EopError::EmptyTable);
        }
        for (i, r) in records.iter().enumerate() {
            let finite = r.mjd.is_finite()
                && r.xp_arcsec.is_finite()
                && r.yp_arcsec.is_finite()
                && r.dut1_seconds.is_finite();
            if !finite {
                return Err(EopError::InvalidRecord { index: i });
            }
            // NaN was rejected above, so a plain comparison is exact here.
            if i > 0 && r.mjd <= records[i - 1].mjd {
                return Err(EopError::InvalidRecord { index: i });
            }
        }
        Ok(EopTable { records })
    }

    /// Returns the first and last UTC MJD covered by the table.
    #[must_use]
    pub fn mjd_range(&self) -> (f64, f64) {
        (
            self.records[0].mjd,
            self.records[self.records.len() - 1].mjd,
        )
    }

    /// Linear interpolation of a per-row quantity at `mjd`.
    fn interpolate(&self, mjd: f64, value: impl Fn(&EopRecord) -> f64) -> Result<f64, EopError> {
        let rec = self.records;
        let n = rec.len();
        let (first, last) = self.mjd_range();
        if !(mjd >= first && mjd <= last) {
            return Err(EopError::OutOfRange { mjd });
        }
        if n == 1 {
            return Ok(value(&rec[0]));
        }
        let i = match rec.binary_search_by(|r| r.mjd.total_cmp(&mjd)) {
            Ok(i) => i.min(n - 2),
            Err(i) => i.saturating_sub(1).min(n - 2),
        };
        let (a, b) = (&rec[i], &rec[i + 1]);
        let t = (mjd - a.mjd) / (b.mjd - a.mjd);
        let (va, vb) = (value(a), value(b));
        Ok(va + t * (vb - va))
    }

    /// UT1 - UTC in seconds at a UTC MJD, by linear interpolation.
    ///
    /// # Errors
    ///
    /// [`EopError::OutOfRange`] if `mjd_utc` is outside the table.
    pub fn ut1_minus_utc(&self, mjd_utc: f64) -> Result<f64, EopError> {
        self.interpolate(mjd_utc, |r| r.dut1_seconds)
    }

    /// Polar motion `(xp, yp)` in arcseconds at a UTC MJD, by linear interpolation.
    ///
    /// # Errors
    ///
    /// [`EopError::OutOfRange`] if `mjd_utc` is outside the table.
    pub fn polar_motion_arcsec(&self, mjd_utc: f64) -> Result<(f64, f64), EopError> {
        let xp = self.interpolate(mjd_utc, |r| r.xp_arcsec)?;
        let yp = self.interpolate(mjd_utc, |r| r.yp_arcsec)?;
        Ok((xp, yp))
    }

    /// Converts a UTC two-part Julian date to UT1, returned as a two-part Julian date.
    ///
    /// Applies `UT1 = UTC + (UT1 - UTC)` by adding the interpolated offset to the
    /// second part, as [`crate::time`] does for TDB. The first part is unchanged.
    /// The MJD used for interpolation is `(utc_hi - 2400000.5) + utc_lo`, where
    /// `2400000.5` is [`MJD_OFFSET`].
    ///
    /// # Errors
    ///
    /// [`EopError::OutOfRange`] if the epoch lies outside the table.
    pub fn utc_to_ut1(&self, utc_hi: f64, utc_lo: f64) -> Result<(f64, f64), EopError> {
        let mjd = (utc_hi - MJD_OFFSET) + utc_lo;
        let dut1 = self.ut1_minus_utc(mjd)?;
        Ok((utc_hi, utc_lo + dut1 / SECONDS_PER_DAY))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(feature = "std"))]
    use alloc::{format, string::String, vec};

    /// Writes `text` right-justified into the span of a blank line.
    fn put(buf: &mut [u8], span: Span, text: &str) {
        let width = span.end - span.start;
        let padded = format!("{text:>width$}");
        buf[span.start..span.end].copy_from_slice(padded.as_bytes());
    }

    /// Builds one line that follows the default map. Flags go at their indices.
    fn build_line(cols: &ColumnMap, values: [&str; 5], flags: (u8, u8)) -> String {
        let [mjd, xp, yp, dut1, lod] = values;
        let mut buf = vec![b' '; 90];
        put(&mut buf, cols.mjd, mjd);
        buf[cols.pm_flag] = flags.0;
        put(&mut buf, cols.pm_x, xp);
        put(&mut buf, cols.pm_y, yp);
        buf[cols.ut1_flag] = flags.1;
        put(&mut buf, cols.ut1_utc, dut1);
        if let Some(idx) = cols.lod_flag {
            buf[idx] = b'I';
        }
        if let Some(span) = cols.lod {
            put(&mut buf, span, lod);
        }
        String::from_utf8(buf).unwrap()
    }

    fn observed(mjd: &str, xp: &str, yp: &str, dut1: &str, lod: &str) -> String {
        build_line(&FINALS2000A_COLUMNS, [mjd, xp, yp, dut1, lod], (b'I', b'I'))
    }

    #[test]
    fn parses_observed_row_at_documented_columns() {
        let line = observed("59000.00", "0.123456", "0.234567", "-0.1234567", "1.2345");
        let rec = parse_finals2000a_line(&line, &FINALS2000A_COLUMNS)
            .unwrap()
            .unwrap();
        assert_eq!(rec.mjd, 59000.0);
        assert_eq!(rec.xp_arcsec, 0.123456);
        assert_eq!(rec.yp_arcsec, 0.234567);
        assert_eq!(rec.dut1_seconds, -0.1234567);
        assert_eq!(rec.lod_ms, Some(1.2345));
    }

    #[test]
    fn skips_predicted_rows_and_blank_lines() {
        let vals = ["59001.00", "0.1", "0.2", "0.3", "0.4"];
        let pred = build_line(&FINALS2000A_COLUMNS, vals, (b'P', b'I'));
        assert_eq!(
            parse_finals2000a_line(&pred, &FINALS2000A_COLUMNS),
            Ok(None)
        );
        let ut1_pred = build_line(&FINALS2000A_COLUMNS, vals, (b'I', b'P'));
        assert_eq!(
            parse_finals2000a_line(&ut1_pred, &FINALS2000A_COLUMNS),
            Ok(None)
        );
        assert_eq!(
            parse_finals2000a_line("   ", &FINALS2000A_COLUMNS),
            Ok(None)
        );
    }

    #[test]
    fn blank_lod_is_none() {
        let line = observed("59000.00", "0.1", "0.2", "0.3", "");
        let rec = parse_finals2000a_line(&line, &FINALS2000A_COLUMNS)
            .unwrap()
            .unwrap();
        assert_eq!(rec.lod_ms, None);
    }

    #[test]
    fn parses_readme_layout_line() {
        // Hand-laid from the readme column table (1-based columns): blank
        // column 79, LOD in 80-86, no LOD flag. Built without the map.
        let line = "000101 51544.00 I  0.123456 0.000010  0.234567 0.000010  I-0.1234567 0.0000010  1.2345 0.0100";
        let rec = parse_finals2000a_line(line, &FINALS2000A_COLUMNS)
            .unwrap()
            .unwrap();
        assert_eq!(rec.mjd, 51544.0);
        assert_eq!(rec.xp_arcsec, 0.123456);
        assert_eq!(rec.yp_arcsec, 0.234567);
        assert_eq!(rec.dut1_seconds, -0.1234567);
        assert_eq!(rec.lod_ms, Some(1.2345));
    }

    #[test]
    fn short_or_bad_line_reports_field() {
        let err = parse_finals2000a_line("59000", &FINALS2000A_COLUMNS).unwrap_err();
        assert!(matches!(
            err,
            EopError::BadLine {
                field: "pm_flag",
                ..
            }
        ));
        let mut bad = observed("59000.00", "0.1", "0.2", "0.3", "");
        bad.replace_range(58..68, "   abc    ");
        let err = parse_finals2000a_line(&bad, &FINALS2000A_COLUMNS).unwrap_err();
        assert!(matches!(
            err,
            EopError::BadLine {
                field: "ut1_utc",
                ..
            }
        ));
    }

    #[cfg(feature = "std")]
    #[test]
    fn text_parser_skips_predicted_and_numbers_bad_lines() {
        let good = observed("59000.00", "0.1", "0.2", "0.3", "0.4");
        let pred = build_line(
            &FINALS2000A_COLUMNS,
            ["59001.00", "0.1", "0.2", "0.3", "0.4"],
            (b'P', b'P'),
        );
        let text = format!("{good}\n{pred}\n\n{good}\n");
        let recs = parse_finals2000a(&text, &FINALS2000A_COLUMNS).unwrap();
        assert_eq!(recs.len(), 2);

        let text = format!("{good}\n59000\n");
        let err = parse_finals2000a(&text, &FINALS2000A_COLUMNS).unwrap_err();
        assert!(matches!(err, EopError::BadLine { line: 2, .. }));
    }

    #[test]
    fn custom_column_map_is_honoured() {
        // A map with the fields moved right by 3 bytes and no LOD.
        let shifted = ColumnMap {
            mjd: Span::new(10, 18),
            pm_flag: 19,
            pm_x: Span::new(21, 30),
            pm_y: Span::new(40, 49),
            ut1_flag: 60,
            ut1_utc: Span::new(61, 71),
            lod_flag: None,
            lod: None,
        };
        let line = build_line(
            &shifted,
            ["59100.50", "0.5", "-0.25", "0.05", ""],
            (b'I', b'I'),
        );
        let rec = parse_finals2000a_line(&line, &shifted).unwrap().unwrap();
        assert_eq!(rec.mjd, 59100.5);
        assert_eq!(rec.yp_arcsec, -0.25);
        assert_eq!(rec.lod_ms, None);
    }

    fn two_row_table() -> [EopRecord; 2] {
        [
            EopRecord {
                mjd: 59000.0,
                xp_arcsec: 0.10,
                yp_arcsec: 0.30,
                dut1_seconds: 0.2,
                lod_ms: None,
            },
            EopRecord {
                mjd: 59002.0,
                xp_arcsec: 0.20,
                yp_arcsec: 0.10,
                dut1_seconds: 0.6,
                lod_ms: None,
            },
        ]
    }

    #[test]
    fn ut1_interpolates_linearly_between_rows() {
        let rows = two_row_table();
        let table = EopTable::new(&rows).unwrap();
        assert!((table.ut1_minus_utc(59000.0).unwrap() - 0.2).abs() < 1e-15);
        assert!((table.ut1_minus_utc(59001.0).unwrap() - 0.4).abs() < 1e-15);
        assert!((table.ut1_minus_utc(59002.0).unwrap() - 0.6).abs() < 1e-15);
        let (xp, yp) = table.polar_motion_arcsec(59001.5).unwrap();
        assert!((xp - 0.175).abs() < 1e-15);
        // yp goes 0.30 -> 0.10; at 0.75 of the interval that is 0.15.
        assert!((yp - 0.15).abs() < 1e-15);
    }

    #[test]
    fn ut1_outside_table_is_an_error() {
        let rows = two_row_table();
        let table = EopTable::new(&rows).unwrap();
        assert_eq!(
            table.ut1_minus_utc(58999.999),
            Err(EopError::OutOfRange { mjd: 58999.999 })
        );
        assert!(table.ut1_minus_utc(59002.001).is_err());
        assert!(table.polar_motion_arcsec(59010.0).is_err());
    }

    #[test]
    fn utc_to_ut1_adds_offset_to_second_part() {
        let rows = two_row_table();
        let table = EopTable::new(&rows).unwrap();
        // UTC JD for MJD 59001.0 is 2459001.5 (hi) with lo = 0.
        let (hi, lo) = table.utc_to_ut1(2_459_001.5, 0.0).unwrap();
        assert_eq!(hi, 2_459_001.5);
        // Offset 0.4 s = 0.4 / 86400 days.
        assert!((lo - 0.4 / SECONDS_PER_DAY).abs() < 1e-15);
        assert!(table.utc_to_ut1(2_458_000.5, 0.0).is_err());
    }

    #[test]
    fn table_rejects_empty_and_unsorted() {
        assert_eq!(EopTable::new(&[]), Err(EopError::EmptyTable));
        let mut rows = two_row_table();
        rows.swap(0, 1);
        assert_eq!(
            EopTable::new(&rows),
            Err(EopError::InvalidRecord { index: 1 })
        );
        let mut rows = two_row_table();
        rows[1].dut1_seconds = f64::NAN;
        assert_eq!(
            EopTable::new(&rows),
            Err(EopError::InvalidRecord { index: 1 })
        );
    }

    #[test]
    fn single_row_table_answers_only_its_epoch() {
        let rows = [two_row_table()[0]];
        let table = EopTable::new(&rows).unwrap();
        assert_eq!(table.ut1_minus_utc(59000.0).unwrap(), 0.2);
        assert!(table.ut1_minus_utc(59000.5).is_err());
    }

    #[test]
    fn arcsec_constant_is_consistent() {
        assert!((ARCSEC_TO_RAD * 206_264.806_247_096_36 - 1.0).abs() < 1e-14);
    }
}
