//! Tests for the NetCDF-3 reader, writer and CF helpers.
//!
//! The byte-level tests use layouts computed by hand from the NetCDF-3 file
//! format specification, so they check the writer and reader against the
//! format rather than against each other.

use alloc::vec;
use alloc::vec::Vec;

use super::cf::{Calendar, CfDateTime, TimeUnits};
use super::{Attribute, Format, NcType, NcValues, Reader, Writer};

/// One fixed dimension `x = 2` and one `NC_INT` variable `v = [1, 2]`, CDF-1.
///
/// Layout: magic and version (8 bytes including numrecs), dim_list (20), gatt_list
/// ABSENT (8), var_list header (8), v name (8), ndims (4), dimid (4), vatt_list
/// ABSENT (8), nc_type (4), vsize (4), begin (4) = 80-byte header, then the two
/// big-endian ints.
const TINY_CDF1: [u8; 88] = [
    // "CDF" 0x01, numrecs = 0
    0x43, 0x44, 0x46, 0x01, 0, 0, 0, 0,
    // dim_list: NC_DIMENSION, nelems = 1, name "x" (len 1, padded), length 2
    0, 0, 0, 0x0A, 0, 0, 0, 1, 0, 0, 0, 1, 0x78, 0, 0, 0, 0, 0, 0, 2, // gatt_list: ABSENT
    0, 0, 0, 0, 0, 0, 0, 0, // var_list: NC_VARIABLE, nelems = 1
    0, 0, 0, 0x0B, 0, 0, 0, 1, // name "v", ndims = 1, dimid = 0
    0, 0, 0, 1, 0x76, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, // vatt_list: ABSENT
    0, 0, 0, 0, 0, 0, 0, 0, // nc_type = NC_INT, vsize = 8, begin = 80
    0, 0, 0, 4, 0, 0, 0, 8, 0, 0, 0, 0x50, // data: 1, 2
    0, 0, 0, 1, 0, 0, 0, 2,
];

/// The same file as CDF-2: the begin offset is 8 bytes and the data starts at 84.
const TINY_CDF2: [u8; 92] = [
    0x43, 0x44, 0x46, 0x02, 0, 0, 0, 0, 0, 0, 0, 0x0A, 0, 0, 0, 1, 0, 0, 0, 1, 0x78, 0, 0, 0, 0, 0,
    0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0B, 0, 0, 0, 1, 0, 0, 0, 1, 0x76, 0, 0, 0, 0, 0, 0, 1,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 8,
    // begin = 84 as a 64-bit offset
    0, 0, 0, 0, 0, 0, 0, 0x54, 0, 0, 0, 1, 0, 0, 0, 2,
];

/// One unlimited dimension `t` (numrecs = 3) and one record variable `s` of
/// `NC_SHORT`, CDF-1. Being the only record variable of a 2-byte type, its
/// record is stored unpadded (vsize = 2), so the data is 6 bytes long.
const RECORD_SHORT_CDF1: [u8; 86] = [
    0x43, 0x44, 0x46, 0x01, 0, 0, 0, 3, // dim "t", length 0 (unlimited)
    0, 0, 0, 0x0A, 0, 0, 0, 1, 0, 0, 0, 1, 0x74, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    // var "s", ndims = 1, dimid = 0
    0, 0, 0, 0x0B, 0, 0, 0, 1, 0, 0, 0, 1, 0x73, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, // nc_type = NC_SHORT, vsize = 2, begin = 80
    0, 0, 0, 3, 0, 0, 0, 2, 0, 0, 0, 0x50, // records: 10, -2, 300
    0, 0x0A, 0xFF, 0xFE, 0x01, 0x2C,
];

