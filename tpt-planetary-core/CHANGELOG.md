# Changelog

All notable changes to `tpt-planetary-core` are recorded here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The workspace-wide history is in the repository's
[CHANGELOG.md](https://github.com/tpt-solutions/tpt-planetary/blob/master/CHANGELOG.md).

## [Unreleased]

### Added

- `no_std` crate with `alloc` and `std` features. The error type works without
  `std`.
- Physical and astronomical constants, each with its cited source. Several
  values were written from memory and are not yet validated.
- Two-part Julian Date (`JulianDate`) and Modified Julian Date types.
- Gregorian and Julian calendar conversion, using Meeus's algorithm in integer
  arithmetic.
- Typed time scales `Utc`, `Tai` and `Tt` through `Epoch<S>`, with conversions
  that take a leap-second table. TDB is not implemented in this crate.
- Embedded leap-second table `EMBEDDED` (version 1; 28 entries from 1972 to
  2017; valid until 2027-06-28). Conversions after the expiry return
  `LeapSecondTableExpired`.
- Spherical helpers: angle newtypes (radians, degrees, hour angle), spherical
  and Cartesian conversion, angular separation, position angle, destination,
  great-circle interpolation and cross-track angle.
- Rotations as unit quaternions on `glam`, with Euler (ZYX) and axis-angle
  conversion, and SLERP.
- WGS84 reference ellipsoid: geodetic and geocentric latitude and Earth-centred
  Cartesian conversion.
- Vincenty's direct and inverse geodesics on the ellipsoid (`geodesic_direct`,
  `geodesic_inverse`). Karney's algorithm is not implemented.
- Benchmark harness using `divan`, in `benches/core.rs`.

### Known limits

- Karney's geodesic algorithm is not implemented.
- The embedded leap-second table expires on 2027-06-28. It must be updated by a
  new release before then.
