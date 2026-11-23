# Changelog

All notable changes to `tpt-planetary-astro` are recorded here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The workspace-wide history is in the repository's
[CHANGELOG.md](https://github.com/tpt-solutions/tpt-planetary/blob/master/CHANGELOG.md).

## [Unreleased]

### Added

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
- WCS projections ARC, ZEA, STG, CAR, MOL and HPX, and SIP distortion with the
  inverse.
- TPV distortion for WCS headers (`CTYPE = 'RA---TPV'`), with the 40-term
  polynomials `PV1_k` and `PV2_k` and inversion for world-to-pixel. Checked
  against wcslib through Astropy.
- FITS image reading and writing for primary and IMAGE HDUs.
- Orbits: Keplerian elements, Kepler's equation, TLE parsing, the near-Earth
  SGP4 model (WGS-72), and `orbit::gibbs_velocity` from three positions on one
  orbit.
- Frames: the mean ecliptic of date (IAU 2006 precession), the true ecliptic of
  date (with nutation), and the nutation chain with the frame bias.
- Precession (`precession`): IAU 2006 angles and matrix.
- Nutation (`nutation`): IAU 2000A_R06 full series in longitude and obliquity,
  the nutation matrix, and the classical equation of the equinoxes.
- CIP X and Y, the CIO and TIO locators, and the CIO-based ICRS to ITRS matrix
  (`cio`), validated against pyerfa.
- Astrometry: annual aberration (checked against ERFA), proper motion along the
  great circle, parallax, and light deflection by the Sun for a source at
  infinity (checked against ERFA `ld` to 3e-8 arcseconds), with the observer's
  heliocentric position from a Keplerian model.
- Reference data: aberration values from pyerfa 2.0.1.5, used by the integration
  test `tests/astrometry_erfa.rs`. The Python validation suite runs against
  pyerfa, Astropy, sgp4 and numpy.
- Scope decisions for 0.1 (`docs/adr/ADR-004-scope-decisions-for-0.1.md`):
  reference data is user-supplied, FK4 and FK5 are out of scope, no async
  runtime is added, Vincenty only, and three-term TDB.

### Changed

- `icrs_to_ecliptic_at_epoch` is renamed `icrs_to_ecliptic_epoch_obliquity`.
  It keeps the J2000 equinox and is not the mean ecliptic of date.

### Fixed

- FITS `uint64` values with `BZERO = 2^63` lost small values to a double
  rounding. They are now converted once.
- WCS CAR longitudes outside [-180, 180] degrees are invalid, as in WCSLIB.
- WCS LATPOLE selects the native pole root nearest to its value, so CRVAL maps
  to itself.
- WCS right ascension that rounds to 360 is folded to 0.

### Known limits

- Three-term TDB: a truncation bound of about 1.7 ms.
- Alt/Az applies no refraction, aberration, light deflection or parallax, and
  the observed place is not implemented.
- The EOP column map is unverified, and no EOP data file is bundled.
- FK4 and FK5 are not implemented.