fn all_types_file(format: Format) -> Vec<u8> {
    let mut w = Writer::new(format);
    w.add_attribute(Attribute::text("title", "synthetic"));
    w.add_attribute(Attribute::new("scale", NcValues::Double(vec![0.5, 2.0])));
    w.add_attribute(Attribute::new("count", NcValues::Int(vec![7])));
    let x = w.add_dimension("x", Some(3)).unwrap();
    let t = w.add_dimension("time", None).unwrap();
    w.add_variable(
        "b",
        &[x],
        vec![Attribute::new("valid", NcValues::Byte(vec![-1, 1]))],
        NcValues::Byte(vec![-128, 0, 127]),
    )
    .unwrap();
    w.add_variable(
        "c",
        &[x],
        vec![Attribute::text("long_name", "chars")],
        NcValues::text("abc"),
    )
    .unwrap();
    w.add_variable("s", &[x], vec![], NcValues::Short(vec![-300, 0, 300]))
        .unwrap();
    w.add_variable(
        "i",
        &[x],
        vec![],
        NcValues::Int(vec![i32::MIN, 0, i32::MAX]),
    )
    .unwrap();
    w.add_variable("f", &[x], vec![], NcValues::Float(vec![1.5, -0.25, 1e10]))
        .unwrap();
    w.add_variable(
        "d",
        &[x],
        vec![Attribute::new(
            "scale_factor",
            NcValues::Double(vec![0.001]),
        )],
        NcValues::Double(vec![core::f64::consts::PI, -1e-300, 1e300]),
    )
    .unwrap();
    w.add_variable(
        "rb",
        &[t, x],
        vec![],
        NcValues::Byte(vec![1, 2, 3, 4, 5, 6]),
    )
    .unwrap();
    w.add_variable("rd", &[t], vec![], NcValues::Double(vec![0.1, 0.2]))
        .unwrap();
    w.add_variable("ri", &[t], vec![], NcValues::Int(vec![-5, 6]))
        .unwrap();
    w.finish().unwrap()
}

#[test]
fn writer_produces_hand_computed_cdf1_bytes() {
    let mut w = Writer::new(Format::Classic);
    let x = w.add_dimension("x", Some(2)).unwrap();
    w.add_variable("v", &[x], vec![], NcValues::Int(vec![1, 2]))
        .unwrap();
    assert_eq!(w.finish().unwrap(), TINY_CDF1.to_vec());
}

#[test]
fn writer_produces_hand_computed_cdf2_bytes() {
    let mut w = Writer::new(Format::Offset64);
    let x = w.add_dimension("x", Some(2)).unwrap();
    w.add_variable("v", &[x], vec![], NcValues::Int(vec![1, 2]))
        .unwrap();
    assert_eq!(w.finish().unwrap(), TINY_CDF2.to_vec());
}

#[test]
fn writer_produces_hand_computed_record_bytes() {
    let mut w = Writer::new(Format::Classic);
    let t = w.add_dimension("t", None).unwrap();
    w.add_variable("s", &[t], vec![], NcValues::Short(vec![10, -2, 300]))
        .unwrap();
    assert_eq!(w.finish().unwrap(), RECORD_SHORT_CDF1.to_vec());
}

#[test]
fn reader_parses_hand_computed_cdf1_bytes() {
    let reader = Reader::new(&TINY_CDF1).unwrap();
    assert_eq!(reader.format(), Format::Classic);
    assert_eq!(reader.numrecs(), 0);
    assert_eq!(reader.dimensions().len(), 1);
    let x = &reader.dimensions()[0];
    assert_eq!((x.name.as_str(), x.len, x.unlimited), ("x", 2, false));
    assert!(reader.attributes().is_empty());
    let v = reader.variable("v").unwrap();
    assert_eq!(v.dim_ids, vec![0]);
    assert_eq!(v.nc_type, NcType::Int);
    assert_eq!(v.vsize, 8);
    assert_eq!(v.begin, 80);
    assert!(!v.is_record);
    assert_eq!(reader.read_i32("v").unwrap(), vec![1, 2]);
    assert_eq!(reader.read_f64("v").unwrap(), vec![1.0, 2.0]);
}

