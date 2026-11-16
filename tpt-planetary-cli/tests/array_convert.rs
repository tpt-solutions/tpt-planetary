//! Integration tests for `tptplan array convert`: Zarr to NetCDF-3 and back.
//!
//! Zarr arrays are written with the array crate, converted by the binary, and
//! read back with the array crate or the NetCDF-3 reader. Values are compared
//! exactly, because the supported types are the ones NetCDF-3 holds natively.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use tpt_planetary_array::dtype::{Endian, Kind};
use tpt_planetary_array::meta::{ArrayMetadata, GroupMetadata, ZarrFormat};
use tpt_planetary_array::netcdf3::{Format, NcValues, Reader, Writer};
use tpt_planetary_array::{Array, DType, FillValue, FsStore, Selection, Store, hierarchy};

const EXIT_OK: i32 = 0;
const EXIT_RUNTIME: i32 = 1;
const EXIT_USAGE: i32 = 2;

fn tptplan(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tptplan"))
        .args(args)
        .output()
        .expect("failed to run tptplan")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("process exited with a code")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A scratch directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("tptplan-convert-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }

    fn root(&self) -> &Path {
        &self.0
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn s(&self, name: &str) -> String {
        self.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Creates a Zarr v2 array at `dir/node` (node may be `group/child`) with the
/// given shape, dimension names and float64 values, chunked in halves.
fn write_array(
    dir: &Path,
    node: &str,
    dtype: DType,
    shape: &[u64],
    names: &[&str],
    values: &[f64],
) {
    let store: Arc<dyn Store> = Arc::new(FsStore::open(dir).expect("open store"));
    let chunks: Vec<u64> = shape.iter().map(|&n| n.div_ceil(2).max(1)).collect();
    let meta = ArrayMetadata::new(ZarrFormat::V2, shape.to_vec(), chunks, dtype)
        .expect("metadata")
        .with_fill_value(FillValue::Null)
        .with_dimension_names(names.iter().map(|n| Some((*n).to_string())).collect());
    let array = Array::create(store, node, meta).expect("create array");
    let selection = vec![Selection::ALL; shape.len()];
    array.write_f64(&selection, values).expect("write values");
}

/// A NetCDF-3 variable for the tests: name, (dimension name, length) pairs and values.
type NetcdfVar<'a> = (&'a str, &'a [(&'a str, usize)], NcValues);

/// Writes a NetCDF-3 file with the given variables.
fn write_netcdf(path: &Path, vars: &[NetcdfVar]) {
    let mut w = Writer::new(Format::Classic);
    let mut ids_by_name: Vec<(&str, usize)> = Vec::new();
    for (name, dims, values) in vars {
        let mut ids = Vec::with_capacity(dims.len());
        for &(dim, len) in dims.iter() {
            let id = match ids_by_name.iter().find(|(n, _)| *n == dim) {
                Some(&(_, id)) => id,
                None => {
                    let id = w.add_dimension(dim, Some(len)).expect("add dimension");
                    ids_by_name.push((dim, id));
                    id
                }
            };
            ids.push(id);
        }
        w.add_variable(name, &ids, Vec::new(), values.clone())
            .expect("add variable");
    }
    fs::write(path, w.finish().expect("finish")).expect("write nc");
}

fn read_zarr(dir: &Path, node: &str) -> (Vec<u64>, Vec<f64>, Option<Vec<Option<String>>>) {
    let store: Arc<dyn Store> = Arc::new(FsStore::open(dir).expect("open store"));
    let array = Array::open(store, node).expect("open array");
    let shape = array.shape().to_vec();
    let selection = vec![Selection::ALL; shape.len()];
    let values = array.read_f64(&selection).expect("read values");
    (shape, values, array.metadata().dimension_names.clone())
}

#[test]
fn zarr_array_to_netcdf_and_back_keeps_values_and_dimensions() {
    let tmp = TempDir::new("roundtrip");
    let values: Vec<f64> = (0..24).map(|i| f64::from(i) * 0.5 - 3.0).collect();
    write_array(
        tmp.root(),
        "temp.zarr",
        DType::FLOAT64,
        &[2, 3, 4],
        &["time", "lat", "lon"],
        &values,
    );

    let nc = tmp.s("temp.nc");
    let out = tptplan(&["array", "convert", &tmp.s("temp.zarr"), &nc]);
    assert_eq!(code(&out), EXIT_OK, "stderr: {}", stderr(&out));
    let reader_bytes = fs::read(&nc).expect("read nc");
    let reader = Reader::new(&reader_bytes).expect("valid NetCDF-3");
    assert_eq!(reader.read_f64("temp").expect("temp"), values);
    assert_eq!(reader.shape("temp").expect("shape"), vec![2, 3, 4]);
    let dims: Vec<&str> = reader
        .variable("temp")
        .unwrap()
        .dim_ids
        .iter()
        .map(|&id| reader.dimensions()[id].name.as_str())
        .collect();
    assert_eq!(dims, vec!["time", "lat", "lon"]);

    // NetCDF back to Zarr: the output is a group holding one array per variable.
    let back = tmp.s("back.zarr");
    let out = tptplan(&["array", "convert", &nc, &back]);
    assert_eq!(code(&out), EXIT_OK, "stderr: {}", stderr(&out));
    let (shape, got, names) = read_zarr(tmp.root(), "back.zarr/temp");
    assert_eq!(shape, vec![2, 3, 4]);
    assert_eq!(got, values);
    assert_eq!(
        names,
        Some(vec![
            Some("time".to_string()),
            Some("lat".to_string()),
            Some("lon".to_string())
        ])
    );
}

#[test]
fn group_of_mixed_numeric_arrays_converts_each_variable() {
    let tmp = TempDir::new("group");
    let root = tmp.root();
    let store: Arc<dyn Store> = Arc::new(FsStore::open(root).expect("open store"));
    hierarchy::write_group(
        store.as_ref(),
        "fields.zarr",
        ZarrFormat::V2,
        &GroupMetadata::default(),
    )
    .expect("group");
    let x: Vec<f64> = (0..5).map(f64::from).collect();
    let y: Vec<f64> = (0..12).map(|i| f64::from(i) + 0.25).collect();
    let z: Vec<f64> = (0..8).map(|i| f64::from(i) - 4.0).collect();
    write_array(
        root,
        "fields.zarr/x",
        DType::new(Kind::Int(32), Endian::Little).unwrap(),
        &[5],
        &["x"],
        &x,
    );
    write_array(
        root,
        "fields.zarr/y",
        DType::FLOAT64,
        &[3, 4],
        &["y", "col"],
        &y,
    );
    write_array(
        root,
        "fields.zarr/z",
        DType::new(Kind::Int(8), Endian::Little).unwrap(),
        &[2, 2, 2],
        &["a", "b", "c"],
        &z,
    );

    let nc = tmp.s("fields.nc");
    let out = tptplan(&["array", "convert", &tmp.s("fields.zarr"), &nc]);
    assert_eq!(code(&out), EXIT_OK, "stderr: {}", stderr(&out));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!(
            "converted 3 variables from {} to NetCDF-3 file {nc}",
            tmp.s("fields.zarr")
        )
    );

    let bytes = fs::read(&nc).expect("read nc");
    let reader = Reader::new(&bytes).expect("valid NetCDF-3");
    assert_eq!(reader.read_i32("x").expect("x"), vec![0, 1, 2, 3, 4]);
    assert_eq!(reader.read_f64("y").expect("y"), y);
    assert_eq!(
        reader.read_values("z").expect("z"),
        NcValues::Byte(z.iter().map(|&v| v as i8).collect())
    );
    // Dimension "x" is shared by x and y and must have one length.
    assert_eq!(reader.dimension("x").map(|d| d.len), Some(5));

    // Zarr v3 output from the same file.
    let back = tmp.s("back3.zarr");
    let out = tptplan(&["array", "convert", &nc, &back, "--zarr-format", "3"]);
    assert_eq!(code(&out), EXIT_OK, "stderr: {}", stderr(&out));
    let (shape, got, _) = read_zarr(root, "back3.zarr/y");
    assert_eq!(shape, vec![3, 4]);
    assert_eq!(got, y);
}

