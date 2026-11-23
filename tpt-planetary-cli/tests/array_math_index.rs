//! Integration tests for the climate functions of `tptplan array math`
//! (`anomaly`, `nino34` and `nino34_relative`), checked against a plain
//! computation of the same quantities.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tpt_planetary_array::meta::{ArrayMetadata, ZarrFormat};
use tpt_planetary_array::{Array, DType, FillValue, FsStore, Selection, Store};

const NT: usize = 60;
const LAT: usize = 13; // -30 .. 30 in 5-degree steps
const LON: usize = 36; // 0 .. 350 in 10-degree steps

fn tptplan(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tptplan"))
        .args(args)
        .output()
        .expect("failed to run tptplan")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("process exited with a code")
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("tptplan-math-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Writes a Zarr v2 float64 array of `shape` holding `values` at `dir/node`.
fn write_field(dir: &Path, node: &str, shape: &[u64], values: &[f64]) {
    fs::create_dir_all(dir.join(node)).expect("create array directory");
    let store: Arc<dyn Store> = Arc::new(FsStore::open(dir).expect("open store"));
    let chunks: Vec<u64> = shape.iter().map(|&n| n.div_ceil(2).max(1)).collect();
    let meta = ArrayMetadata::new(ZarrFormat::V2, shape.to_vec(), chunks, DType::FLOAT64)
        .expect("metadata")
        .with_fill_value(FillValue::Null);
    let array = Array::create(store, node, meta).expect("create array");
    array
        .write_f64(&vec![Selection::ALL; shape.len()], values)
        .expect("write values");
}

/// Reads the whole output array written at `path`.
fn read_output(path: &Path) -> (Vec<u64>, Vec<f64>) {
    let store: Arc<dyn Store> = Arc::new(FsStore::open(path).expect("open output store"));
    let array = Array::open(store, "").expect("open output array");
    let shape = array.shape().to_vec();
    let values = array
        .read_f64(&vec![Selection::ALL; shape.len()])
        .expect("read output");
    (shape, values)
}

fn lat_coords() -> Vec<f64> {
    (0..LAT).map(|i| -30.0 + 5.0 * i as f64).collect()
}

fn lon_coords() -> Vec<f64> {
    (0..LON).map(|j| 10.0 * j as f64).collect()
}

/// The field at month `t`, latitude `i`, longitude `j`.
fn value(t: usize, i: usize, j: usize) -> f64 {
    let lat = lat_coords()[i];
    let lon = lon_coords()[j];
    let pi2 = 2.0 * std::f64::consts::PI;
    let season = 1.2 * (pi2 * t as f64 / 12.0 + lat.to_radians()).cos();
    let event = 0.8
        * (pi2 * t as f64 / 40.0).sin()
        * (-((lat / 8.0).powi(2)) - ((lon - 215.0) / 40.0).powi(2)).exp();
    season + event + 0.02 * t as f64 + 0.1 * (lon / 40.0).sin()
}

fn field() -> Vec<f64> {
    let mut data = Vec::with_capacity(NT * LAT * LON);
    for t in 0..NT {
        for i in 0..LAT {
            for j in 0..LON {
                data.push(value(t, i, j));
            }
        }
    }
    data
}

/// Monthly anomaly by hand: subtract each calendar month's mean over the record.
fn anomaly(data: &[f64]) -> Vec<f64> {
    let cell = LAT * LON;
    let mut sums = vec![0.0; 12 * cell];
    let mut count = [0usize; 12];
    for t in 0..NT {
        count[t % 12] += 1;
        for k in 0..cell {
            sums[(t % 12) * cell + k] += data[t * cell + k];
        }
    }
    let mut out = data.to_vec();
    for t in 0..NT {
        for k in 0..cell {
            out[t * cell + k] -= sums[(t % 12) * cell + k] / count[t % 12] as f64;
        }
    }
    out
}

/// Area-weighted box mean per month, by hand.
fn box_mean(anom: &[f64], lat_range: (f64, f64), lon_range: (f64, f64)) -> Vec<f64> {
    let cell = LAT * LON;
    (0..NT)
        .map(|t| {
            let (mut num, mut den) = (0.0, 0.0);
            for i in 0..LAT {
                let lat = lat_coords()[i];
                if lat < lat_range.0 || lat > lat_range.1 {
                    continue;
                }
                let w = lat.to_radians().cos();
                for j in 0..LON {
                    let lon = lon_coords()[j];
                    if lon < lon_range.0 || lon > lon_range.1 {
                        continue;
                    }
                    num += anom[t * cell + i * LON + j] * w;
                    den += w;
                }
            }
            num / den
        })
        .collect()
}

/// A directory with the field as input `sst`, and the output path inside it.
fn setup(label: &str) -> (TempDir, PathBuf, String) {
    let dir = TempDir::new(label);
    write_field(
        dir.path(),
        "sst",
        &[NT as u64, LAT as u64, LON as u64],
        &field(),
    );
    let input = format!("sst={}", dir.path().join("sst").display());
    let output = dir.path().join("out.zarr");
    (dir, output, input)
}

fn math(expression: &str, input: &str, output: &Path, grid: &[&str]) -> Output {
    let mut args = vec![
        "array",
        "math",
        expression,
        "--input",
        input,
        "--output",
        output.to_str().unwrap(),
    ];
    args.extend_from_slice(grid);
    tptplan(&args)
}

const GRID: [&str; 8] = [
    "--lat-start",
    "-30",
    "--lat-step",
    "5",
    "--lon-start",
    "0",
    "--lon-step",
    "10",
];

fn assert_close(got: &[f64], want: &[f64], tol: f64, what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: length");
    for (k, (g, w)) in got.iter().zip(want).enumerate() {
        assert!((g - w).abs() < tol, "{what} at {k}: got {g}, want {w}");
    }
}

#[test]
fn anomaly_keeps_the_shape_and_matches_the_hand_computation() {
    let (_dir, output, input) = setup("anomaly");
    let out = math("anomaly(sst)", &input, &output, &[]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let (shape, values) = read_output(&output);
    assert_eq!(shape, vec![NT as u64, LAT as u64, LON as u64]);
    assert_close(&values, &anomaly(&field()), 1e-9, "anomaly");
}

#[test]
fn nino34_is_one_value_per_month_and_matches_the_hand_computation() {
    let (_dir, output, input) = setup("nino34");
    let out = math("nino34(sst)", &input, &output, &GRID);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let (shape, values) = read_output(&output);
    assert_eq!(shape, vec![NT as u64]);
    let want = box_mean(&anomaly(&field()), (-5.0, 5.0), (190.0, 240.0));
    assert_close(&values, &want, 1e-9, "nino34");
}

#[test]
fn nino34_relative_subtracts_the_tropical_mean() {
    let (_dir, output, input) = setup("relative");
    let out = math("nino34_relative(sst)", &input, &output, &GRID);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let (_, values) = read_output(&output);
    let anom = anomaly(&field());
    let nino = box_mean(&anom, (-5.0, 5.0), (190.0, 240.0));
    let tropics = box_mean(&anom, (-20.0, 20.0), (0.0, 360.0));
    let want: Vec<f64> = nino.iter().zip(&tropics).map(|(n, t)| n - t).collect();
    assert_close(&values, &want, 1e-9, "nino34_relative");
}

#[test]
fn an_index_combines_with_arithmetic() {
    let (_dir, output, input) = setup("combined");
    let out = math("nino34(sst) * 2 - 1", &input, &output, &GRID);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let (_, values) = read_output(&output);
    let want: Vec<f64> = box_mean(&anomaly(&field()), (-5.0, 5.0), (190.0, 240.0))
        .iter()
        .map(|v| v * 2.0 - 1.0)
        .collect();
    assert_close(&values, &want, 1e-9, "combined");
}

#[test]
fn an_index_without_a_grid_is_a_usage_error() {
    let (_dir, output, input) = setup("nogrid");
    let out = math("nino34(sst)", &input, &output, &[]);
    assert_eq!(code(&out), 2);
    assert!(!output.exists());
}

#[test]
fn a_partial_grid_is_a_usage_error() {
    let (_dir, output, input) = setup("partial");
    let out = math("nino34(sst)", &input, &output, &["--lat-start", "-30"]);
    assert_eq!(code(&out), 2);
}

#[test]
fn an_unknown_function_is_a_syntax_error() {
    let (_dir, output, input) = setup("unknown");
    let out = math("climatology(sst)", &input, &output, &[]);
    assert_eq!(code(&out), 2);
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a function"));
}
