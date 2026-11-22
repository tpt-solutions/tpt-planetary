//! Micro-benchmarks for the array engine: Zarr v2 chunk reads, lazy
//! reductions and byte-level codec round trips.
//!
//! All data is generated in memory. These measure speed only.

use std::sync::Arc;

use divan::black_box;
use tpt_planetary_array::Array;
use tpt_planetary_array::codec::Codec;
use tpt_planetary_array::codec::compress::{GzipCodec, Lz4Codec};
use tpt_planetary_array::dtype::{DType, Endian, Kind};
use tpt_planetary_array::lazy::Expr;
use tpt_planetary_array::meta::{ArrayMetadata, ZarrFormat};
use tpt_planetary_array::selection::{Selection, Slice};
use tpt_planetary_array::store::{MemoryStore, Store};

const N: u64 = 1000;

fn main() {
    divan::main();
}

/// Builds a 1000x1000 float64 Zarr v2 array in memory, filled with a
/// deterministic ramp. Chunks are 250x250 so a full read touches 16 chunks.
fn filled_array() -> Array {
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let dtype = DType::new(Kind::Float(64), Endian::Little).expect("float64");
    let meta =
        ArrayMetadata::new(ZarrFormat::V2, vec![N, N], vec![250, 250], dtype).expect("metadata");
    let array = Array::create(store, "field", meta).expect("create");
    let values: Vec<f64> = (0..N * N).map(|i| i as f64 * 0.001).collect();
    let full = [
        Selection::Range(Slice::range(0, N as i64)),
        Selection::Range(Slice::range(0, N as i64)),
    ];
    array.write_f64(&full, &values).expect("write");
    array
}

/// Reads one 250x250 float64 chunk (500 KB decoded) from the store.
#[divan::bench]
fn zarr_v2_read_one_chunk(bencher: divan::Bencher) {
    let array = filled_array();
    bencher.bench_local(|| array.read_chunk(black_box(&[1, 2])).expect("read"));
}

/// Reads the whole 1000x1000 float64 array as f64 values.
#[divan::bench]
fn zarr_v2_read_full_f64(bencher: divan::Bencher) {
    let array = filled_array();
    let full = [
        Selection::Range(Slice::range(0, N as i64)),
        Selection::Range(Slice::range(0, N as i64)),
    ];
    bencher.bench_local(|| array.read_f64(black_box(&full)).expect("read"));
}

/// Lazy, chunk-streamed mean over the whole 1000x1000 array.
#[divan::bench]
fn lazy_mean_full_array(bencher: divan::Bencher) {
    let array = filled_array();
    let store = array.store().clone();
    bencher
        .with_inputs(|| Array::open(store.clone(), "field").expect("open"))
        .bench_values(|a| Expr::from_array(a).mean().expect("mean"));
}

/// Lazy `a * 2 + 1` then mean over 1,000,000 in-memory f64 values: elementwise
/// arithmetic and a full reduction, with no chunk reads. Run once without and
/// once with `--features simd` to compare the scalar and vectorised paths.
#[divan::bench]
fn lazy_scale_shift_mean_in_memory_1m(bencher: divan::Bencher) {
    let values: Vec<f64> = (0..N * N).map(|i| i as f64 * 0.001).collect();
    bencher
        .with_inputs(|| Expr::from_values(vec![N * N], values.clone()).expect("values"))
        .bench_values(|e| e.mul_scalar(2.0).add_scalar(1.0).mean().expect("mean"));
}

/// Deterministic 1 MiB payload that is neither all zeros nor random.
fn payload_1mib() -> Vec<u8> {
    (0..(1usize << 20)).map(|i| ((i / 8) % 251) as u8).collect()
}

/// gzip (level 6) encode and decode of 1 MiB.
#[divan::bench]
fn codec_gzip_round_trip_1mib(bencher: divan::Bencher) {
    let data = payload_1mib();
    let codec = GzipCodec::new(6);
    bencher.bench_local(|| {
        let enc = codec.encode(black_box(&data)).expect("encode");
        codec.decode(black_box(&enc)).expect("decode")
    });
}

/// LZ4 block encode and decode of 1 MiB.
#[divan::bench]
fn codec_lz4_round_trip_1mib(bencher: divan::Bencher) {
    let data = payload_1mib();
    let codec = Lz4Codec;
    bencher.bench_local(|| {
        let enc = codec.encode(black_box(&data)).expect("encode");
        codec.decode(black_box(&enc)).expect("decode")
    });
}

/// zstd (level 1, the only level the pure-Rust encoder uses) encode and decode of 1 MiB.
#[divan::bench]
fn codec_zstd_round_trip_1mib(bencher: divan::Bencher) {
    let data = payload_1mib();
    let codec = tpt_planetary_array::codec::compress::ZstdCodec::new(1);
    bencher.bench_local(|| {
        let enc = codec.encode(black_box(&data)).expect("encode");
        codec.decode(black_box(&enc)).expect("decode")
    });
}
