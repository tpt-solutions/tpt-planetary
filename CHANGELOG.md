# Changelog

All notable changes to this project are recorded here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

#### Workspace and project

- Cargo workspace with crates `tpt-planetary-core` (`no_std`),
  `tpt-planetary-array`, `tpt-planetary-astro`, `tpt-planetary-cli` (`tptplan`)
  and `bindings/tpt-planetary-py` (scaffold only; the Python bindings are in
  progress and not usable yet).
- Dual `MIT OR Apache-2.0` licence files.
- `deny.toml` licence, ban and source policy for `cargo deny`.
- CI workflow for build, test, fmt, clippy, docs, MSRV, `no_std` and `cargo deny`.
- Architecture decision records ADR-001 to ADR-003 in `docs/adr/`.
- User guide in `docs/guide/`: getting started, time and frames, arrays, the
  command line, and status and limitations.

#### tpt-planetary-core

- `no_std` crate with `alloc` and `std` features. The error type works without
  `std`.
- Physical and astronomical constants, each with its cited source. Several
  values were written from memory and are not yet validated (see the status
  page).
- Two-part Julian Date and Modified Julian Date types.
- Gregorian and Julian calendar conversion, using Meeus's algorithm in integer
  arithmetic.
- Typed time scales `Utc`, `Tai` and `Tt` through `Epoch<S>`, with conversions
  that take a leap-second table.
- Embedded leap-second table `EMBEDDED` (version 1; 28 entries from 1972 to
  2017; valid until 2027-06-28). Conversions after the expiry return
  `LeapSecondTableExpired`.
- Spherical helpers: angle newtypes (radians, degrees, hour angle),
  spherical and Cartesian conversion, angular separation, position angle,
  destination, great-circle interpolation and cross-track angle.
- Rotations as unit quaternions on `glam`, with Euler (ZYX) and axis-angle
  conversion, and SLERP.
- WGS84 reference ellipsoid: geodetic and geocentric latitude and Earth-centred
  Cartesian conversion. Vincenty and Karney inverse formulas are not
  implemented.

#### tpt-planetary-array

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

#### tpt-planetary-astro

- TT to TDB and back, using the three leading terms of the Fairhead and
  Bretagnon series (the full series is not implemented).
- Earth rotation angle, GMST (IAU 2006) and GAST.
- Mean obliquity of the ecliptic (IAU 2006).
- `finals2000A` EOP parser and `EopTable` with UT1-UTC and polar-motion
  interpolation. The column map was written from memory and is unverified.
- Typed sky coordinates `SkyCoord<F>` with ICRS, Galactic, mean ecliptic of
  J2000, true ecliptic (caller-supplied obliquity) and Alt/Az frames, and the
  transforms between them that are implemented.
- NAIF DAF/SPK reader for byte slices, Chebyshev evaluation of position and
  velocity, segment lookup, one-iteration light-time correction, and the
  Earth-Sun distance helpers (from a kernel, and an analytic low-precision
  series in AU).
- FITS WCS header parser, linear pixel transform, TAN, SIN and AIT projections,
  and LONPOLE and LATPOLE rotation.

#### tpt-planetary-cli

- `tptplan` binary built with `clap`, with global `--verbose` and
  `--format table|json`, shell completions (`completions`) and man pages
  (`man`).
- `array inspect` for local Zarr arrays.
- `astro convert` for timestamps between UTC, TAI and TT.
- `ephemeris query` (requires a kernel path; the successful query path is not
  yet validated).
- Exit codes: 0 success, 1 runtime error, 2 usage error, 3 not implemented.
  `array math`, `array convert`, remote array locations, non-time frames and
  scales, and `ephemeris query` without `--kernel` exit with 3.
- Integration tests for the binary (`tests/cli.rs`).

#### Tests and documentation

- End-to-end test of the spec section 7 data-flow example, scaled down
  (`tpt-planetary-cli/tests/dataflow.rs`): a synthetic Zarr dataset, a lazy
  box mean with cos(latitude) weights, chunk-access checks, a UTC to TT to TDB
  conversion, an Earth-Sun distance check, and a round-trip of the 2-D result.
