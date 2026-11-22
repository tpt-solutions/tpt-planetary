//! Reads Zarr v2 and v3 arrays written by zarr-python (see
//! `tests/fixtures/gen_zarr_fixtures.py`) and compares every value with the
//! expected bytes that zarr-python produced. This is the conformance check
//! for the codecs listed in the fixture generator, including Blosc with the
//! LZ4, Zstd, zlib and blosclz back ends, bit and byte shuffle, transpose,
//! partial writes that leave chunks missing, and a v3 shard with a gzip inner
//! chain.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tpt_planetary_array::array::Array;
use tpt_planetary_array::store::FsStore;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zarr")
}

/// Each fixture directory, with its expected-bytes file next to it.
fn fixtures() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(fixture_root()).expect("fixture directory") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let expected = fixture_root().join(format!("{name}.expected.raw"));
            out.push((name, expected));
        }
    }
    out.sort();
    out
}

#[test]
fn every_fixture_matches_zarr_python() {
    let fixtures = fixtures();
    assert!(fixtures.len() >= 15, "found {} fixtures", fixtures.len());
    let mut failures = Vec::new();
    for (name, expected_path) in &fixtures {
        let expected = fs::read(expected_path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", expected_path.display()));
        let store = Arc::new(FsStore::open(fixture_root().join(name)).expect("open store"));
        let outcome = (|| -> Result<Vec<u8>, String> {
            let array = Array::open(store.clone(), "").map_err(|e| format!("open: {e}"))?;
            let (_, bytes) = array.read_bytes(&[]).map_err(|e| format!("read: {e}"))?;
            Ok(bytes)
        })();
        match outcome {
            Ok(bytes) if bytes == expected => {}
            Ok(bytes) => failures.push(format!(
                "{name}: {} bytes read, {} expected, first difference at {:?}",
                bytes.len(),
                expected.len(),
                bytes.iter().zip(&expected).position(|(a, b)| a != b)
            )),
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    assert!(
        failures.is_empty(),
        "fixture mismatches:\n{}",
        failures.join("\n")
    );
}

#[test]
fn fixtures_cover_both_zarr_formats_and_blosc_back_ends() {
    let fixtures = fixtures();
    let names: Vec<&str> = fixtures.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.iter().any(|n| n.starts_with("v2_")));
    assert!(names.iter().any(|n| n.starts_with("v3_")));
    for backend in ["lz4", "zstd", "zlib", "blosclz"] {
        assert!(
            names
                .iter()
                .any(|n| n.contains(&format!("blosc_{backend}"))),
            "no blosc fixture with {backend}"
        );
    }
    assert!(names.iter().any(|n| n.contains("sharded")));
    assert!(names.iter().any(|n| n.contains("transpose")));
}