#[test]
fn reader_parses_hand_computed_cdf2_bytes() {
    let reader = Reader::new(&TINY_CDF2).unwrap();
    assert_eq!(reader.format(), Format::Offset64);
    assert_eq!(reader.variable("v").unwrap().begin, 84);
    assert_eq!(reader.read_i32("v").unwrap(), vec![1, 2]);
}

#[test]
fn reader_parses_hand_computed_record_bytes() {
    let reader = Reader::new(&RECORD_SHORT_CDF1).unwrap();
    assert_eq!(reader.numrecs(), 3);
    let t = reader.dimension("t").unwrap();
    assert!(t.unlimited);
    assert_eq!(t.len, 3);
    let s = reader.variable("s").unwrap();
    assert!(s.is_record);
    assert_eq!(s.vsize, 2);
    assert_eq!(
        reader.read_values("s").unwrap(),
        NcValues::Short(vec![10, -2, 300])
    );
    assert_eq!(reader.shape("s").unwrap(), vec![3]);
}

#[test]
fn round_trip_all_types_both_formats() {
    for format in [Format::Classic, Format::Offset64] {
        let bytes = all_types_file(format);
        let reader = Reader::new(&bytes).unwrap();
        assert_eq!(reader.format(), format);
        assert_eq!(reader.numrecs(), 2);
        assert_eq!(
            reader.read_values("b").unwrap(),
            NcValues::Byte(vec![-128, 0, 127])
        );
        assert_eq!(reader.read_values("c").unwrap().as_text(), Some("abc"));
        assert_eq!(
            reader.read_values("s").unwrap(),
            NcValues::Short(vec![-300, 0, 300])
        );
        assert_eq!(reader.read_i32("i").unwrap(), vec![i32::MIN, 0, i32::MAX]);
        assert_eq!(reader.read_f32("f").unwrap(), vec![1.5, -0.25, 1e10]);
        assert_eq!(
            reader.read_values("d").unwrap(),
            NcValues::Double(vec![core::f64::consts::PI, -1e-300, 1e300])
        );
        assert_eq!(
            reader.read_values("rb").unwrap(),
            NcValues::Byte(vec![1, 2, 3, 4, 5, 6])
        );
        assert_eq!(reader.read_f64("rd").unwrap(), vec![0.1, 0.2]);
        assert_eq!(reader.read_i32("ri").unwrap(), vec![-5, 6]);
        assert_eq!(reader.shape("rb").unwrap(), vec![2, 3]);
    }
}

#[test]
fn round_trip_attributes() {
    let bytes = all_types_file(Format::Classic);
    let reader = Reader::new(&bytes).unwrap();
    assert_eq!(
        reader.attribute("title").unwrap().values.as_text(),
        Some("synthetic")
    );
    assert_eq!(
        reader.attribute("scale").unwrap().values,
        NcValues::Double(vec![0.5, 2.0])
    );
    assert_eq!(
        reader.attribute("count").unwrap().values,
        NcValues::Int(vec![7])
    );
    let b = reader.variable("b").unwrap();
    assert_eq!(
        b.attribute("valid").unwrap().values,
        NcValues::Byte(vec![-1, 1])
    );
    let c = reader.variable("c").unwrap();
    assert_eq!(
        c.attribute("long_name").unwrap().values.as_text(),
        Some("chars")
    );
    let d = reader.variable("d").unwrap();
    assert_eq!(
        d.attribute("scale_factor").unwrap().values,
        NcValues::Double(vec![0.001])
    );
}

