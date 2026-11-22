//! Integration tests for `tptplan array index`: the Niño-3.4 indices of a Zarr
//! field, checked against a plain computation of the same quantities.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tpt_planetary_array::meta::{ArrayMetadata, ZarrFormat};
use tpt_planetary_array::{Array, DType, FillValue, FsStore, Selection, Store};

const NT: usize = 60; // five years of months
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
        // Tests run in parallel in one process, so each directory gets its own number.
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("tptplan-index-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
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

fn lat_coords() -> Vec<f64> {
    (0..LAT).map(|i| -30.0 + 5.0 * i as f64).collect()
}

fn lon_coords() -> Vec<f64> {
    (0..LON).map(|j| 10.0 * j as f64).collect()
}

/// The field at month `t`, latitude `i`, longitude `j`: seasonal cycle, a warm
/// event in the central Pacific, a warming trend, and a zonal wave.
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

/// The `values` array of the JSON output, parsed without a JSON library.
fn json_values(text: &str) -> Vec<f64> {
    let key = "\"values\": [";
    let start = text.find(key).expect("values member") + key.len();
    let end = start + text[start..].find(']').expect("closing bracket");
    text[start..end]
        .split(',')
        .map(|v| v.trim().parse::<f64>().expect("number"))
        .collect()
}

fn fixture() -> (TempDir, String) {
    let tmp = TempDir::new("field");
    write_field(
        &tmp.0,
        "sst",
        &[NT as u64, LAT as u64, LON as u64],
        &field(),
    );
    let input = tmp.0.join("sst").to_string_lossy().into_owned();
    (tmp, input)
}

fn grid_args() -> [&'static str; 8] {
    [
        "--lat-start",
        "-30",
        "--lat-step",
        "5",
        "--lon-start",
        "0",
        "--lon-step",
        "10",
    ]
}

#[test]
fn nino34_matches_a_direct_computation() {
    let (_tmp, input) = fixture();
    let mut args = vec![
        "--format", "json", "array", "index", "nino34", "--input", &input,
    ];
    args.extend_from_slice(&grid_args());
    let out = tptplan(&args);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let got = json_values(&String::from_utf8_lossy(&out.stdout));
    let want = box_mean(&anomaly(&field()), (-5.0, 5.0), (190.0, 240.0));
    assert_eq!(got.len(), NT);
    for (t, (g, w)) in got.iter().zip(&want).enumerate() {
        assert!((g - w).abs() < 1e-9, "month {t}: {g} vs {w}");
    }
}

#[test]
fn relative_index_is_nino34_minus_the_tropical_mean() {
    let (_tmp, input) = fixture();
    let mut args = vec![
        "--format",
        "json",
        "array",
        "index",
        "nino34-relative",
        "--input",
        &input,
    ];
    args.extend_from_slice(&grid_args());
    let out = tptplan(&args);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let got = json_values(&String::from_utf8_lossy(&out.stdout));
    let anom = anomaly(&field());
    let nino = box_mean(&anom, (-5.0, 5.0), (190.0, 240.0));
    let tropics = box_mean(&anom, (-20.0, 20.0), (0.0, 350.0));
    for t in 0..NT {
        assert!((got[t] - (nino[t] - tropics[t])).abs() < 1e-9, "month {t}");
    }
}

#[test]
fn table_output_has_one_row_per_month() {
    let (_tmp, input) = fixture();
    let mut args = vec!["array", "index", "nino34", "--input", &input];
    args.extend_from_slice(&grid_args());
    let out = tptplan(&args);
    assert_eq!(code(&out), 0);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("index: nino34"));
    assert!(text.contains("time steps: 60"));
    // Rows are "month value"; the header lines do not start with a number.
    let rows = text
        .lines()
        .filter(|l| {
            l.split_whitespace()
                .next()
                .is_some_and(|w| w.parse::<usize>().is_ok())
        })
        .count();
    assert_eq!(rows, NT);
}

#[test]
fn bad_requests_are_usage_errors() {
    let (_tmp, input) = fixture();
    // Unknown index.
    let mut args = vec!["array", "index", "nino4", "--input", &input];
    args.extend_from_slice(&grid_args());
    assert_eq!(code(&tptplan(&args)), 2);
    // A descending (non-positive) step.
    let descending = [
        "--lat-start",
        "30",
        "--lat-step",
        "-5",
        "--lon-start",
        "0",
        "--lon-step",
        "10",
    ];
    let mut args = vec!["array", "index", "nino34", "--input", &input];
    args.extend_from_slice(&descending);
    assert_eq!(code(&tptplan(&args)), 2);
    // A missing input is a runtime error.
    let mut args = vec!["array", "index", "nino34", "--input", "no-such-field-dir"];
    args.extend_from_slice(&grid_args());
    assert_eq!(code(&tptplan(&args)), 1);
}

#[test]
fn a_field_that_is_not_three_dimensional_is_a_usage_error() {
    let tmp = TempDir::new("flat");
    write_field(&tmp.0, "flat", &[4, 4], &[1.0; 16]);
    let input = tmp.0.join("flat").to_string_lossy().into_owned();
    let mut args = vec!["array", "index", "nino34", "--input", &input];
    args.extend_from_slice(&grid_args());
    assert_eq!(code(&tptplan(&args)), 2);
}