#[test]
fn unsupported_dtypes_are_rejected_with_a_clear_message() {
    let tmp = TempDir::new("dtype");
    let root = tmp.root();
    write_array(root, "wide.zarr", DType::INT64, &[2], &["n"], &[1.0, 2.0]);
    let out = tptplan(&["array", "convert", &tmp.s("wide.zarr"), &tmp.s("wide.nc")]);
    assert_eq!(code(&out), EXIT_RUNTIME);
    assert!(
        stderr(&out).contains("no 64-bit integers"),
        "stderr: {}",
        stderr(&out)
    );
    assert!(!tmp.join("wide.nc").exists());

    write_array(
        root,
        "unsigned.zarr",
        DType::new(Kind::UInt(8), Endian::Little).unwrap(),
        &[2],
        &["n"],
        &[1.0, 2.0],
    );
    let out = tptplan(&[
        "array",
        "convert",
        &tmp.s("unsigned.zarr"),
        &tmp.s("unsigned.nc"),
    ]);
    assert_eq!(code(&out), EXIT_RUNTIME);
    assert!(stderr(&out).contains("no unsigned integers"));
}

#[test]
fn netcdf_text_variables_are_rejected() {
    let tmp = TempDir::new("text");
    write_netcdf(
        &tmp.join("text.nc"),
        &[("label", &[("n", 3)], NcValues::text("abc"))],
    );
    let out = tptplan(&["array", "convert", &tmp.s("text.nc"), &tmp.s("text.zarr")]);
    assert_eq!(code(&out), EXIT_RUNTIME);
    assert!(stderr(&out).contains("NC_CHAR"), "stderr: {}", stderr(&out));
}

#[test]
fn direction_and_path_errors_have_the_right_exit_codes() {
    let tmp = TempDir::new("paths");
    let root = tmp.root();
    write_array(root, "a.zarr", DType::FLOAT64, &[2], &["n"], &[1.0, 2.0]);
    write_netcdf(
        &tmp.join("b.nc"),
        &[("v", &[("n", 2)], NcValues::Double(vec![1.0, 2.0]))],
    );

    // Zarr to Zarr and NetCDF to NetCDF are not conversions.
    let out = tptplan(&["array", "convert", &tmp.s("a.zarr"), &tmp.s("c.zarr")]);
    assert_eq!(code(&out), EXIT_USAGE);
    let out = tptplan(&["array", "convert", &tmp.s("b.nc"), &tmp.s("c.nc")]);
    assert_eq!(code(&out), EXIT_USAGE);

    // Input that is neither .nc nor a directory.
    fs::write(tmp.join("notes.txt"), "x").unwrap();
    let out = tptplan(&["array", "convert", &tmp.s("notes.txt"), &tmp.s("d.zarr")]);
    assert_eq!(code(&out), EXIT_USAGE);

    // Missing input and existing output are runtime errors.
    let out = tptplan(&["array", "convert", &tmp.s("missing.nc"), &tmp.s("e.zarr")]);
    assert_eq!(code(&out), EXIT_RUNTIME);
    let out = tptplan(&["array", "convert", &tmp.s("b.nc"), &tmp.s("a.zarr")]);
    assert_eq!(code(&out), EXIT_RUNTIME);
    assert!(stderr(&out).contains("already exists"));
}
