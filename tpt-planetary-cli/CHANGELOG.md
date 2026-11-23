# Changelog

All notable changes to `tpt-planetary-cli` (the `tptplan` binary) are recorded here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The workspace-wide history is in the repository's
[CHANGELOG.md](https://github.com/tpt-solutions/tpt-planetary/blob/master/CHANGELOG.md).

## [Unreleased]

### Added

- The `tptplan` binary, built with `clap`, with global `--verbose` and
  `--format table|json` options.
- Shell completions (`completions`) and man pages (`man`).
- `array inspect` for Zarr arrays. Reads local directories, `http://` URLs and
  `s3://bucket/key` URLs (the last needs `TPT_S3_ENDPOINT`, an `http://`
  endpoint). `https://` exits with 3.
- `array math`: an expression over named Zarr inputs, written as a float64 Zarr
  array, or as a NetCDF-3 file when the output name ends in `.nc`.
- `array convert` between Zarr and NetCDF-3, for int8, int16, int32, float32 and
  float64 variables.
- `astro convert` for timestamps between UTC, TAI, TT and TDB (TDB through the
  full series, geocentre), and for coordinates
  between ICRS, Galactic and the mean ecliptic of J2000. `--epoch` supplies the
  TT epoch for the ecliptic-of-date frames.
- `ephemeris query` for the position of a body from an SPK kernel, as ICRS.
- Exit codes: 0 success, 1 runtime error, 2 usage error, 3 not implemented.
  `https://` locations, non-time frames and scales, and `ephemeris query`
  without `--kernel` exit with 3.
- Integration tests for the binary: `tests/cli.rs`, `tests/array_math.rs`,
  `tests/array_convert.rs` and `tests/remote_inspect.rs`. The end-to-end pipeline
  test `tests/dataflow.rs` runs the libraries rather than the binary.

### Known limits

- `ephemeris query` has no bundled kernel, and its output has not been validated
  against JPL Horizons.
- `array convert` does not apply CF time decoding to time variables.
- `https://` is not supported, because no TLS is built in.
