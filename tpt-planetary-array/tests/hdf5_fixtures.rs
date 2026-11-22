//! Reads the HDF5 fixtures written by h5py (tools/gen_hdf5_fixtures.py) and
//! compares every dataset with the values h5py wrote.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tpt_planetary_array::error::Error;
use tpt_planetary_array::hdf5::{Hdf5File, ObjectKind};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hdf5")
}

fn expected() -> serde_json::Map<String, Value> {
    let text = fs::read_to_string(fixture_dir().join("expected.json")).expect("expected values");
    match serde_json::from_str(&text).expect("valid JSON") {
        Value::Object(m) => m,
        _ => panic!("expected an object"),
    }
}

fn open(name: &str) -> Vec<u8> {
    fs::read(fixture_dir().join(name)).expect("fixture file")
}

#[test]
fn every_dataset_matches_h5py() {
    let files = ["basic.h5", "chunked.h5", "scales.h5", "latest.h5"];
    let bytes: Vec<Vec<u8>> = files.iter().map(|f| open(f)).collect();
    let mut checked = 0;
    for (path, values) in expected() {
        let want: Vec<f64> = values
            .as_array()
            .expect("array of values")
            .iter()
            .map(|v| v.as_f64().expect("number"))
            .collect();
        let got = files
            .iter()
            .zip(&bytes)
            .find_map(|(_, data)| {
                let file = Hdf5File::open(data).ok()?;
                let ds = file.dataset(&path).ok()?;
                Some(ds.read_f64())
            })
            .unwrap_or_else(|| panic!("{path} not found in any fixture"))
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        // serde_json's default float parser can be one ulp off, so compare with a
        // tolerance far below any layout error (which gives wrong values, not tiny ones).
        assert_eq!(got.len(), want.len(), "dataset {path}");
        for (g, w) in got.iter().zip(&want) {
            assert!(
                (g - w).abs() <= 1e-14 * w.abs().max(1.0),
                "dataset {path}: {g} vs {w}"
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 21, "every dataset in expected.json was checked");
}

#[test]
fn groups_datasets_and_shapes() {
    let data = open("basic.h5");
    let file = Hdf5File::open(&data).unwrap();
    assert_eq!(file.kind("/").unwrap(), ObjectKind::Group);
    assert_eq!(file.kind("/outer/inner").unwrap(), ObjectKind::Group);
    assert_eq!(file.kind("/vector").unwrap(), ObjectKind::Dataset);
    let mut root = file.list("/").unwrap();
    root.sort();
    assert_eq!(root, vec!["cube", "outer", "scalar", "vector"]);
    assert_eq!(file.dataset("/cube").unwrap().shape(), &[2, 3, 4]);
    assert_eq!(file.dataset("/scalar").unwrap().shape(), &[] as &[u64]);
    assert_eq!(file.dataset("/outer/inner/i16").unwrap().shape(), &[3, 4]);
    assert!(matches!(
        file.dataset("/outer"),
        Err(Error::InvalidSelection(_))
    ));
    assert!(matches!(file.dataset("/missing"), Err(Error::NotFound(_))));
}

#[test]
fn attributes_read_as_strings_and_numbers() {
    let data = open("basic.h5");
    let file = Hdf5File::open(&data).unwrap();
    let root = file.attributes("/").unwrap();
    let title = root.iter().find(|a| a.name == "title").expect("title");
    assert_eq!(title.as_str().unwrap(), "basic fixture");
    let count = root.iter().find(|a| a.name == "count").expect("count");
    assert_eq!(count.values_f64().unwrap(), vec![7.0]);
    let scale = file.attributes("/outer/inner").unwrap();
    assert_eq!(scale[0].values_f64().unwrap(), vec![2.5]);
    let vector = file.attributes("/vector").unwrap();
    let units = vector.iter().find(|a| a.name == "units").expect("units");
    assert_eq!(units.as_str().unwrap(), "m s-1");
    let valid = vector.iter().find(|a| a.name == "valid").expect("valid");
    assert_eq!(valid.shape, vec![2]);
    assert_eq!(valid.values_f64().unwrap(), vec![1.0, 2.0]);
}

#[test]
fn scales_file_has_dimension_scales_as_datasets() {
    let data = open("scales.h5");
    let file = Hdf5File::open(&data).unwrap();
    let temp = file.dataset("/temp").unwrap();
    assert_eq!(temp.shape(), &[4, 3]);
    // The dimension-scale attributes are references; their datasets still read.
    assert_eq!(
        file.dataset("/lat").unwrap().read_f64().unwrap(),
        vec![-30.0, 0.0, 30.0]
    );
    let attrs = file.attributes("/temp").unwrap();
    let units = attrs.iter().find(|a| a.name == "units").expect("units");
    assert_eq!(units.as_str().unwrap(), "K");
}

#[test]
fn version_two_headers_and_links() {
    let data = open("latest.h5");
    let file = Hdf5File::open(&data).unwrap();
    assert_eq!(file.list("/results/run1").unwrap(), vec!["values"]);
    let attrs = file.attributes("/").unwrap();
    assert_eq!(attrs[0].as_str().unwrap(), "latest");
    let n = file.attributes("/results/run1").unwrap();
    assert_eq!(n[0].values_f64().unwrap(), vec![3.0]);
    assert_eq!(file.dataset("/flags").unwrap().dtype().size, 1);
}

#[test]
fn chunk_index_versions_the_reader_does_not_support_are_reported() {
    let data = open("latest.h5");
    let file = Hdf5File::open(&data).unwrap();
    match file.dataset("/chunked_v4") {
        Err(Error::Unsupported(msg)) => assert!(msg.contains("layout version 4"), "{msg}"),
        other => panic!(
            "expected an unsupported-layout error, got {:?}",
            other.map(|d| d.shape().to_vec())
        ),
    }
}
