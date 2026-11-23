# Changelog

All notable changes to the `tpt-planetary` Python package are recorded here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The workspace-wide history is in the repository's
[CHANGELOG.md](https://github.com/tpt-solutions/tpt-planetary/blob/master/CHANGELOG.md).

## [Unreleased]

### Added

- The `tpt_planetary` package, built with PyO3 and maturin as the abi3 extension
  `tpt_planetary._core` (CPython 3.9 and newer, one wheel per platform).
- `JulianDate` for two-part Julian Dates: construction from a calendar date or
  from (day, fraction), conversion to calendar and MJD, day arithmetic, and
  comparison.
- Time-scale conversions between UTC, TAI and TT: `utc_to_tai`, `tai_to_utc`,
  `tai_to_tt`, `tt_to_tai`, `utc_to_tt` and `tt_to_utc`.
- `leap_second_table_info` and `leap_second_table_expired`, to inspect the
  embedded leap-second table.
- Sky functions: `angular_separation`, `icrs_to_galactic` and `galactic_to_icrs`,
  for NumPy arrays of degrees.
- `earth_rotation_angle` and `gmst_iau2006`.
- `Wcs`: FITS-style WCS from a header mapping, with `pixel_to_world`,
  `world_to_pixel` and `crval`, for TAN, SIN and AIT projections.
- `open_zarr` and `ZarrArray`: read a local Zarr v2 or v3 array as float64 NumPy
  data, and reduce it with `sum`, `mean`, `min` and `max`, streamed through
  chunks with the GIL released.
- `TptPlanetaryError`, raised for errors other than bad input or I/O.

### Known limits

- Remote stores (S3, HTTP), asynchronous I/O and NetCDF-3 opening are not
  exposed.
- TDB, SPK ephemerides, Alt/Az and ecliptic frames, and UT1 are not exposed.
- Writing arrays from Python is not supported.
- The leap-second table expires on 2027-06-28, after which UTC conversions raise
  `TptPlanetaryError`.
- No wheels are published yet.
