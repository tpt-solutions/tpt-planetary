# Numerical validation against reference software

Status: first pass, Windows, Python 3.13. The checks live in
`bindings/tpt-planetary-py/tests/validation/` and run with pytest. They compare
the Rust crates with pyerfa and Astropy (wcslib via `astropy.wcs`). Passing
tests show agreement within the stated tolerance. They do not show agreement
beyond that, and they are not a claim about the full algorithms.

## Reference software

| Package | Version | Used for |
| --- | --- | --- |
| pyerfa | 2.0.1.5 | `erfa.dat`, `erfa.dtdb`, `erfa.era00`, `erfa.gmst06`, `erfa.obl06`, `erfa.rx`, `erfa.bp06` |
| astropy | 8.0.2 | `LeapSeconds.from_iers_leap_seconds` (bundled list, read offline), `SkyCoord` (ICRS, Galactic, `BarycentricMeanEcliptic`, `GeocentricMeanEcliptic`), `astropy.wcs.WCS` (wcslib) |
| numpy | 2.5.4 | sampling and vector arithmetic |

Quantities the Python bindings do not expose (TDB, obliquity and the ecliptic
frame) are computed by the `validation_dump` example in
`tpt-planetary-astro/examples/`, a std-only helper that is not part of the
public API.

## Results

Tolerances were set before the measurements, and the justification is in each
test's docstring. A test marked xfail is strict: it fails the suite if it starts
passing, so the status cannot drift silently.

| Quantity | Reference function and version | Epochs or points | Max difference | Tolerance | Status |
| --- | --- | --- | --- | --- | --- |
| Leap-second table (28 steps, last value 37 s) | `erfa.dat` (pyerfa 2.0.1.5); `LeapSeconds` (astropy 8.0.2) | 119 dates: 1 Jan and 1 Jul 1972-2017, plus the day before each step | Integer steps identical. Max residual 4.7e-12 s (two-part JD rounding) | Exact integer; residual below 1 ns | pass |
| TT -> TDB (three leading terms) | `erfa.dtdb` (pyerfa 2.0.1.5) | 404 TT epochs, 2000-2026 | 14.89 us (mean 1.28 us, std 5.0 us) | 1 us for validation; 20 us regression guard | xfail (validation); pass (guard) |
| Earth rotation angle | `erfa.era00` (pyerfa 2.0.1.5) | 200 UT1 = UTC epochs, 2000-2026, UT1-UTC = 0 | 4.0e-6 arcsec | 1 mas | pass |
| GMST (IAU 2006) | `erfa.gmst06` (pyerfa 2.0.1.5) | same 200 epochs, TT = UT1 + 69.184 s | 3.6e-6 arcsec (2.4e-4 ms of time) | 1 mas | pass |
| Mean obliquity, IAU 2006 | `erfa.obl06` (pyerfa 2.0.1.5) | J2000.0, +1 and +10 Julian centuries | 0 (identical at the printed precision) | 1 mas | pass |
| ICRS -> Galactic | `SkyCoord.galactic` (astropy 8.0.2) | 1000 random sphere points | 0.0249 arcsec (mean 0.0195 arcsec) | 0.1 arcsec | pass |
| Galactic -> ICRS | `SkyCoord.icrs` (astropy 8.0.2) | 1000 random sphere points | 0.0249 arcsec (mean 0.0196 arcsec) | 0.1 arcsec | pass |
| Mean ecliptic (J2000), definition without frame bias | `erfa.obl06` + `erfa.rx` (pyerfa 2.0.1.5) | 500 random sphere points | 6.1e-10 arcsec | 1 mas | pass |
| Mean ecliptic vs Astropy barycentric frame | `BarycentricMeanEcliptic(equinox="J2000")` (astropy 8.0.2) | same 500 points | lon 0.256 arcsec, lat 0.022 arcsec | 1 mas | xfail (definitional, see below) |
| Same ERFA construction with the `bp06` frame bias | `erfa.bp06` + `erfa.obl06` + `erfa.rx` (pyerfa 2.0.1.5) | same 500 points | 2.0e-10 arcsec | 1 mas | pass |
| Mean ecliptic vs Astropy geocentric frame | `GeocentricMeanEcliptic(equinox="J2000")` (astropy 8.0.2) | same 500 points | lon 741.8 arcsec, lat 20.7 arcsec | 1 mas | xfail (different frame, see below) |
| WCS TAN, RA/Dec, high dec, LONPOLE default | `astropy.wcs.WCS` (wcslib, astropy 8.0.2) | 2000 pixels; 2000 sky points within 6 deg | pix -> world 6.6e-14 deg; world -> pix 1.1e-10 px | 1e-9 deg; 1e-6 px | pass |
| WCS SIN, CD matrix with rotation | same | 2000 pixels; 2000 sky points within 6 deg | pix -> world 6.6e-14 deg; world -> pix 9.9e-10 px | 1e-9 deg; 1e-6 px | pass |
| WCS AIT, dec0 = -60, LONPOLE default 180 | same | 2000 pixels (1244 inside the domain); 2000 sky points | pix -> world 1.4e-13 deg; world -> pix 2.0e-13 px (edge points excluded) | 1e-9 deg; 1e-6 px | pass (after fix) |