#[test]
fn cdf1_and_cdf2_differ_only_in_offsets() {
    let classic = all_types_file(Format::Classic);
    let offset64 = all_types_file(Format::Offset64);
    // Each variable's begin is 4 bytes wider in CDF-2 and the data follows.
    let nvars = Reader::new(&classic).unwrap().variables().len();
    assert_eq!(offset64.len(), classic.len() + 4 * nvars);
    assert_eq!(&classic[..3], b"CDF");
    assert_eq!(classic[3], 1);
    assert_eq!(offset64[3], 2);
    let a = Reader::new(&classic).unwrap();
    let b = Reader::new(&offset64).unwrap();
    for var in a.variables() {
        assert_eq!(
            a.read_values(&var.name).unwrap(),
            b.read_values(&var.name).unwrap()
        );
    }
}

#[test]
fn multiple_record_variables_interleave_per_record() {
    let mut w = Writer::new(Format::Classic);
    let t = w.add_dimension("t", None).unwrap();
    let y = w.add_dimension("y", Some(2)).unwrap();
    w.add_variable("a", &[t, y], vec![], NcValues::Int(vec![1, 2, 3, 4, 5, 6]))
        .unwrap();
    w.add_variable("fixed", &[y], vec![], NcValues::Double(vec![9.0, 8.0]))
        .unwrap();
    w.add_variable("b", &[t], vec![], NcValues::Short(vec![7, 8, 9]))
        .unwrap();
    let bytes = w.finish().unwrap();
    let reader = Reader::new(&bytes).unwrap();
    assert_eq!(reader.read_i32("a").unwrap(), vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(reader.read_f64("fixed").unwrap(), vec![9.0, 8.0]);
    assert_eq!(reader.read_f64("b").unwrap(), vec![7.0, 8.0, 9.0]);
    // a: 8 bytes per record, b: 2 bytes padded to 4, so the record stride is 12.
    assert_eq!(reader.variable("a").unwrap().vsize, 8);
    assert_eq!(reader.variable("b").unwrap().vsize, 4);
}

#[test]
fn zero_records_read_as_empty() {
    let mut w = Writer::new(Format::Offset64);
    let t = w.add_dimension("t", None).unwrap();
    w.add_variable("r", &[t], vec![], NcValues::Double(vec![]))
        .unwrap();
    let bytes = w.finish().unwrap();
    let reader = Reader::new(&bytes).unwrap();
    assert_eq!(reader.numrecs(), 0);
    assert!(reader.read_values("r").unwrap().is_empty());
}

#[test]
fn writer_converts_from_f64() {
    let mut w = Writer::new(Format::Classic);
    let x = w.add_dimension("x", Some(4)).unwrap();
    w.add_variable_f64("i", &[x], vec![], NcType::Int, &[1.9, -2.9, 0.0, 3.0])
        .unwrap();
    w.add_variable_f64("b", &[x], vec![], NcType::Byte, &[-128.0, 127.0, 0.5, -0.5])
        .unwrap();
    let bytes = w.finish().unwrap();
    let reader = Reader::new(&bytes).unwrap();
    assert_eq!(reader.read_i32("i").unwrap(), vec![1, -2, 0, 3]);
    assert_eq!(reader.read_f64("b").unwrap(), vec![-128.0, 127.0, 0.0, 0.0]);
}

#[test]
fn writer_rejects_invalid_input() {
    let mut w = Writer::new(Format::Classic);
    assert!(w.add_dimension("", Some(1)).is_err());
    assert!(w.add_dimension("a/b", Some(1)).is_err());
    assert!(w.add_dimension("zero", Some(0)).is_err());
    let x = w.add_dimension("x", Some(2)).unwrap();
    assert!(w.add_dimension("x", Some(3)).is_err());
    w.add_dimension("t", None).unwrap();
    assert!(w.add_dimension("t2", None).is_err());
    // Size mismatch for a fixed variable.
    assert!(
        w.add_variable("v", &[x], vec![], NcValues::Int(vec![1]))
            .is_err()
    );
    // Unlimited dimension not first.
    let t = 1;
    assert!(
        w.add_variable("bad", &[x, t], vec![], NcValues::Int(vec![1, 2]))
            .is_err()
    );
    // Out-of-range value for NC_BYTE.
    assert!(
        w.add_variable_f64("b", &[x], vec![], NcType::Byte, &[1.0, 200.0])
            .is_err()
    );
    // NaN cannot be an integer.
    assert!(NcValues::from_f64(NcType::Int, &[f64::NAN]).is_err());
    // Undefined dimension id.
    assert!(
        w.add_variable("u", &[9], vec![], NcValues::Int(vec![]))
            .is_err()
    );
    // Duplicate variable.
    w.add_variable("v", &[x], vec![], NcValues::Int(vec![1, 2]))
        .unwrap();
    assert!(
        w.add_variable("v", &[x], vec![], NcValues::Int(vec![1, 2]))
            .is_err()
    );
    // Record variables must agree on the record count.
    let t = 1;
    w.add_variable("r1", &[t], vec![], NcValues::Int(vec![1, 2]))
        .unwrap();
    w.add_variable("r2", &[t], vec![], NcValues::Int(vec![1, 2, 3]))
        .unwrap();
    assert!(w.finish().is_err());
}

#[test]
fn writer_rejects_duplicate_attribute_names() {
    let mut w = Writer::new(Format::Classic);
    w.add_attribute(Attribute::text("a", "one"));
    w.add_attribute(Attribute::text("a", "two"));
    assert!(w.finish().is_err());
}

#[test]
fn reader_rejects_bad_headers() {
    // Not a NetCDF-3 file.
    assert!(Reader::new(b"\x89HDF\r\n\x1a\n").is_err());
    assert!(Reader::new(b"").is_err());
    // Unknown version and CDF-5.
    let mut bad = TINY_CDF1;
    bad[3] = 9;
    assert!(Reader::new(&bad).is_err());
    bad[3] = 5;
    assert!(Reader::new(&bad).is_err());
    // Streaming numrecs.
    let mut streaming = TINY_CDF1;
    streaming[4..8].copy_from_slice(&[0xFF; 4]);
    assert!(Reader::new(&streaming).is_err());
    // Undefined dimension id in the variable (dimid is at bytes 56..60).
    let mut bad_dim = TINY_CDF1;
    bad_dim[56..60].copy_from_slice(&[0, 0, 0, 5]);
    assert!(Reader::new(&bad_dim).is_err());
    // Wrong list tag for the dimension list (tag is at bytes 8..12).
    let mut bad_tag = TINY_CDF1;
    bad_tag[11] = 0x0B;
    assert!(Reader::new(&bad_tag).is_err());
    // Unknown NC type code (nc_type is at bytes 68..72).
    let mut bad_type = TINY_CDF1;
    bad_type[68..72].copy_from_slice(&[0, 0, 0, 77]);
    assert!(Reader::new(&bad_type).is_err());
    // vsize too small for the declared element count (vsize is at bytes 72..76).
    let mut small_vsize = TINY_CDF1;
    small_vsize[72..76].copy_from_slice(&[0, 0, 0, 4]);
    assert!(Reader::new(&small_vsize).is_err());
    // Data offset inside the header.
    let mut inside = TINY_CDF1;
    inside[79] = 0x10;
    assert!(Reader::new(&inside).is_err());
}

#[test]
fn reader_fails_cleanly_for_data_beyond_end() {
    let truncated = &TINY_CDF1[..TINY_CDF1.len() - 1];
    let reader = Reader::new(truncated).unwrap();
    assert!(reader.read_values("v").is_err());
    assert!(reader.read_values("missing").is_err());
}

#[test]
fn truncated_inputs_never_panic() {
    let full = all_types_file(Format::Classic);
    for len in 0..full.len() {
        exercise(&full[..len]);
    }
    let full2 = all_types_file(Format::Offset64);
    for len in 0..full2.len() {
        exercise(&full2[..len]);
    }
    for len in 0..TINY_CDF1.len() {
        exercise(&TINY_CDF1[..len]);
    }
}

#[test]
fn mutated_inputs_never_panic() {
    let original = all_types_file(Format::Offset64);
    let values = [0u8, 1, 2, 5, 0x0A, 0x0B, 0x0C, 0x7F, 0x80, 0xFF];
    for index in 0..original.len() {
        for &value in &values {
            let mut bytes = original.clone();
            bytes[index] = value;
            exercise(&bytes);
        }
    }
}

#[test]
fn pseudo_random_garbage_never_panics() {
    // Linear congruential generator, so the test needs no RNG dependency.
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as u8
    };
    for round in 0..2000 {
        let len = (round % 200) as usize;
        let mut bytes: Vec<u8> = (0..len).map(|_| next()).collect();
        if round % 2 == 0 && bytes.len() >= 4 {
            bytes[..3].copy_from_slice(b"CDF");
            bytes[3] = if round % 4 == 0 { 1 } else { 2 };
        }
        exercise(&bytes);
    }
}

