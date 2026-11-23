# tpt-planetary-core

Foundation crate for [tpt-planetary](https://github.com/tpt-solutions/tpt-planetary):
physical and astronomical constants, two-part Julian Dates, Gregorian and Julian
calendars, typed UTC, TAI and TT epochs with an embedded leap-second table,
spherical geometry, WGS84 geodesy and quaternion rotations.

The crate is `no_std` when its default `std` feature is turned off, so it builds
for embedded and flight-software targets without the standard library.

**Status:** version 0.1.0, not yet published to crates.io. The API may change
before 1.0. Several constants were written from memory and are not yet checked
against their sources (see [Limitations](#limitations)).

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
| `constants` | Physical and astronomical constants, each with its cited source. |
| `jd` | `JulianDate` and `ModifiedJulianDate`: two-part (day, fraction) representations. |
| `calendar` | Gregorian and Julian calendar conversion (Meeus, chapter 7), in integer arithmetic. `CivilDate`, `CivilTime`, `CalendarSystem`. |
| `scale` | Typed epochs `Epoch<S>` for the time scales `Utc`, `Tai` and `Tt`, and conversions between them through a leap-second table. |
| `leap` | `LeapSecondTable` and the embedded table `EMBEDDED` (version 1, 1972 to 2017), with expiry checks. |
| `angle` | Newtypes `Radians`, `Degrees` and `HourAngle`, with wrapping helpers. |
| `sphere` | `SphericalCoord`, Cartesian conversion, angular separation, position angle, destination, great-circle interpolation and cross-track angle. |
| `ellipsoid` | The WGS84 reference ellipsoid: geodetic and geocentric latitude, geodetic to and from ECEF, and direct and inverse geodesics (Vincenty). |
| `rotation` | Unit-quaternion rotations on `glam`: Euler ZYX and axis-angle conversion, and SLERP. |
| `error` | The crate `Error` type, which works without `std`. |

Time scales are typed, so a TAI value cannot be used where a UTC value is
expected. TDB is not implemented in this crate; `tpt-planetary-astro` has a
three-term TT to TDB conversion.

## Installation

Once published, add the crate with Cargo:

```toml
[dependencies]
tpt-planetary-core = "0.1"
```

Until then, depend on it by path from a checkout of the repository:

```toml
[dependencies]
tpt-planetary-core = { path = "../tpt-planetary/tpt-planetary-core" }
```

The minimum supported Rust version is 1.95. The edition is 2024.

## Examples

### Convert a TT epoch to UTC

The J2000.0 epoch, converted to TAI and then to UTC through the embedded
leap-second table:

```rust
use tpt_planetary_core::leap::EMBEDDED;
use tpt_planetary_core::scale::{Epoch, Tai, Tt, Utc};
use tpt_planetary_core::calendar::CivilDate;

// 2000-01-01 12:00 TT, the J2000.0 epoch.
let tt = Epoch::<Tt>::from_jd(tpt_planetary_core::jd::JulianDate::J2000);
let tai = tt.to_scale::<Tai>(&EMBEDDED)?;
let utc = tai.to_scale::<Utc>(&EMBEDDED)?;
assert_eq!(utc.to_civil(&EMBEDDED)?.date, CivilDate::new(2000, 1, 1));
# Ok::<(), tpt_planetary_core::Error>(())
```

### Angular separation and position angle

```rust
use tpt_planetary_core::angle::Degrees;
use tpt_planetary_core::sphere::{angular_separation, SphericalCoord};

let a = SphericalCoord::new(Degrees(0.0).into(), Degrees(0.0).into());
let b = SphericalCoord::new(Degrees(0.0).into(), Degrees(90.0).into());
assert!((angular_separation(a, b).to_degrees().0 - 90.0).abs() < 1e-12);
```

### Geodetic to ECEF on WGS84

```rust
use tpt_planetary_core::angle::Radians;
use tpt_planetary_core::ellipsoid::{Geodetic, WGS84};

let equator = Geodetic { longitude: Radians(0.0), latitude: Radians(0.0), height: 0.0 };
let v = WGS84.geodetic_to_ecef(equator);
assert!((v.x - 6_378_137.0).abs() < 1e-6);
```

## Feature flags and `no_std`

| Feature | Default | Effect |
| --- | --- | --- |
| `std` | yes | Enables the standard library. Implies `alloc`. |
| `alloc` | no | Enables items that need heap allocation. No public item currently needs it, so this feature changes nothing in the API. |

For a `no_std` build, turn off the default features:

```sh
cargo build -p tpt-planetary-core --no-default-features
```

Floating-point functions (`sin`, `sqrt`, `floor` and others) are not in `core`
for `f64`. The crate routes them through `glam`, built with its `libm` feature,
so no other dependency is needed. Removing `no_std` support is a breaking change.

## Limitations

- **Constants.** Several values were written from memory and are marked as
  unvalidated in the source and in the project status page. Check any constant
  against its cited source before relying on it for a published result.
- **Leap seconds.** `EMBEDDED` is valid until 2027-06-28. Conversions after that
  date return `LeapSecondTableExpired`. Until a release updates the embedded
  table, UTC conversions after that date are not available.
- **Geodesics.** Vincenty's direct and inverse formulas are implemented. Karney's
  algorithm is not.
- **TDB.** Not implemented in this crate.
- **Validation.** Results are checked by the crate's own unit tests. Comparison
  with reference software (SOFA, ERFA, Astropy) is in the workspace validation
  suite, not in this crate.

## Where this crate fits

```text
tpt-planetary-core        constants, time scales, spherical math   (no_std)
        |
        +--> tpt-planetary-array   chunked N-D arrays, Zarr, lazy engine
        |
        +--> tpt-planetary-astro   frames, ephemeris, WCS
```

`tpt-planetary-astro` depends on this crate with the `alloc` feature.
`tpt-planetary-array` does not depend on it.

## Development

From the repository root:

```sh
cargo test -p tpt-planetary-core
cargo bench -p tpt-planetary-core     # divan benchmark, `benches/core.rs`
cargo build -p tpt-planetary-core --no-default-features
```

The project-wide documentation is in the
[user guide](https://github.com/tpt-solutions/tpt-planetary/blob/master/docs/guide/index.md),
the [architecture decision records](https://github.com/tpt-solutions/tpt-planetary/tree/master/docs/adr),
and the [validation notes](https://github.com/tpt-solutions/tpt-planetary/blob/master/docs/validation.md).

## Licence

Dual-licensed under either of

- MIT licence ([LICENSE-MIT](https://github.com/tpt-solutions/tpt-planetary/blob/master/LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](https://github.com/tpt-solutions/tpt-planetary/blob/master/LICENSE-APACHE))

at your option. Unless you state otherwise, any contribution you submit is
dual-licensed in the same way.
