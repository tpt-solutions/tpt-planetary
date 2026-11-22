//! Writes Zarr v2 arrays compressed with Blosc through this crate, for the
//! interoperability check in `tests/fixtures/check_blosc_written.py`, which
//! reads them with zarr-python. Not part of the public API.
//!
//! Usage: `cargo run -p tpt-planetary-array --example write_blosc_zarr -- <out_dir>`
//! (the directory must not exist yet). Each array holds the values
//! `0.25 * row + 1.5 * col` in a 50 x 40 float64 grid, with 16 x 16 chunks, so the
//! edge chunks are partial.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use tpt_planetary_array::meta::{ArrayMetadata, CodecSpec, ZarrFormat};
use tpt_planetary_array::{Array, DType, FillValue, FsStore, Store};

const ROWS: u64 = 50;
const COLS: u64 = 40;

fn values() -> Vec<f64> {
    (0..ROWS)
        .flat_map(|r| (0..COLS).map(move |c| 0.25 * r as f64 + 1.5 * c as f64))
        .collect()
}

fn main() {
    let out: PathBuf = std::env::args()
        .nth(1)
        .expect("usage: write_blosc_zarr <out_dir>")
        .into();
    std::fs::create_dir_all(&out).expect("create output directory");
    let store: Arc<dyn Store> = Arc::new(FsStore::open(&out).expect("open store"));
    let cases = [
        ("lz4_noshuffle", "lz4", 0),
        ("lz4_shuffle", "lz4", 1),
        ("lz4_bitshuffle", "lz4", 2),
        ("zlib_shuffle", "zlib", 1),
        ("zstd_shuffle", "zstd", 1),
        ("zstd_bitshuffle", "zstd", 2),
    ];
    for (name, cname, shuffle) in cases {
        let config = json!({"cname": cname, "clevel": 5, "shuffle": shuffle, "blocksize": 0});
        let meta = ArrayMetadata::new(
            ZarrFormat::V2,
            vec![ROWS, COLS],
            vec![16, 16],
            DType::FLOAT64,
        )
        .expect("metadata")
        .with_fill_value(FillValue::Float(0.0))
        .with_codecs(vec![CodecSpec {
            name: "blosc".into(),
            config,
        }]);
        let array = Array::create(store.clone(), name, meta).expect("create");
        array.write_f64(&[], &values()).expect("write");
        println!("wrote {name}");
    }
}