/// Parses `bytes` and reads every variable, discarding the results. Errors
/// are expected for malformed input; the test only checks that nothing panics.
fn exercise(bytes: &[u8]) {
    if let Ok(reader) = Reader::new(bytes) {
        let names: Vec<_> = reader.variables().iter().map(|v| v.name.clone()).collect();
        for name in names {
            let _ = reader.read_values(&name);
            let _ = reader.read_f64(&name);
            let _ = reader.shape(&name);
        }
    }
}

fn dt(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: f64) -> CfDateTime {
    CfDateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
    }
}

#[test]
fn cf_days_since_example() {
    let units = TimeUnits::parse("days since 2000-01-01", Calendar::Standard).unwrap();
    assert_eq!(units.decode(1.0).unwrap(), dt(2000, 1, 2, 0, 0, 0.0));
    assert_eq!(units.decode(0.0).unwrap(), dt(2000, 1, 1, 0, 0, 0.0));
    assert_eq!(units.decode(-1.0).unwrap(), dt(1999, 12, 31, 0, 0, 0.0));
    // 2000 is a leap year.
    assert_eq!(units.decode(59.0).unwrap(), dt(2000, 2, 29, 0, 0, 0.0));
    assert_eq!(units.decode(60.0).unwrap(), dt(2000, 3, 1, 0, 0, 0.0));
    assert_eq!(units.decode(0.5).unwrap(), dt(2000, 1, 1, 12, 0, 0.0));
}