### Findings

- **AIT inverse accepted pixels outside the projection (fixed).** The inverse
  checked the ellipse with semi-axes 4 and 2 rad, not the Hammer domain with
  semi-axes 2*sqrt(2) and sqrt(2) rad. Pixels between the two edges returned
  sky positions that project back to other pixels. For example, pixel (300, 10)
  on the test header returned (127.98, -21.82), which projects to (114.3, 45.9).
  wcslib returns NaN for it. The fix is in `tpt-planetary-astro/src/wcs.rs`
  (`Projection::inverse`, the `Ait` arm): reject when
  `2*z2 - 1 < -1e-12`, where `z2 = 1 - (xr/4)^2 - (yr/2)^2`. The regression test
  is `ait_inverse_rejects_pixels_outside_hammer_ellipse`. The existing edge test
  (`ait_edge_at_equator_is_two_root_two_radians`) still passes.
- **Mean ecliptic frame bias (xfail, not changed).** `MeanEcliptic` is the
  ICRS-axes rotation by the J2000 obliquity, as its doc comment states. It does
  not apply the ICRS -> J2000 frame bias (about 17 mas). ERFA reproduces the
  Astropy barycentric frame to 2e-10 arcsec once the `bp06` bias is included.
  Whether the frame should include the bias is a definitional choice, so the
  code was left alone. Changing it would be a documented API decision.
- **Geocentric ecliptic (xfail, not changed).** Astropy's geocentric frame
  applies the observer's velocity aberration, which is up to about 20 arcsec.
  The observed latitude offset of 20.7 arcsec is consistent with that. Our
  frame is barycentric, so the frames differ.
- **TDB (xfail, not changed).** The three leading terms leave residuals up to
  14.9 us. The crate documents this as an approximation. The regression guard
  keeps it from growing past 20 us.
- **Galactic.** The 0.025 arcsec difference is inside the bound from the
  published constants: each of the three constants (pole RA and Dec, node) is
  rounded to 5 decimals, up to 18 mas each, and the ICRS and FK5-J2000 pole
  definitions differ by about 17 mas.

## Not validated

- **Full TDB series.** Only the three leading terms are implemented, and only
  the geocentric term is checked. Topocentric terms and dates outside
  2000-2026 are untested.
- **Nutation, GAST and true-of-date ecliptic.** Nutation is not implemented, so
  the equation of the equinoxes and the true obliquity are not checked. GAST is
  not compared.
- **Precession.** Not implemented and not compared.
- **Earth orientation.** Polar motion, UT1-UTC and the `finals2000A` parser are
  not compared with IERS data. The GMST and ERA tests use UT1-UTC = 0 as
  specified.
- **Ephemerides.** The SPK/DAF reader, Chebyshev evaluation and light time are
  not validated against JPL. The required kernels are not in this repository.
- **Other frames.** ICRS -> AltAz and ITRS/geodetic conversions are not compared.
- **WCS scope.** Only TAN, SIN and AIT, with LONPOLE and LATPOLE left to their
  defaults. Explicit LONPOLE and LATPOLE values, the PC and CD mix, SIP, TPV,
  alternative descriptions and other projections are not compared. The antimeridian
  cut of AIT is excluded from the world -> pixel comparison because the sign of x
  there is a convention.
- **Leap seconds.** Python exposes only the entry count and last value, so the
  entries are checked through conversion results at the step dates, not
  directly. Expiry handling is covered by the existing Rust tests.

## How to run

```sh
cargo test -p tpt-planetary-astro
pytest bindings/tpt-planetary-py/tests/validation -q -rxX
```

The validation tests call `cargo run --example validation_dump`, so a Rust
toolchain must be on `PATH`. The Python extension must be built with `maturin
develop` first.
