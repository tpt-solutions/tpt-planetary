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

- TT to TDB and back, using the full 787-term Fairhead and Bretagnon series
  with the JPL-mass terms (geocentre; matches `erfa.dtdb`).
- Herrick-Gibbs velocity from three closely spaced positions
  (`orbit::herrick_gibbs_velocity`).
- Read-only HDF5 subset in `tpt-planetary-array` (`hdf5`): groups, datasets and
  attributes of files written with the default HDF5 format, with deflate, shuffle
  and fletcher32 filters. Checked against h5py fixtures.
- Astrometric directions from an SPK kernel (`ephemeris::astrometric`, and
  `tptplan ephemeris query --astrometric`), with a converged light time. Checked
  against JPL Horizons at five epochs (0.015 arcsec maximum).
- Climate-index pipelines on lazy fields (`ops::climate`): monthly anomaly,
  area-weighted box means, the Niño-3.4 index and its relative form.
- Read-only HDF5 subset, and blosclz far back-references (distances above 8191).
- FITS binary tables: `Hdu::table` reads the fixed-width column forms with
  scaling and nulls, checked against astropy.
- FITS binary table writer (`fits::write_bintable`) for the same fixed-width
  forms, with TSCAL, TZERO and TNULL; checked by a round trip and by astropy.
- FITS ASCII table writer (`fits::write_ascii_table`) for A, I, Fw.d and Ew.d fields,
  with TSCAL, TZERO and TNULL; checked by a round trip and by astropy.
- FITS tile-compressed integer images, RICE_1, GZIP_1 and GZIP_2: `Hdu::compressed_values`, checked
  against astropy.
- FITS bit, complex and variable-length binary columns: `read_bits`, `read_complex`
  and `read_vla` (with the heap and THEAP), checked against astropy.
- FITS ASCII tables: `Hdu::ascii_table` reads `XTENSION = 'TABLE'` columns of A, I,
  F, E and D fields with scaling and TNULL, checked against astropy.
- TCG and TCB time scales (`time::tt_to_tcg`, `tdb_to_tcb` and their inverses), checked
  against pyerfa; `tptplan astro convert` accepts `tcg` and `tcb`.
- CK attitude kernels, type 2 (`ck::CkFile`): the C-matrix and angular velocity of an
  instrument at an SCLK time, checked against SPICE's `ckgpav`.
- Apparent directions from an SPK kernel (`ephemeris::apparent_direction`, and
  `tptplan ephemeris query --apparent`): light deflection by the Sun and annual
  aberration on the astrometric direction. Checked against Horizons apparent
  positions at five epochs (0.11 arcsec maximum).
- `tptplan array index` for the Niño-3.4 indices of a local Zarr field.
- Python `nino34` and `nino34_relative` on a `ZarrArray`, computed lazily with
  the GIL released and checked against a NumPy computation.
- Apparent directions (`tptplan ephemeris query --apparent`), checked against
  Horizons at five epochs.
- UT1 in `tptplan astro convert` (`--eop FILE`, an IERS finals2000A file), checked
  against astropy's IERS-B values; interpolation across a gap of more than a day is
  an error.
- `TransformContext::from_utc_and_eop`: TT, UT1 and polar motion for a UTC epoch from
  the leap-second table and an EOP table.
- IERS C04 time series (`eopc04`) read by `parse_c04`, and `parse_eop_text` picks
  C04 or finals2000A from the file header; `tptplan astro convert --eop` accepts both.
  Checked on a 2017 excerpt against the finals2000A rows for the same dates.
- Binary PCK orientation (`pck::orientation_matrix`): the J2000 to ITRF93 rotation from
  a type-2 PCK, checked against SPICE's pxform.
- IAU 2000B nutation (`nutation::nutation_angles_2000b`), from ERFA's `nut00b`
  coefficients; matches `erfa.nut00b` to 1e-13 rad.
- Atmospheric refraction for Alt/Az: the ERFA `refco` constants
  (`frames::refraction_coefficients`) and `frames::icrs_to_altaz_refracted_at`.
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
- `array math` (lazy expressions over named Zarr inputs) writes a Zarr array,
  or a NetCDF-3 file when the output name ends in `.nc`.