#[test]
fn cf_other_units_and_reference_times() {
    let hours = TimeUnits::parse("hours since 1900-01-01 00:00:00", Calendar::Standard).unwrap();
    assert_eq!(hours.decode(36.0).unwrap(), dt(1900, 1, 2, 12, 0, 0.0));
    let minutes = TimeUnits::parse("minutes since 2000-01-01 00:00", Calendar::Standard).unwrap();
    assert_eq!(minutes.decode(90.0).unwrap(), dt(2000, 1, 1, 1, 30, 0.0));
    let seconds =
        TimeUnits::parse("seconds since 1970-01-01T00:00:00", Calendar::Standard).unwrap();
    assert_eq!(
        seconds.decode(86_399.5).unwrap(),
        dt(1970, 1, 1, 23, 59, 59.5)
    );
    let fractional = TimeUnits::parse("days since 2000-02-28", Calendar::Standard).unwrap();
    assert_eq!(fractional.decode(1.0).unwrap(), dt(2000, 2, 29, 0, 0, 0.0));
    assert_eq!(fractional.decode(2.0).unwrap(), dt(2000, 3, 1, 0, 0, 0.0));
}

#[test]
fn cf_gregorian_reform_in_standard_calendar() {
    // Julian 1582-10-04 is followed by Gregorian 1582-10-15.
    let standard = TimeUnits::parse("days since 1582-10-04", Calendar::Standard).unwrap();
    assert_eq!(standard.decode(1.0).unwrap(), dt(1582, 10, 15, 0, 0, 0.0));
    let proleptic =
        TimeUnits::parse("days since 1582-10-04", Calendar::ProlepticGregorian).unwrap();
    assert_eq!(proleptic.decode(1.0).unwrap(), dt(1582, 10, 5, 0, 0, 0.0));
    // 1900 is not a leap year in the Gregorian calendar, but is in the Julian
    // one; 1900-02-28 + 1 day is 1900-03-01 in both used here.
    let p1900 = TimeUnits::parse("days since 1900-02-28", Calendar::ProlepticGregorian).unwrap();
    assert_eq!(p1900.decode(1.0).unwrap(), dt(1900, 3, 1, 0, 0, 0.0));
    // Standard calendar before the reform uses Julian leap years (1500 is leap there).
    let julian_leap = TimeUnits::parse("days since 1500-02-28", Calendar::Standard).unwrap();
    assert_eq!(julian_leap.decode(1.0).unwrap(), dt(1500, 2, 29, 0, 0, 0.0));
}

