//! Integration tests for `tptplan array math`.
//!
//! Inputs are small Zarr arrays written with the array crate. Results are
//! compared with values computed by hand in the test, and the exit codes are
//! checked for syntax errors (2) and missing inputs (1).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use tpt_planetary_array::meta::{ArrayMetadata, ZarrFormat};
use tpt_planetary_array::{Array, DType, FillValue, FsStore, Selection, Store};

const EXIT_OK: i32 = 0;
const EXIT_RUNTIME: i32 = 1;
const EXIT_USAGE: i32 = 2;

const ROWS: u64 = 3;
const COLS: u64 = 4;

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
            std::env::temp_dir().join(format!("tptplan-math-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }

    fn root(&self) -> &Path {
        &self.0
    }

    fn s(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Writes a `ROWS x COLS` float64 Zarr array `name` under `dir`.
fn write_input(dir: &Path, name: &str, values: &[f64], fill: FillValue) {
    let store: Arc<dyn Store> = Arc::new(FsStore::open(dir).expect("open store"));
    let meta = ArrayMetadata::new(ZarrFormat::V2, vec![ROWS, COLS], vec![2, 3], DType::FLOAT64)
        .expect("metadata")
        .with_fill_value(fill);
    let array = Array::create(store, name, meta).expect("create array");
    array
        .write_f64(&[Selection::ALL, Selection::ALL], values)
        .expect("write values");
}

fn read_result(dir: &Path, name: &str) -> Vec<f64> {
    let store: Arc<dyn Store> = Arc::new(FsStore::open(dir).expect("open store"));
    let array = Array::open(store, name).expect("open result");
    assert_eq!(array.shape(), &[ROWS, COLS]);
    array
        .read_f64(&[Selection::ALL, Selection::ALL])
        .expect("read result")
}

fn a_values() -> Vec<f64> {
    // a[i][j] = 10 i + j
    (0..ROWS)
        .flat_map(|i| (0..COLS).map(move |j| (10 * i + j) as f64))
        .collect()
}

fn b_values() -> Vec<f64> {
    // b[i][j] = i + 0.5 (non-zero, so it can be divided by)
    (0..ROWS)
        .flat_map(|i| (0..COLS).map(move |_| i as f64 + 0.5))
        .collect()
}

/// Sets up inputs `a` and `b` in a fresh temp directory.
fn setup(label: &str) -> TempDir {
    let tmp = TempDir::new(label);
    write_input(tmp.root(), "a.zarr", &a_values(), FillValue::Null);
    write_input(tmp.root(), "b.zarr", &b_values(), FillValue::Null);
    tmp
}

fn run_math(tmp: &TempDir, expression: &str, out_name: &str) -> Output {
    tptplan(&[
        "array",
        "math",
        expression,
        "--input",
        &format!("a={}", tmp.s("a.zarr")),
        "--input",
        &format!("b={}", tmp.s("b.zarr")),
        "--output",
        &tmp.s(out_name),
    ])
}

fn assert_close(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (k, (a, e)) in actual.iter().zip(expected).enumerate() {
        assert!(
            (a - e).abs() <= 1e-12 * e.abs().max(1.0),
            "element {k}: got {a}, expected {e}"
        );
    }
}

#[test]
fn expression_on_two_arrays_matches_hand_computed_values() {
    let tmp = setup("hand");
    let a = a_values();
    let b = b_values();

    // (a - b) * 2 + -b / 4 + 1.5, computed element by element.
    let out = run_math(&tmp, "(a - b) * 2 + -b / 4 + 1.5", "out.zarr");
    assert_eq!(code(&out), EXIT_OK, "stderr: {}", stderr(&out));
    let expected: Vec<f64> = a
        .iter()
        .zip(&b)
        .map(|(a, b)| (a - b) * 2.0 + (-b) / 4.0 + 1.5)
        .collect();
    assert_close(&read_result(tmp.root(), "out.zarr"), &expected);
}

#[test]
fn scalar_operands_on_either_side_broadcast() {
    let tmp = setup("scalars");
    let a = a_values();
    let b = b_values();

    // 10 / b - 2 * a, and 3 - a, and a / 2.
    let out = run_math(&tmp, "10 / b - 2 * a", "div.zarr");
    assert_eq!(code(&out), EXIT_OK, "stderr: {}", stderr(&out));
    let expected: Vec<f64> = a.iter().zip(&b).map(|(a, b)| 10.0 / b - 2.0 * a).collect();
    assert_close(&read_result(tmp.root(), "div.zarr"), &expected);

    let out = run_math(&tmp, "3 - a / 2", "sub.zarr");
    assert_eq!(code(&out), EXIT_OK, "stderr: {}", stderr(&out));
    let expected: Vec<f64> = a.iter().map(|a| 3.0 - a / 2.0).collect();
    assert_close(&read_result(tmp.root(), "sub.zarr"), &expected);
}

#[test]
fn fill_values_in_inputs_become_nan_in_the_output() {
    let tmp = TempDir::new("fill");
    let mut values = a_values();
    values[5] = -9999.0;
    write_input(tmp.root(), "a.zarr", &values, FillValue::Float(-9999.0));
    write_input(tmp.root(), "b.zarr", &b_values(), FillValue::Null);
    let out = run_math(&tmp, "a + b", "out.zarr");
    assert_eq!(code(&out), EXIT_OK, "stderr: {}", stderr(&out));
    let got = read_result(tmp.root(), "out.zarr");
    assert!(got[5].is_nan());
    assert_eq!(got[0], a_values()[0] + b_values()[0]);
}

#[test]
fn syntax_errors_exit_with_code_2() {
    let tmp = setup("syntax");
    for bad in ["a +", "a $ b", "(a", "a b", "", "1.2.3", "*a"] {
        let out = run_math(&tmp, bad, "never.zarr");
        assert_eq!(
            code(&out),
            EXIT_USAGE,
            "expression {bad:?}: {}",
            stderr(&out)
        );
        assert!(
            stderr(&out).contains("syntax error") || stderr(&out).contains("empty"),
            "expression {bad:?}: {}",
            stderr(&out)
        );
    }
    assert!(!tmp.root().join("never.zarr").exists());
}

#[test]
fn constant_only_expression_is_a_usage_error() {
    let tmp = setup("constant");
    let out = tptplan(&["array", "math", "2 * 3", "--output", &tmp.s("c.zarr")]);
    assert_eq!(code(&out), EXIT_USAGE);
    assert!(stderr(&out).contains("no array operand"));
}

#[test]
fn missing_inputs_exit_with_code_1() {
    let tmp = setup("missing");
    // The name c is used but not given.
    let out = tptplan(&[
        "array",
        "math",
        "a + c",
        "--input",
        &format!("a={}", tmp.s("a.zarr")),
        "--output",
        &tmp.s("out.zarr"),
    ]);
    assert_eq!(code(&out), EXIT_RUNTIME);
    assert!(stderr(&out).contains("no input named 'c'"));

    // The input path does not exist.
    let out = tptplan(&[
        "array",
        "math",
        "a * 2",
        "--input",
        &format!("a={}", tmp.s("nope.zarr")),
        "--output",
        &tmp.s("out.zarr"),
    ]);
    assert_eq!(code(&out), EXIT_RUNTIME);
    assert!(stderr(&out).contains("cannot open input 'a'"));
    assert!(!tmp.root().join("out.zarr").exists());
}

#[test]
fn output_path_rules_are_enforced() {
    let tmp = setup("output");
    let out = run_math(&tmp, "a + b", "a.zarr");
    assert_eq!(
        code(&out),
        EXIT_RUNTIME,
        "existing output: {}",
        stderr(&out)
    );
    assert!(stderr(&out).contains("already exists"));

    let out = run_math(&tmp, "a + b", "result.nc");
    assert_eq!(code(&out), EXIT_USAGE);
    assert!(stderr(&out).contains("must not end in .nc"));
}

#[test]
fn bad_input_specs_are_usage_errors() {
    let tmp = setup("specs");
    let out = tptplan(&[
        "array",
        "math",
        "a",
        "--input",
        "a-path",
        "--output",
        &tmp.s("o.zarr"),
    ]);
    assert_eq!(code(&out), EXIT_USAGE);
    assert!(stderr(&out).contains("expected NAME=PATH"));
}

#[test]
fn help_documents_the_grammar() {
    let out = tptplan(&["array", "math", "--help"]);
    assert_eq!(code(&out), EXIT_OK);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("GRAMMAR"));
    assert!(text.contains("unary      := '-' unary | primary"));
    assert!(text.contains("--input <NAME=PATH>") || text.contains("--input"));
}