- `array convert` between Zarr and NetCDF-3.
- `array inspect` also reads `http://` URLs and `s3://bucket/key` (the latter
  needs `TPT_S3_ENDPOINT`, an http:// endpoint).
- `astro convert` for the mean and true ecliptic of date, with `--epoch`.
- Exit codes: 0 success, 1 runtime error, 2 usage error, 3 not implemented.
  `https://` locations, non-time frames and scales, and `ephemeris query`
  without `--kernel` exit with 3.
- Integration tests for the binary (`tests/cli.rs`, `tests/array_math.rs`,
  `tests/array_convert.rs`, `tests/remote_inspect.rs`).

#### Added after the first draft

- Zarr v3 sharding: index parsing with and without CRC-32C, and index-driven
  partial reads. When the chunk pipeline is a single `sharding_indexed` codec,
  a read fetches the index and only the inner chunks it needs, merging nearby
  ranges into one request.
- Blosc decoding for LZ4, zlib, zstd, blosclz and snappy, with byte and bit
  shuffle and split blocks. The far-distance blosclz form is rejected.
- Stores: `HttpStore` (plain HTTP with range requests), `S3Store` (path-style
  S3 over HTTP with SigV4 signing and paged listing), `LimitedStore` (bounds
  concurrent operations), `Store::get_range` and `Store::size`, and an
  `AsyncStore` trait with a blanket impl for synchronous stores and a minimal
  `block_on`. No TLS is built in, so `https://` is not supported.
- CF calendars `360_day`, `all_leap`, `366_day` and `julian`, and
  `calendar_month_labels` for grouping by month.
- `Expr::group_mean`: means over caller-supplied integer labels along an axis.
- NetCDF-3 interop checked against files written by netCDF4 and read by xarray.
- WCS projections ARC, ZEA, STG, CAR, MOL and HPX, SIP distortion with the
  inverse, and FITS image reading and writing for primary and IMAGE HDUs.
- Orbits: Keplerian elements, Kepler's equation, TLE parsing and the near-Earth
  SGP4 model (WGS-72), with the SDP4 deep-space terms for orbits of 225 minutes
  or more (checked against the sgp4 package: 1.2e-9 km over 226 states).
- Frames: the mean ecliptic of date (IAU 2006 precession), the true ecliptic of
  date (with nutation), and the nutation chain with the frame bias.
- Astrometry: annual aberration (checked against ERFA), proper motion along the
  great circle, and parallax. Light deflection by the Sun is not implemented.
- Vincenty's inverse and direct geodesics on the ellipsoid.
- Reference data: Zarr fixtures from zarr-python 3.4 and numcodecs, NetCDF-3
  fixtures from netCDF4, and aberration values from pyerfa 2.0.1.5. The
  generators are in `tests/fixtures/` (Python, run by hand).
- Notices: `THIRD-PARTY-NOTICES.md` generated by cargo-about (`docs/notices.md`).
- Light deflection by the Sun for a source at infinity (checked against ERFA
  `ld` to 3e-8 arcseconds), with the observer's heliocentric position from a
  Keplerian model.
- Gibbs velocity from three positions on one orbit (`orbit::gibbs_velocity`).
- Scope decisions for 0.1 (`docs/adr/ADR-004-scope-decisions-for-0.1.md`):
  reference data is user-supplied, FK4 and FK5 are out of scope, no async
  runtime is added and Vincenty only. (The three-term TDB decision in that
  record was later replaced by the full series.)
- The Python validation suite runs (321 passed, 3 expected failures) with
  astropy, sgp4 and numpy.

#### Added (Blosc encode, writer fix)

- Blosc encoding for LZ4, zlib and zstd, with byte and bit shuffle, the same
  split rule as decoding, and a memcpy fallback. zarr-python reads the arrays
  written by this crate (checked for six codec and shuffle combinations).
- Gibbs velocity from three positions (listed above).

#### Added (tabular bridge)

-  feature of : one-dimensional Zarr arrays as
  tpt-strata columns (NaN becomes NULL), and CSV output. Replaces the Parquet
  interchange, which is not used because of its Apache-2.0-only dependencies.

#### Added (TPV)

- TPV distortion for WCS headers (`CTYPE = 'RA---TPV'`), with the 40-term
  polynomials `PV1_k` and `PV2_k` and inversion for world-to-pixel. Checked
  against wcslib through astropy.

#### Fixed (latest)

- The Zarr v2 writer now puts the codec name under `id` in the compressor and
  filter objects. Before this, zarr-python could not read compressed arrays
  that this crate wrote.

- WCS CAR longitudes outside [-180, 180] degrees are invalid, as in WCSLIB.
- WCS LATPOLE selects the native pole root nearest to its value; it is no longer
  used directly, so CRVAL maps to itself.

#### Changed

- `icrs_to_ecliptic_at_epoch` is renamed `icrs_to_ecliptic_epoch_obliquity`.
  It keeps the J2000 equinox and is not the mean ecliptic of date.
- The Blosc don't-split flag is 0x10 (it was read as 0x08 in the first draft).
- CF calendars `360_day`, `all_leap` and `julian` were rejected in the first
  draft; they are accepted now.

#### Fixed

- FITS `uint64` values with `BZERO = 2^63` lost small values to a double
  rounding; they are now converted once.
- Chunk offsets in array geometry saturate instead of wrapping on overflow.
- WCS right ascension that rounds to 360 is folded to 0.

#### Tests and documentation

- End-to-end test of the spec section 7 data-flow example, scaled down
  (`tpt-planetary-cli/tests/dataflow.rs`): a synthetic Zarr dataset, a lazy
  box mean with cos(latitude) weights, chunk-access checks, a UTC to TT to TDB
  conversion, an Earth-Sun distance check, and a round-trip of the 2-D result.