#[test]
fn cf_noleap_calendar() {
    let units = TimeUnits::parse("days since 2001-01-01", Calendar::NoLeap).unwrap();
    assert_eq!(units.decode(365.0).unwrap(), dt(2002, 1, 1, 0, 0, 0.0));
    assert_eq!(units.decode(59.0).unwrap(), dt(2001, 3, 1, 0, 0, 0.0));
    let feb = TimeUnits::parse("days since 2000-02-28", Calendar::NoLeap).unwrap();
    assert_eq!(feb.decode(1.0).unwrap(), dt(2000, 3, 1, 0, 0, 0.0));
    assert_eq!(Calendar::from_name("365_day").unwrap(), Calendar::NoLeap);
}

#[test]
fn cf_encode_inverts_decode() {
    let units = TimeUnits::parse("hours since 1990-06-15 06:30:00", Calendar::Standard).unwrap();
    for offset in [0.0, 1.0, 1234.5, -77.25, 100_000.0] {
        let time = units.decode(offset).unwrap();
        let back = units.encode(time).unwrap();
        assert!((back - offset).abs() < 1e-9, "offset {offset} -> {back}");
    }
    let days = TimeUnits::parse("days since 2000-01-01", Calendar::Standard).unwrap();
    assert_eq!(days.encode(dt(2000, 2, 29, 0, 0, 0.0)).unwrap(), 59.0);
}

#[test]
fn cf_rejects_unsupported_and_invalid_units() {
    let standard = Calendar::Standard;
    assert!(TimeUnits::parse("months since 2000-01-01", standard).is_err());
    assert!(TimeUnits::parse("years since 2000-01-01", standard).is_err());
    assert!(TimeUnits::parse("days since 2000-02-30", standard).is_err());
    assert!(TimeUnits::parse("days since 2000-13-01", standard).is_err());
    assert!(TimeUnits::parse("days from 2000-01-01", standard).is_err());
    assert!(TimeUnits::parse("days since", standard).is_err());
    assert!(TimeUnits::parse("", standard).is_err());
    assert!(TimeUnits::parse("days since 2000-01-01 25:00", standard).is_err());
    assert!(TimeUnits::parse("days since 2000-01-01 00:00Z", standard).is_err());
    assert!(TimeUnits::parse("days since 1582-10-10", standard).is_err());
    assert!(Calendar::from_name("360_day").is_err());
    assert!(Calendar::from_name("all_leap").is_err());
    assert!(Calendar::from_name("julian").is_err());
}

