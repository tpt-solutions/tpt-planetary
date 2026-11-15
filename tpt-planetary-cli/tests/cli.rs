//! End-to-end tests that run the `tptplan` binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use tpt_planetary_array::meta::{ArrayMetadata, ZarrFormat};
use tpt_planetary_array::{Array, DType, FillValue, FsStore, Selection, Store};

/// Exit codes, as documented in `main.rs`.
const EXIT_OK: i32 = 0;
const EXIT_RUNTIME: i32 = 1;
const EXIT_USAGE: i32 = 2;
const EXIT_NOT_IMPLEMENTED: i32 = 3;

fn tptplan(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tptplan"))
        .args(args)
        .output()
        .expect("failed to run tptplan")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("process exited with a code")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A scratch directory under the system temp dir, named with the process id
/// and a label, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("tptplan-cli-test-{label}-{}", std::process::id()));
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

/// Creates a Zarr v2 array `name` under `parent` with shape [4, 6], chunks
/// [2, 3], float64 data, dimension names (y, x) and a `units` attribute.
fn make_array(parent: &Path, name: &str) {
    let store: Arc<dyn Store> = Arc::new(FsStore::open(parent).expect("open store"));
    let meta = ArrayMetadata::new(ZarrFormat::V2, vec![4, 6], vec![2, 3], DType::FLOAT64)
        .expect("metadata")
        .with_fill_value(FillValue::Float(f64::NAN))
        .with_dimension_names(vec![Some("y".to_string()), Some("x".to_string())]);
    let array = Array::create(store, name, meta).expect("create array");
    let values: Vec<f64> = (0..24).map(f64::from).collect();
    array
        .write_f64(&[Selection::ALL, Selection::ALL], &values)
        .expect("write data");
    // Attributes are written by hand so that the test needs no serde_json.
    // The v2 dimension names live in .zattrs under _ARRAY_DIMENSIONS, so the
    // file must carry both keys (writing it again replaces the crate's file).
    let attrs = r#"{"units": "K", "_ARRAY_DIMENSIONS": ["y", "x"]}"#;
    fs::write(parent.join(name).join(".zattrs"), attrs).expect("write attrs");
}

#[test]
fn version_flag_prints_name_and_version() {
    let out = tptplan(&["--version"]);
    assert_eq!(code(&out), EXIT_OK);
    let text = stdout(&out);
    assert!(text.contains("tptplan"), "stdout: {text}");
    assert!(text.contains(env!("CARGO_PKG_VERSION")), "stdout: {text}");
}

#[test]
fn bad_arguments_exit_with_usage_code() {
    for args in [
        vec!["astro", "convert"],
        vec!["astro", "convert", "2017-01-01T00:00:00", "--from", "utc"],
        vec!["frobnicate"],
        vec!["--format", "yaml", "array", "inspect", "x.zarr"],
        vec!["array"],
    ] {
        let out = tptplan(&args);
        assert_eq!(
            code(&out),
            EXIT_USAGE,
            "args {args:?}, stderr {}",
            stderr(&out)
        );
    }
}

#[test]
fn astro_convert_utc_to_tai_on_2017_new_year() {
    let out = tptplan(&[
        "astro",
        "convert",
        "2017-01-01T00:00:00",
        "--from",
        "utc",
        "--to",
        "tai",
    ]);
    assert_eq!(code(&out), EXIT_OK, "stderr {}", stderr(&out));
    assert!(
        stdout(&out).starts_with("2017-01-01T00:00:37 TAI"),
        "stdout: {}",
        stdout(&out)
    );
}

#[test]
fn astro_convert_json_output() {
    let out = tptplan(&[
        "--format",
        "json",
        "astro",
        "convert",
        "2017-01-01T00:00:00",
        "--from",
        "utc",
        "--to",
        "tai",
    ]);
    assert_eq!(code(&out), EXIT_OK, "stderr {}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains(r#""timestamp": "2017-01-01T00:00:37""#),
        "stdout: {text}"
    );
}

#[test]
fn astro_convert_bad_timestamp_is_usage_error() {
    let out = tptplan(&[
        "astro",
        "convert",
        "2017-01-01",
        "--from",
        "utc",
        "--to",
        "tai",
    ]);
    assert_eq!(code(&out), EXIT_USAGE);
}

#[test]
fn array_math_and_convert_are_not_implemented() {
    let math = tptplan(&["array", "math", "a + b"]);
    assert_eq!(code(&math), EXIT_NOT_IMPLEMENTED);
    assert!(stderr(&math).contains("array math is not implemented"));

    let convert = tptplan(&["array", "convert", "in.nc", "out.zarr"]);
    assert_eq!(code(&convert), EXIT_NOT_IMPLEMENTED);
    assert!(stderr(&convert).contains("array convert is not implemented"));
}

#[test]
fn remote_array_locations_are_not_implemented() {
    for uri in [
        "s3://bucket/temperature.zarr",
        "https://example.org/temperature.zarr",
    ] {
        let out = tptplan(&["array", "inspect", uri]);
        assert_eq!(code(&out), EXIT_NOT_IMPLEMENTED, "uri {uri}");
        assert!(
            stderr(&out).contains("not supported in this build"),
            "stderr: {}",
            stderr(&out)
        );
    }
}

#[test]
fn non_time_scales_are_not_implemented() {
    let out = tptplan(&[
        "astro", "convert", "10,20", "--from", "icrs", "--to", "galactic",
    ]);
    assert_eq!(code(&out), EXIT_NOT_IMPLEMENTED);
}

#[test]
fn ephemeris_without_kernel_is_not_implemented() {
    let out = tptplan(&["ephemeris", "query", "jupiter", "2017-01-01T00:00:00"]);
    assert_eq!(code(&out), EXIT_NOT_IMPLEMENTED);
    assert!(stderr(&out).contains("--kernel"));
}

#[test]
fn ephemeris_with_missing_kernel_is_runtime_error() {
    let out = tptplan(&[
        "ephemeris",
        "query",
        "jupiter",
        "2017-01-01T00:00:00",
        "--kernel",
        "definitely-missing-kernel.bsp",
    ]);
    assert_eq!(code(&out), EXIT_RUNTIME);
}

#[test]
fn ephemeris_unknown_body_is_usage_error() {
    let out = tptplan(&[
        "ephemeris",
        "query",
        "pluto",
        "2017-01-01T00:00:00",
        "--kernel",
        "unused.bsp",
    ]);
    assert_eq!(code(&out), EXIT_USAGE);
}

#[test]
fn array_inspect_reports_metadata_in_table_form() {
    let dir = TempDir::new("inspect-table");
    make_array(dir.path(), "temperature.zarr");
    let target = dir.path().join("temperature.zarr");
    let out = tptplan(&["array", "inspect", target.to_str().expect("utf-8 path")]);
    assert_eq!(code(&out), EXIT_OK, "stderr {}", stderr(&out));
    let text = stdout(&out);
    for expected in [
        "format: zarr v2",
        "shape: [4, 6]",
        "chunk shape: [2, 3]",
        "chunk grid: [2, 2]",
        "dtype: <f8",
        "fill value: \"NaN\"",
        "dimension names: [y, x]",
        "  units: \"K\"",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
    }
}

#[test]
fn array_inspect_json_output() {
    let dir = TempDir::new("inspect-json");
    make_array(dir.path(), "temperature.zarr");
    let target = dir.path().join("temperature.zarr");
    let out = tptplan(&[
        "--format",
        "json",
        "array",
        "inspect",
        target.to_str().expect("utf-8 path"),
    ]);
    assert_eq!(code(&out), EXIT_OK, "stderr {}", stderr(&out));
    let text = stdout(&out);
    for expected in [
        r#""format": "zarr v2""#,
        r#""shape": [4, 6]"#,
        r#""chunk_shape": [2, 3]"#,
        r#""dtype": "<f8""#,
        r#""dimension_names": ["y", "x"]"#,
        r#""attributes": {"#,
        r#""units": "K""#,
    ] {
        assert!(text.contains(expected), "missing {expected} in:\n{text}");
    }
}

#[test]
fn array_inspect_verbose_prints_stages_to_stderr() {
    let dir = TempDir::new("inspect-verbose");
    make_array(dir.path(), "temperature.zarr");
    let target = dir.path().join("temperature.zarr");
    let out = tptplan(&[
        "-v",
        "array",
        "inspect",
        target.to_str().expect("utf-8 path"),
    ]);
    assert_eq!(code(&out), EXIT_OK, "stderr {}", stderr(&out));
    let err = stderr(&out);
    for stage in [
        "checking location",
        "opening store",
        "reading node metadata",
        "formatting output",
    ] {
        assert!(err.contains(stage), "missing stage {stage:?} in: {err}");
    }
    assert!(!stdout(&out).contains("checking location"));
}

#[test]
fn array_inspect_on_missing_or_empty_directory_is_runtime_error() {
    let missing = tptplan(&["array", "inspect", "definitely-missing-array.zarr"]);
    assert_eq!(code(&missing), EXIT_RUNTIME);

    let dir = TempDir::new("inspect-empty");
    let out = tptplan(&["array", "inspect", dir.path().to_str().expect("utf-8 path")]);
    assert_eq!(code(&out), EXIT_RUNTIME);
    assert!(stderr(&out).contains("no Zarr metadata"));
}
