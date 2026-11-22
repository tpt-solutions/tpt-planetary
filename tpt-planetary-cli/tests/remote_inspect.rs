//! `tptplan array inspect` on remote locations: an `http://` URL served by a
//! small local server, and the `s3://` and `https://` error paths.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

const EXIT_OK: i32 = 0;
const EXIT_USAGE: i32 = 2;
const EXIT_NOT_IMPLEMENTED: i32 = 3;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tpt-planetary-array/tests/fixtures/zarr")
}

fn tptplan(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_tptplan"))
        .args(args)
        .env_remove("TPT_S3_ENDPOINT")
        .output()
        .expect("run tptplan")
}

/// Serves files below `root` over HTTP/1.1 (one request per connection) from a
/// background thread. The thread ends with the test process.
fn serve(root: PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            let target = request
                .split_whitespace()
                .nth(1)
                .unwrap_or("/")
                .trim_start_matches('/')
                .to_string();
            let file = root.join(&target);
            let response = match fs::read(&file) {
                Ok(body) if file.is_file() => {
                    let mut out = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    out.extend_from_slice(&body);
                    out
                }
                _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_vec(),
            };
            let _ = stream.write_all(&response);
        }
    });
    format!("http://127.0.0.1:{port}")
}

#[test]
fn http_location_is_inspected_over_the_network() {
    let base = serve(fixture_root());
    let uri = format!("{base}/v2_none");
    let out = tptplan(&["--format", "json", "array", "inspect", &uri]);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(
        out.status.code(),
        Some(EXIT_OK),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("\"shape\""), "stdout: {stdout}");
    assert!(
        stdout.contains("float64") || stdout.contains("<f8"),
        "stdout: {stdout}"
    );
}

#[test]
fn http_location_that_is_not_an_array_is_a_runtime_error() {
    let base = serve(fixture_root());
    let out = tptplan(&["array", "inspect", &format!("{base}/no_such_array")]);
    assert_ne!(out.status.code(), Some(EXIT_OK));
    assert_ne!(out.status.code(), Some(EXIT_NOT_IMPLEMENTED));
}

#[test]
fn s3_without_an_endpoint_is_a_usage_error() {
    let out = tptplan(&["array", "inspect", "s3://bucket/temperature.zarr"]);
    assert_eq!(out.status.code(), Some(EXIT_USAGE));
    assert!(String::from_utf8_lossy(&out.stderr).contains("TPT_S3_ENDPOINT"));
}

#[test]
fn https_is_not_implemented() {
    let out = tptplan(&["array", "inspect", "https://example.org/temperature.zarr"]);
    assert_eq!(out.status.code(), Some(EXIT_NOT_IMPLEMENTED));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not supported in this build"));
}