#[test]
fn cf_time_variable_round_trip_through_file() {
    let mut w = Writer::new(Format::Classic);
    let t = w.add_dimension("time", None).unwrap();
    w.add_variable(
        "time",
        &[t],
        vec![
            Attribute::text("units", "days since 2000-01-01"),
            Attribute::text("calendar", "proleptic_gregorian"),
        ],
        NcValues::Double(vec![0.0, 1.0, 2.5]),
    )
    .unwrap();
    let bytes = w.finish().unwrap();
    let reader = Reader::new(&bytes).unwrap();
    let times = super::cf::decode_times(&reader, "time").unwrap();
    assert_eq!(times[0], dt(2000, 1, 1, 0, 0, 0.0));
    assert_eq!(times[1], dt(2000, 1, 2, 0, 0, 0.0));
    assert_eq!(times[2], dt(2000, 1, 3, 12, 0, 0.0));
    let units = super::cf::time_units(&reader, "time").unwrap();
    assert_eq!(units.calendar(), Calendar::ProlepticGregorian);
    assert_eq!(units.unit(), super::cf::TimeUnit::Days);
}

/// Reference file written by `scipy.io.netcdf_file` (version 1, CDF-1). The
/// values match the generating script: time unlimited with 2 records, x = 3.
const SCIPY_CDF1: &[u8] = include_bytes!("testdata/scipy_cdf1.nc");
/// The same content written by scipy as CDF-2.
const SCIPY_CDF2: &[u8] = include_bytes!("testdata/scipy_cdf2.nc");
/// Scipy file with a single record variable of `NC_SHORT` (unpadded records).
const SCIPY_SOLE_SHORT: &[u8] = include_bytes!("testdata/scipy_sole_short_cdf1.nc");

fn check_scipy_reference(bytes: &[u8], format: Format) {
    let reader = Reader::new(bytes).unwrap();
    assert_eq!(reader.format(), format);
    assert_eq!(reader.numrecs(), 2);
    assert_eq!(reader.dimension("time").unwrap().len, 2);
    assert!(reader.dimension("time").unwrap().unlimited);
    assert_eq!(reader.dimension("x").unwrap().len, 3);
    assert_eq!(
        reader.attribute("title").unwrap().values.as_text(),
        Some("scipy reference")
    );
    assert_eq!(
        reader.attribute("scale").unwrap().values,
        NcValues::Double(vec![0.5, 2.0])
    );
    assert_eq!(
        reader.read_values("b").unwrap(),
        NcValues::Byte(vec![-128, 0, 127])
    );
    assert_eq!(
        reader.read_values("h").unwrap(),
        NcValues::Short(vec![-300, 0, 300])
    );
    assert_eq!(reader.read_i32("i").unwrap(), vec![i32::MIN, 0, i32::MAX]);
    assert_eq!(reader.read_f32("f").unwrap(), vec![1.5, -0.25, 1e10]);
    let d = reader.variable("d").unwrap();
    assert_eq!(
        d.attribute("units").unwrap().values.as_text(),
        Some("m s-1")
    );
    assert_eq!(
        reader.read_f64("d").unwrap(),
        vec![core::f64::consts::PI, -1e-300, 1e300]
    );
    assert_eq!(reader.read_values("c").unwrap().as_text(), Some("abc"));
    assert_eq!(reader.read_f64("rd").unwrap(), vec![0.1, 0.2]);
    assert_eq!(reader.read_i32("ri").unwrap(), vec![-5, 6]);
    assert!(reader.variable("rd").unwrap().is_record);
    assert!(!reader.variable("b").unwrap().is_record);
}

#[test]
fn reads_scipy_cdf1_reference() {
    check_scipy_reference(SCIPY_CDF1, Format::Classic);
}

#[test]
fn reads_scipy_cdf2_reference() {
    check_scipy_reference(SCIPY_CDF2, Format::Offset64);
}

#[test]
fn reads_scipy_sole_record_short_unpadded() {
    let reader = Reader::new(SCIPY_SOLE_SHORT).unwrap();
    assert_eq!(reader.numrecs(), 3);
    assert_eq!(reader.variable("s").unwrap().vsize, 2);
    assert_eq!(
        reader.read_values("s").unwrap(),
        NcValues::Short(vec![10, -2, 300])
    );
}
