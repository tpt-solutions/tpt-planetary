# Changelog

All notable changes to `tpt-planetary-array` are recorded here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The workspace-wide history is in the repository's
[CHANGELOG.md](https://github.com/tpt-solutions/tpt-planetary/blob/master/CHANGELOG.md).

## [Unreleased]

### Added

- N-D shapes and C-order strides; dtypes (bool, signed and unsigned integers,
  floats, complex, fixed-length bytes and unicode), with endianness.
- Fill values, regular chunk grids, chunk-key encodings (default and v2),
  selections (ranges, steps, integer indices), and chunk-aligned region
  planning.
- Zarr v2 metadata (`.zarray`, `.zgroup`, `.zattrs`, consolidated `.zmetadata`)
  and Zarr v3 metadata (`zarr.json`).
- `Array` handle: create, open, chunk-level and selection-level reads and
  writes of bytes and `f64`, including partial-chunk writes.
- Groups: reading and writing node metadata, listing a hierarchy, and
  consolidating v2 metadata.
- Synchronous `Store` trait with `FsStore` (local directory) and `MemoryStore`
  (in memory) implementations.
- Codecs: gzip and zlib (`miniz_oxide`), LZ4 (`lz4_flex`), shuffle and delta
  filters, and a codec registry. Zarr v3 `bytes` codec endianness.
- Lazy expressions (`Expr`): loads from arrays, slicing, scalar add and
  multiply, elementwise arithmetic, comparison, unary operations, `where`, and
  valid-value masks.
- Chunk-streamed reductions with bounded block size: sum, count, mean, min, max
  and standard deviation (Welford), with NaN-aware and NaN-propagating variants,
  and axis-wise reductions.
- Broadcasting rules, rolling-mean and block-mean along one axis, and
  cos(latitude) area weighting with `weighted_mean`.
- NetCDF-3 reader and writer for Classic (CDF-1) and 64-bit offset (CDF-2)
  files, with dimensions, attributes and record variables.
- CF time units (seconds, minutes, hours, days) and decoding and encoding for
  the `standard`, `proleptic_gregorian` and `noleap` calendars.
- Zarr v3 sharding: index parsing with and without CRC-32C, and index-driven
  partial reads. When the chunk pipeline is a single `sharding_indexed` codec,
  a read fetches the index and only the inner chunks it needs, merging nearby
  ranges into one request.
- Blosc decoding for LZ4, zlib, zstd, blosclz and snappy, with byte and bit
  shuffle and split blocks.
- Blosc encoding for LZ4, zlib and zstd, with byte and bit shuffle, the same
  split rule as decoding, and a memcpy fallback.
- Stores: `HttpStore` (plain HTTP with range requests), `S3Store` (path-style
  S3 over HTTP with SigV4 signing and paged listing), `LimitedStore` (bounds
  concurrent operations), `Store::get_range` and `Store::size`, and an
  `AsyncStore` trait with a blanket impl for synchronous stores and a minimal
  `block_on`. No TLS is built in, so `https://` is refused.
- Byte-budgeted LRU chunk cache (`cache`) and a retry-with-backoff wrapper
  (`RetryStore`, `retry`) for transiently failing stores.
- CF calendars `360_day`, `all_leap`, `366_day` and `julian`, and
  `calendar_month_labels` for grouping by month.
- `Expr::group_mean`: means over caller-supplied integer labels along an axis.
- Tabular bridge behind the `strata` feature: one-dimensional Zarr arrays as
  `tpt-strata` columns (NaN becomes NULL), and CSV output. Parquet is not used,
  because its dependencies are Apache-2.0 only.
- Parallel block evaluation behind the `threads` feature (`EvalOptions::workers`),
  and vectorised kernels behind the `simd` feature (`tpt-simd-vector`).
- Zarr fixtures from zarr-python 3.4 and numcodecs, and NetCDF-3 fixtures from
  netCDF4. NetCDF-3 interop is checked against files read by xarray. The
  generators are in `tests/fixtures/` (Python, run by hand).

### Changed

- CF calendars `360_day`, `all_leap` and `julian` were rejected in the first
  draft; they are accepted now.
- The Blosc don't-split flag is 0x10 (it was read as 0x08 in the first draft).

### Fixed

- The Zarr v2 writer puts the codec name under `id` in the compressor and filter
  objects. Before this, zarr-python could not read compressed arrays that this
  crate wrote.
- Chunk offsets in array geometry saturate instead of wrapping on overflow.

### Known limits

- No TLS, so `https://` locations are refused.
- No NetCDF-4, HDF5 or CDF-5 support.
- `HttpStore` is read-only.
