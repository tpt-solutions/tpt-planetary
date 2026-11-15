# tpt-planetary (Python)

PyO3 bindings for the tpt-planetary Rust crates: two-part Julian Dates and
UTC/TAI/TT conversion, ICRS/Galactic transforms, ERA/GMST, FITS-style WCS
(TAN, SIN, AIT), and reads and lazy reductions on local Zarr arrays.

This package is MIT OR Apache-2.0. It is not published to PyPI yet.

## Build from source

Requires Rust 1.85 or newer, Python 3.9 or newer and
[maturin](https://www.maturin.rs/).

```sh
pip install maturin numpy
maturin develop          # editable install into the active virtualenv
pytest tests -q
```

Or build a wheel with `maturin build --release` and install it with `pip`.
The wheel is `abi3-py39`, so one wheel per platform serves CPython 3.9 and newer.

## Quick example

```python
import numpy as np
import tpt_planetary as tp

jd = tp.JulianDate.from_calendar(2000, 1, 1, 43_200.0)   # J2000.0, UTC
tai = tp.utc_to_tai(jd)

l, b = tp.icrs_to_galactic(np.array([266.40498829]), np.array([-28.93617776]))

wcs = tp.Wcs.from_header({
    "CRPIX1": 1.0, "CRPIX2": 1.0, "CRVAL1": 180.0, "CRVAL2": 60.0,
    "CTYPE1": "RA---TAN", "CTYPE2": "DEC--TAN",
    "CDELT1": -0.001, "CDELT2": 0.001,
})
ra, dec = wcs.pixel_to_world(np.array([1.0]), np.array([1.0]))

arr = tp.open_zarr("data/temperature.zarr")   # local directory, Zarr v2 or v3
print(arr.shape, arr.dtype, arr.chunks)
block = arr.read((slice(0, 10), slice(None)))  # float64 ndarray
print(arr.mean(), arr.min(), arr.max(), arr.sum())
```

## API summary

| Area | Names |
| --- | --- |
| Time | `JulianDate`, `utc_to_tai`, `tai_to_utc`, `tai_to_tt`, `tt_to_tai`, `utc_to_tt`, `tt_to_utc` |
| Leap seconds | `leap_second_table_info`, `leap_second_table_expired` |
| Sky | `angular_separation`, `icrs_to_galactic`, `galactic_to_icrs` |
| Earth rotation | `earth_rotation_angle`, `gmst_iau2006` |
| WCS | `Wcs.from_header`, `Wcs.pixel_to_world`, `Wcs.world_to_pixel`, `Wcs.crval` |
| Arrays | `open_zarr`, `ZarrArray.read`, `.shape`, `.ndim`, `.chunks`, `.dtype`, `.sum()`, `.mean()`, `.min()`, `.max()` |
| Errors | `TptPlanetaryError` (base class for non-input, non-I/O errors) |

The full signatures are in `python/tpt_planetary/__init__.pyi`.

## Conventions

- Sky coordinates and angles are in degrees on the Python side. Array inputs
  must be 1-D float64 NumPy arrays of equal length; other dtypes raise `TypeError`.
  Scalars are not accepted by the array functions; wrap them in `np.array([x])`.
- Pixel coordinates for WCS are FITS 1-based. Positions outside a projection's
  domain return NaN.
- Julian Dates are two-part (`day`, `fraction`). The scale (UTC, TAI, TT) is
  not stored; the caller tracks it. A UTC leap second (23:59:60) cannot be
  built with `JulianDate.from_calendar`.
- The leap-second table is embedded (IERS history 1972 to 2017, valid until
  2027-06-28). UTC conversions after that date raise `TptPlanetaryError`.

## Errors

| Python exception | Raised for |
| --- | --- |
| `ValueError` | Bad input: invalid dates, selections, slice steps, array lengths, WCS headers |
| `OSError` | I/O failures reading the Zarr store |
| `FileNotFoundError` | No Zarr array metadata at the given path (an `OSError` subclass) |
| `TypeError` | Arrays of the wrong dtype |
| `TptPlanetaryError` | Everything else, including an expired leap-second table |

## Copies and the GIL

- `ZarrArray.read` copies the selected elements into a new float64 array. The
  chunk buffers are not returned zero-copy. Non-float64 stores are converted
  during the copy.
- `sum`, `mean`, `min` and `max` stream through chunks in blocks; they never
  materialise the whole array. The GIL is released for the read and the
  reductions.
- The coordinate and WCS functions run with the GIL held. They are short
  element-wise loops.

## Not supported in this release

- Remote stores (S3, HTTP), asynchronous I/O and NetCDF3 opening.
- xarray and Dask backends, SunPy and Astropy adapters.
- TDB, ephemerides (SPK), AltAz and ecliptic frames, EOP-based UT1.
- Writing arrays from Python.

## Wheels

Wheels for manylinux and musllinux (x86_64, aarch64), macOS (x86_64, arm64)
and Windows (x86_64) are built by a CI workflow kept outside this crate. Build
them with `maturin build --release` on each platform and attach the results to the release.
