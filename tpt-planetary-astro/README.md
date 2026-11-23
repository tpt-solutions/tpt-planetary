# tpt-planetary-astro

Astronomy layer for [tpt-planetary](https://github.com/tpt-solutions/tpt-planetary):
time scales and Earth rotation, sky coordinate frames with their transforms,
Earth orientation parameters, Solar System ephemerides from NAIF SPK files,
satellite orbits from TLEs, astrometric corrections, and FITS image and WCS
handling.

The goal is to cover in pure Rust, with no C libraries, the parts of Astropy and
SOFA/ERFA that planetary-scale work uses. The crate has no FFI.

**Status:** version 0.1.0, not yet published to crates.io. The API may change
before 1.0. Numerical results are checked against ERFA, Astropy and wcslib in
the workspace validation suite, not by the crate's own tests alone. Some
coefficients are still unvalidated, and some features are partial. See
[Limitations](#limitations).

## Contents

- [What it provides](#what-it-provides)
- [Installation](#installation)
- [Examples](#examples)
- [Feature flags and `no_std`](#feature-flags-and-no_std)
- [Limitations](#limitations)
- [Where this crate fits](#where-this-crate-fits)
- [Development](#development)
- [Changelog](CHANGELOG.md)
- [Licence](#licence)

## What it provides

| Module | Contents |
| --- | --- |
| `time` | TT to TDB and back (the full Fairhead and Bretagnon series, geocentre), Earth rotation angle, GMST and GAST (IAU 2006), and the mean obliquity of the ecliptic (IAU 2006). |
| `precession` | IAU 2006 precession angles and matrix. No frame bias in the matrix. |
| `nutation` | IAU 2000A_R06 nutation: the full luni-solar and planetary series in longitude and obliquity, the nutation matrix, and the equation of the equinoxes. |
| `cio` | CIP coordinates X and Y, the CIO and TIO locators, frame bias, and the CIO-based ICRS to ITRS matrix (IAU 2006/2000A). |
| `eop` | `finals2000A` parser and `EopTable`, which interpolates UT1-UTC and polar motion. No EOP data file is bundled. |
| `frames` | Typed sky coordinates `SkyCoord<F>` in ICRS, Galactic, mean ecliptic of J2000, true ecliptic (with caller-supplied obliquity), and Alt/Az frames, and the transforms between them that are implemented. Also the mean and true ecliptic and equator of date. |
| `astrometry` | Annual aberration (checked against ERFA), proper motion, parallax, and light deflection by the Sun for a source at infinity. |
| `ephemeris` | NAIF DAF/SPK reader for byte slices, segment lookup, Chebyshev evaluation of position and velocity, one-iteration light-time correction, and analytic Earth-Sun distance and velocity series. |
| `orbit` | Keplerian elements, Kepler's equation, TLE parsing with checksums, the near-Earth SGP4 model (WGS-72), and Gibbs velocity from three positions. |
| `wcs` | FITS WCS header parsing, linear pixel transform, the projections TAN, SIN, AIT, ARC, ZEA, STG, CAR, MOL and HPX, LONPOLE and LATPOLE rotation, SIP and TPV distortion, and the inverse world-to-pixel transform. |
| `fits` | FITS header and image reading and writing for primary and IMAGE HDUs. |

## Installation

Once published, add the crate with Cargo:

```toml
[dependencies]
tpt-planetary-astro = "0.1"
```

Until then, depend on it by path from a checkout of the repository. The minimum
supported Rust version is 1.95. The edition is 2024. The crate depends on
`tpt-planetary-core` with its `alloc` feature.

## Examples

### Read and write a FITS image

This example is compiled as a doctest in `fits`. It writes a 2 by 2 float64
primary HDU with one header card, then reads the header and pixels back:

```rust
use tpt_planetary_astro::fits::{self, Bitpix, Scaling, Value, WriteCard};

let header = [WriteCard {
    keyword: "OBJECT",
    value: Some(Value::Text("M31")),
    comment: "target",
}];
let data = [1.0, 2.5, -3.0, 4.0];
let bytes = fits::write_primary_f64(&header, Bitpix::F64, Scaling::NONE, &[2, 2], &data)?;

let hdu = fits::primary(&bytes)?;
assert_eq!(hdu.header().get("OBJECT"), Some(Value::Text("M31")));
assert_eq!(hdu.values().collect::<Vec<f64>>(), data);
# Ok::<(), fits::FitsError>(())
```

### Propagate a satellite from a TLE with SGP4

The first element set of the SGP4 verification file (Vanguard 1, 1958). This is
also a doctest in `orbit`. The position is in kilometres, in the TEME frame:

```rust
use tpt_planetary_astro::orbit::Sgp4;

let line1 = "1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753";
let line2 = "2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667";
let sat = Sgp4::from_tle_lines(line1, line2)?;
let state = sat.propagate(360.0)?;
// Position in km (TEME), close to the published reference -7154.031202 km.
assert!((state.position_km[0] + 7154.031202).abs() < 1e-5);
# Ok::<(), tpt_planetary_astro::orbit::OrbitError>(())
```

### Transform a star to Alt/Az

At the North Pole, the altitude of a star equals its declination. This is a
doctest in `frames`:

```rust
use tpt_planetary_astro::frames::{GeodeticObserver, Icrs, SkyCoord, TransformContext, icrs_to_altaz_at};

let ctx = TransformContext::new((2_451_545.0, 0.0), (2_451_545.0, 0.0), 0.0, 0.0);
let obs = GeodeticObserver::from_degrees(0.0, 90.0, 0.0);
let star = SkyCoord::<Icrs>::from_degrees(0.0, 30.0);
let altaz = icrs_to_altaz_at(star, obs, &ctx);
assert!((altaz.lat_deg() - 30.0).abs() < 0.01);
```

The Alt/Az transform applies no refraction; see [Limitations](#limitations).

## Feature flags and `no_std`

| Feature | Default | Effect |
| --- | --- | --- |
| `std` | yes | Enables the trigonometric modules (`time`, `frames`, `nutation`, `cio`, `wcs`), the ephemeris file reader and light-time helper, and file I/O. |

Some modules work without `std` when given data as slices. The SPK parser is one
example, and so are the angle-only parts of `precession` and `nutation`, and the
fundamental arguments and series tables. Build without `std` with:

```sh
cargo build -p tpt-planetary-astro --no-default-features
```

## Limitations

- **TDB.** The geocentre value uses the full series. The topocentric terms are
  not implemented.
- **Alt/Az.** `icrs_to_altaz_at` applies no atmospheric refraction, no annual or
  diurnal aberration, no light deflection and no parallax. The observed
  (refracted) place is not implemented.
- **Frames not implemented.** FK4 and FK5 are out of scope for 0.1 (see
  [ADR-004](https://github.com/tpt-solutions/tpt-planetary/blob/master/docs/adr/ADR-004-scope-decisions-for-0.1.md)).
  There is no apparent place, and no typed ITRS, CIRS or TIRS frame.
- **Nutation.** The IAU 2000B model is not implemented. The series follow the
  IERS Conventions (2010) tables, which differ from ERFA's `nut06a` by up to
  5.3 microarcseconds in dpsi over 1900 to 2100 (see
  [validation notes](https://github.com/tpt-solutions/tpt-planetary/blob/master/docs/validation.md)).
- **Earth orientation.** No EOP series is bundled. `xp` and `yp` are inputs. The
  polar-motion term that needs a polar-motion time series is not computed. The
  `finals2000A` column map was written from memory and has not been checked
  against the IERS readme.
- **Ephemeris.** The SPK file is read from a byte slice; memory mapping is not
  implemented, because it would need `unsafe`. Light time uses one iteration.
  No SPK kernels are bundled, so `tptplan ephemeris query` needs a kernel path.
- **Light deflection** is implemented for a source at infinity only.
- **WCS.** TPV distortion was checked against wcslib through Astropy. Round trips
  of the projections are tested on synthetic headers in the crate's own tests.
- **Constants and coefficients** written from memory are marked as such in the
  source, including the Galactic pole values and the Earth-Sun series, which is
  accurate to about 1e-4 AU by its own documentation.

## Where this crate fits

```text
tpt-planetary-core        constants, time scales, spherical math   (no_std)
        |
        +--> tpt-planetary-astro   time, frames, ephemeris, WCS   <-- this crate
                    |
                    +--> tpt-planetary-cli    `tptplan astro`, `tptplan ephemeris`
                    +--> bindings/tpt-planetary-py   Wcs, JulianDate, sky transforms
```

This crate does not depend on `tpt-planetary-array`, so the two can be used
independently.

## Development

From the repository root:

```sh
cargo test -p tpt-planetary-astro
cargo run -p tpt-planetary-astro --example validation_dump
cargo run -p tpt-planetary-astro --example wcs_dump
cargo run -p tpt-planetary-astro --example orbit_dump
cargo bench -p tpt-planetary-astro     # divan benchmark, `benches/astro.rs`
```

The integration test `tests/astrometry_erfa.rs` compares the aberration results
with ERFA values. Its reference data is in `tests/fixtures/`. The Python
validation suite in `bindings/tpt-planetary-py/tests/validation/` checks the
rest against pyerfa, Astropy, sgp4 and wcslib.

The project-wide documentation is in the
[user guide](https://github.com/tpt-solutions/tpt-planetary/blob/master/docs/guide/time-and-frames.md),
[validation notes](https://github.com/tpt-solutions/tpt-planetary/blob/master/docs/validation.md),
and [status and limitations](https://github.com/tpt-solutions/tpt-planetary/blob/master/docs/guide/status-and-limitations.md).

## Licence

Dual-licensed under either of

- MIT licence ([LICENSE-MIT](https://github.com/tpt-solutions/tpt-planetary/blob/master/LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](https://github.com/tpt-solutions/tpt-planetary/blob/master/LICENSE-APACHE))

at your option. Unless you state otherwise, any contribution you submit is
dual-licensed in the same way.
