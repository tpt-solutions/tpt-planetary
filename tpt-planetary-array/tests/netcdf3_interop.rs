//! Reads NetCDF-3 files written by the netCDF C library (through netCDF4-python,
//! see `tests/fixtures/gen_netcdf3_fixtures.py`) and checks every value and the
//! CF time decoding against the JSON that the generator recorded.

use std::fs;
use std::path::Path;

use serde_json::Value;
use tpt_planetary_array::netcdf3::{Format, Reader, cf};

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/netcdf3")
        .join(name);
    fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

fn expected() -> Value {
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/netcdf3/netcdf4lib_values.json"),
    )
    .expect("values file");
    serde_json::from_str(&text).expect("valid JSON")
}

fn as_f64s(v: &Value) -> Vec<f64> {
    v.as_array()
        .expect("array")
        .iter()
        .map(|x| x.as_f64().expect("number"))
        .collect()
}

fn check_file(name: &str, format: Format) {
    let bytes = fixture(name);
    let reader = Reader::new(&bytes).expect("parse");
    assert_eq!(reader.format(), format);
    assert_eq!(
        reader.numrecs(),
        4,
        "record count from the unlimited dimension"
    );
    assert_eq!(
        reader.attribute("title").and_then(|a| a.values.as_text()),
        Some("interop fixture")
    );

    let exp = expected();
    let values = &exp["values"];
    // Floating-point and integer variables, including a record variable with
    // a non-trivial shape.
    for name in ["time", "lat", "lon", "temp", "code", "flag"] {
        let want: Vec<f64> = as_f64s(&values[name]);
        let got = reader
            .read_f64(name)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(got, want, "variable {name} in {name}");
    }
    assert_eq!(reader.shape("temp").unwrap(), vec![4, 3, 4]);
    assert_eq!(reader.shape("code").unwrap(), vec![3, 4]);
}

#[test]
fn classic_cdf1_file_matches_netcdf_library() {
    check_file("netcdf4lib_classic.nc", Format::Classic);
}

#[test]
fn offset64_cdf2_file_matches_netcdf_library() {
    check_file("netcdf4lib_64bit.nc", Format::Offset64);
}

#[test]
fn cf_time_units_decode_as_the_library_defines_them() {
    let bytes = fixture("netcdf4lib_classic.nc");
    let reader = Reader::new(&bytes).unwrap();
    let units = cf::time_units(&reader, "time").unwrap();
    assert_eq!(units.calendar(), cf::Calendar::NoLeap);
    // 59.5 days after 2000-01-01 in the 365-day calendar: 1 March 2000 at noon.
    let t = units.decode(59.5).unwrap();
    assert_eq!((t.year, t.month, t.day, t.hour), (2000, 3, 1, 12));
    // 365 days later is 1 January 2001 in the 365-day calendar.
    let t = units.decode(365.0).unwrap();
    assert_eq!((t.year, t.month, t.day), (2001, 1, 1));
}

/// The file written by this crate's NetCDF-3 writer (through the `tptplan`
/// CLI, from the zarr-python `v2_zlib` fixture) is read back to the same values.
/// `check_netcdf_writer.py` checks the same file with netCDF4 and xarray.
#[test]
fn file_written_by_tpt_planetary_reads_back_to_zarr_values() {
    let bytes = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/netcdf3/tpt_from_zarr_v2_zlib.nc"),
    )
    .expect("writer fixture");
    let reader = Reader::new(&bytes).expect("parse");
    assert_eq!(reader.format(), Format::Classic);
    let expected_raw = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zarr/v2_zlib.expected.raw"),
    )
    .expect("zarr expected values");
    let expected: Vec<f64> = expected_raw
        .chunks_exact(8)
        .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
        .collect();
    let got = reader.read_f64("v2_zlib").expect("variable");
    assert_eq!(got, expected);
    assert_eq!(reader.shape("v2_zlib").unwrap(), vec![12, 10]);
}
