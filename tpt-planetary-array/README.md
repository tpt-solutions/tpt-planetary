# tpt-planetary-array

Chunked N-dimensional arrays for [tpt-planetary](https://github.com/tpt-solutions/tpt-planetary):
Zarr v2 and v3 reading and writing, a set of pure-Rust codecs, storage backends
from memory and local disk to plain-HTTP and S3 endpoints, NetCDF-3 interop, and
a lazy expression engine whose reductions run in bounded memory.

It is the array layer of the project. The aim is to replace NetCDF-C and HDF5
for planetary-scale data without any C library. The crate has no FFI.

**Status:** version 0.1.0, not yet published to crates.io. The API may change
before 1.0. Local Zarr and NetCDF-3 are the best-tested paths. The network
stores are tested against local servers, not against a production object store.

## Contents

- [What it provides](#what-it-provides)
- [Installation](#installation)
- [Quick start](#quick-start)
- [Stores](#stores)
- [Codecs](#codecs)
- [Lazy expressions and reductions](#lazy-expressions-and-reductions)
- [NetCDF-3](#netcdf-3)
- [Feature flags](#feature-flags)
- [Limitations](#limitations)
- [Where this crate fits](#where-this-crate-fits)
- [Development](#development)
- [Changelog](CHANGELOG.md)
- [Licence](#licence)

## What it provides

| Area | Modules | Contents |
| --- | --- | --- |
| Data model | `dtype`, `fill`, `grid`, `selection` | N-D shapes and C-order strides; element types (bool, signed and unsigned integers, floats, complex, fixed-length bytes and unicode) with endianness; fill values; regular chunk grids; chunk-key encodings; ranges, steps and integer indices. |
| Metadata | `meta` | Zarr v2 (`.zarray`, `.zgroup`, `.zattrs`, consolidated `.zmetadata`) and Zarr v3 (`zarr.json`). |
| Arrays and groups | `array`, `hierarchy`, `plan` | The `Array` handle: create, open, chunk-level and selection-level reads and writes of bytes and `f64`, including partial-chunk writes. Group metadata, hierarchy listing and v2 consolidation. Chunk-aligned region planning. |
| Stores | `store`, `cache`, `retry` | The synchronous `Store` trait and its implementations (see [Stores](#stores)), an optional byte-budgeted LRU chunk cache, and a retry-with-backoff wrapper. |
| Codecs | `codec` | See [Codecs](#codecs). |
| Lazy evaluation | `lazy`, `ops` | The `Expr` graph and bounded-memory reductions (see [Lazy expressions](#lazy-expressions-and-reductions)). |
| NetCDF-3 | `netcdf3` | Reader and writer for Classic (CDF-1) and 64-bit offset (CDF-2) files. |
| Tabular bridge | `strata` (feature) | One-dimensional arrays as columns of `tpt-strata` tables, and CSV output. |

## Installation

Once published, add the crate with Cargo:

```toml
[dependencies]
tpt-planetary-array = "0.1"
```

Until then, depend on it by path from a checkout of the repository. The minimum
supported Rust version is 1.95. The edition is 2024.

## Quick start

Create a float64 array in memory, write it, and read a slice back. This is the
crate-level example, which is compiled as a doctest:

```rust
use tpt_planetary_array::{Array, MemoryStore, Selection, Slice};
use tpt_planetary_array::meta::{ArrayMetadata, ZarrFormat};
use tpt_planetary_array::{DType, FillValue};
use std::sync::Arc;

let store = Arc::new(MemoryStore::new());
let meta = ArrayMetadata::new(ZarrFormat::V2, vec![4, 6], vec![2, 3], DType::FLOAT64)
    .unwrap()
    .with_fill_value(FillValue::Float(f64::NAN));
let array = Array::create(store.clone(), "temperature", meta).unwrap();

let values: Vec<f64> = (0..24).map(f64::from).collect();
array.write_f64(&[Selection::ALL, Selection::ALL], &values).unwrap();
```

To open an existing local Zarr array, use `FsStore` with `Array::open`. The
`tptplan array inspect` command in [tpt-planetary-cli](../tpt-planetary-cli/)
prints the same metadata from the shell.

## Stores

Every store implements the synchronous `Store` trait: keyed byte reads and
writes, with `get_range` and `size` for partial reads. The implementations:

| Store | Feature | Reads | Writes | Notes |
| --- | --- | --- | --- | --- |
| `MemoryStore` | `std` | yes | yes | In-memory, for tests and temporary data. |
| `FsStore` | `std` | yes | yes | A local directory. |
| `HttpStore` | `std` | yes | no | Plain `http://` with HTTP range requests. Refuses `https://`: no TLS is built in. |
| `S3Store` | `std` | yes | yes | Path-style S3 over plain HTTP, with SigV4 signing and paged listing. Use it with a local or private endpoint such as MinIO. |
| `LimitedStore` | `std` | passes through | passes through | Bounds the number of concurrent operations on an inner store. |
| `RetryStore` | none | passes through | passes through | Retries transient failures with a `RetryPolicy` and backoff. |

`AsyncStore` (in `store::async_store`) is a trait with a blanket implementation
for every synchronous store, plus a minimal `block_on`. No async runtime is
added as a dependency.

## Codecs

Pure Rust throughout. Codecs are chosen through the metadata and resolved by
the codec registry in `codec`.

- Compression: gzip and zlib (`miniz_oxide`), LZ4 (`lz4_flex`), Zstandard
  (`ruzstd`).
- Filters and transforms: shuffle, bit-shuffle, delta and transpose.
- Blosc: decoding for LZ4, zlib, zstd, blosclz and snappy, with byte and bit
  shuffle and split blocks. Encoding for LZ4, zlib and zstd, with a memcpy
  fallback. The far-distance blosclz form is rejected.
- Zarr v3 `bytes` (endianness) and `crc32c` checksums.
- Zarr v3 `sharding_indexed`: index parsing with and without CRC-32C. When the
  pipeline is a single sharding codec, a read fetches the index and only the
  inner chunks it needs, merging nearby ranges into one request.

Blosc arrays written by this crate are read by zarr-python for the six codec and
shuffle combinations checked in the workspace tests. The Zarr v2 writer puts the codec name under `id`
in compressor and filter objects, as zarr-python expects.

## Lazy expressions and reductions

`Expr` describes a computation over arrays without running it. Evaluation runs
chunk by chunk with a bounded block size, so memory use does not grow with the
array size.

- Loads from arrays, slicing, scalar add and multiply, elementwise arithmetic,
  comparison, unary operations, `where`, and valid-value masks.
- Broadcasting rules, rolling mean and block mean along one axis.
- Reductions: sum, count, mean, min, max and standard deviation (Welford), each
  with NaN-aware and NaN-propagating variants, and axis-wise forms.
- `weighted_mean` with cos(latitude) area weighting.
- `group_mean` over caller-supplied integer labels along an axis.

`EvalOptions` sets `block_elements`, the maximum size of one evaluated block,
and `workers`, the number of blocks evaluated concurrently. Only the `threads`
feature uses more than one worker.

## NetCDF-3

The reader and writer handle Classic (CDF-1) and 64-bit offset (CDF-2) files,
with dimensions, attributes and record variables. CF time units (seconds,
minutes, hours, days) are decoded and encoded for the `standard`,
`proleptic_gregorian`, `noleap`, `360_day`, `all_leap`, `366_day` and `julian`
calendars. `calendar_month_labels` groups time offsets by calendar month.

Interoperability is checked against files written by netCDF4 and read by xarray,
and against fixtures generated by SciPy's `netcdf` module. The fixtures are in
`tests/fixtures/`.

## Feature flags

| Feature | Default | Effect |
| --- | --- | --- |
| `std` | yes | Enables the in-memory, local-filesystem and HTTP/S3 stores, and `std::error` integration. Without it the crate is `no_std` with `alloc`. |
| `strata` | no | Tabular bridge to `tpt-strata` (MIT OR Apache-2.0, no dependencies). Parquet is not used: its Apache-2.0-only dependencies are outside the licence policy. Implies `std`. |
| `simd` | no | Vectorised elementwise and reduction kernels through `tpt-simd-vector` (rust-version 1.95). The scalar code is the reference. |
| `threads` | no | Evaluates lazy-expression blocks in parallel with `std::thread::scope`. The serial path is the reference. Implies `std`. |

Build without the standard library:

```sh
cargo build -p tpt-planetary-array --no-default-features
```

## Limitations

- **No TLS.** `https://` locations are refused. Use a plain-HTTP endpoint or a
  local store.
- **Read-only HTTP.** `HttpStore` cannot write or delete. `S3Store` can, but its
  tests run against a local test server, not a production S3 service.
- **Not implemented:** NetCDF-4 and HDF5, CDF-5, and async I/O built on a
  runtime. The `AsyncStore` trait exists, but no async network backend is wired
  up.
- **Validation.** Zarr round-trips are tested within this crate. Interop with
  zarr-python and NetCDF is tested against the fixtures listed above, not against
  every codec and version combination.
- **Performance.** Benchmarks against xarray, netCDF4 and NumPy are not yet
  published. The `divan` benchmark in `benches/array.rs` measures this crate
  alone.

## Where this crate fits

```text
tpt-planetary-core        constants, time scales, spherical math   (no_std)

tpt-planetary-array       chunked N-D arrays, Zarr, lazy engine   <-- this crate
        |
        +--> tpt-planetary-cli    `tptplan array ...`
        +--> bindings/tpt-planetary-py   ZarrArray and open_zarr
```

This crate does not depend on `tpt-planetary-core`. It is independent of the
astronomy layer, so it can be used for other gridded data.

## Development

From the repository root:

```sh
cargo test -p tpt-planetary-array
cargo test -p tpt-planetary-array --all-features
cargo run -p tpt-planetary-array --example write_blosc_zarr
cargo bench -p tpt-planetary-array
```

The integration tests are in `tests/`: `zarr_conformance.rs` and
`netcdf3_interop.rs`. The fixture generators are in `tests/fixtures/` and are
Python scripts that are run by hand.

The project-wide documentation is in the
[user guide](https://github.com/tpt-solutions/tpt-planetary/blob/master/docs/guide/arrays.md),
the [architecture decision records](https://github.com/tpt-solutions/tpt-planetary/tree/master/docs/adr),
and the [dependency audit notes](https://github.com/tpt-solutions/tpt-planetary/blob/master/docs/dependencies.md).

## Licence

Dual-licensed under either of

- MIT licence ([LICENSE-MIT](https://github.com/tpt-solutions/tpt-planetary/blob/master/LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](https://github.com/tpt-solutions/tpt-planetary/blob/master/LICENSE-APACHE))

at your option. Unless you state otherwise, any contribution you submit is
dual-licensed in the same way.
