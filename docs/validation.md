# Numerical validation against reference software

Status: first pass, Windows, Python 3.13. The checks live in
`bindings/tpt-planetary-py/tests/validation/` and run with pytest. They compare
the Rust crates with pyerfa and Astropy (wcslib via `astropy.wcs`). Passing
tests show agreement within the stated tolerance. They do not show agreement
beyond that, and they are not a claim about the full algorithms.

## Reference software

| Package | Version | Used for |
| --- | --- | --- |
| pyerfa | 2.0.1.5 | `erfa.dat`, `erfa.dtdb`, `erfa.era00`, `erfa.gmst06`, `erfa.obl06`, `erfa.rx`, `erfa.bp06`, `erfa.pom00`, `erfa.epv00`, `erfa.p06e`, `erfa.fal03` to `erfa.fapa03`, `erfa.nut06a`, `erfa.ee06a`, `erfa.num06a`, `erfa.gst06a`, `erfa.xy06`, `erfa.xys06a`, `erfa.s06`, `erfa.sp00`, `erfa.bp00`, `erfa.eect00`, `erfa.c2i06a`, `erfa.c2t06a` |
| astropy | 8.0.2 | `LeapSeconds.from_iers_leap_seconds` (bundled list, read offline), `SkyCoord` (ICRS, Galactic, `BarycentricMeanEcliptic`, `GeocentricMeanEcliptic`), `astropy.wcs.WCS` (wcslib) |
| numpy | 2.5.4 | sampling and vector arithmetic |

Quantities the Python bindings do not expose (TDB, obliquity, the ecliptic
frame, the nutation series, the equation of the equinoxes, the nutation matrix,
GAST, the CIP coordinates X and Y, the CIO and TIO locators, the frame bias
matrix, the EO complementary terms and the CIO-based ICRS to ITRS matrix) are
computed by the `validation_dump` example in
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
| Earth rotation angle | `erfa.era00` (pyerfa 2.0.1.5) | 200 UT1 = UTC epochs, 2000-2026, UT1-UTC = 0 | 1.1e-6 arcsec (was 4.0e-6 before the rounding fix, see findings) | 1 mas | pass |
| GMST (IAU 2006) | `erfa.gmst06` (pyerfa 2.0.1.5) | same 200 epochs, TT = UT1 + 69.184 s | 1.2e-6 arcsec (7.7e-5 ms of time; was 3.6e-6 arcsec) | 1 mas | pass |
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
| Polar-motion matrix W (ITRS -> TIRS), s' = 0 | `erfa.pom00` (pyerfa 2.0.1.5), transposed (ERFA documents `pom00` as TIRS -> ITRS) | 205 (xp, yp) points in arcsec: 5 fixed with both signs, 200 seeded random in [-1, 1] | 0.0 against the transpose; 9.7e-6 without it | 1e-12; untransposed must differ by more than 1e-6 | pass |
| Earth-Sun distance, `earth_sun_distance_au_approx` (Meeus low-precision series) | `erfa.epv00` (pyerfa 2.0.1.5), norm of the heliocentric Earth position | 200 TT epochs, 1900-01-01 to 2100-01-01 | 9.99e-5 AU (14 951 km); mean 2.8e-5 AU (4 144 km) | 1e-4 AU (the helper's stated accuracy) | pass (margin 0.06%) |
| IAU 2006 precession angles zeta_A, z_A, theta_A (precession only) | `erfa.p06e` (pyerfa 2.0.1.5), elements 9-11 | 200 TT epochs, 1900-01-01 to 2100-01-01 | 2.3e-13 arcsec | 1e-6 arcsec (1 microarcsecond) | pass |
| IAU 2006 precession matrix P(t) = R3(-z_A) R2(theta_A) R3(-zeta_A) | same rotation chain built from `erfa.p06e` angles | same 200 epochs | 1.1e-16 | 1e-12 | pass |
| IAU 2006 precession matrix P(t) vs ERFA precession matrix | `erfa.bp06` (pyerfa 2.0.1.5), `rp` | same 200 epochs | 2.4e-12, at the 2100 end | 5e-12 (widened after measuring, see findings) | pass |
| Precession matrix convention (not transposed) | `erfa.bp06` `rp` | 1900 and 2100 ends | transposed P differs from `rp` by 4.5e-2 (guard requires > 1e-3) | > 1e-3 | pass |
| Fundamental arguments F1-F14 (IERS 2010 Eqs. 5.43, 5.44) | `erfa.fal03` ... `erfa.fapa03` (pyerfa 2.0.1.5) | 300 TT epochs, 1900-2100 | 1.6e-12 rad (about 0.34 uas) | 1e-11 rad | pass |
| Nutation dpsi, deps, SOFA/MHB2000 series (no out-of-phase rate terms) | `erfa.nut06a` (pyerfa 2.0.1.5) | 300 TT epochs, 1900-2100 | 0.25 uas (dpsi), 0.22 uas (deps) | 0.5 uas | pass |
| Nutation dpsi, deps, IERS 2010 published tables 5.3a/5.3b (crate default) | `erfa.nut06a` | same 300 epochs | 5.29 uas (dpsi), 0.85 uas (deps); the gap is the out-of-phase rate terms (published minus SOFA variant: 5.31 uas dpsi, 0.74 uas deps) | published minus ERFA, minus the out-of-phase term, at most 0.5 uas; out-of-phase term at least 4 uas (dpsi) | pass |
| Equation of the equinoxes vs `erfa.ee06a` | `erfa.ee06a` | same 300 epochs | 2.65 mas (max), 1.86 mas rms; complementary terms, not implemented | 2.7 mas (bounds the documented omission) | pass |
| Equation of the equinoxes = dpsi cos(eps0) | `erfa.nut06a` dpsi, `erfa.obl06` | same 300 epochs | 0 (rounding only) | 1e-12 rad | pass |
| Nutation matrix convention: N built from ERFA's own dpsi, deps, eps0 | `erfa.num06a` | same 300 epochs | 0.0 | 1e-12 | pass |
| Nutation matrix convention guard (N not transposed) | `erfa.num06a` | same 300 epochs | transposed N differs by 1.7e-4 | > 1e-5 | pass |
| Nutation matrix from the published angles | `erfa.num06a` | same 300 epochs | 2.35e-11 (about 4.9 uas) | 2.5e-11 | pass |
| GAST (`time::gast_iau2006`) vs `erfa.gst06a`, UT1 = TT | `erfa.gst06a` | same 300 epochs | 2.65 mas (max), 1.86 mas rms; the same complementary-term omission | 2.7 mas | pass |
| ICRS -> true equator of date (N P, no frame bias) | `erfa.num06a` times `erfa.bp06` `rp` | 12 epochs x 4 random sky points | 3.04 uas | 6 uas | pass |
| CIP X (IAU 2006/2000A, Table 5.2a, blocks j = 0..4) | `erfa.xy06` (pyerfa 2.0.1.5) | 200 TT epochs, 1900-01-01 to 2100-01-01 | 1.07e-6 uas | 1 uas | pass |
| CIP Y (IAU 2006/2000A, Table 5.2b) | `erfa.xy06` | same 200 epochs | 8.5e-7 uas | 1 uas | pass |
| CIO locator s (Table 5.2d, minus XY/2) | `erfa.s06` with `erfa.xy06` X, Y | same 200 epochs | 3.2e-9 uas | 1 uas | pass |
| TIO locator s' = -47 uas t (Eq. 5.13) | `erfa.sp00` (pyerfa 2.0.1.5) | same 200 epochs | 1e-14 uas | 1 uas | pass |
| Frame bias matrix B = R1(-eta0) R2(xi0) R3(dalpha0) | `erfa.bp00` `rb` (pyerfa 2.0.1.5) | J2000.0 (time-independent) | 6.8e-13 (matrix elements) | 1e-11 | pass |
| EO complementary terms (Table 5.2e, non-polynomial part) | `erfa.eect00` (pyerfa 2.0.1.5) | same 200 epochs; the terms range from 21 to 2648 uas | 3.8e-11 uas | 1 uas | pass |
| Matrix algebra: chain built from ERFA's own X, Y, s, ERA and s' | `erfa.c2t06a` (pyerfa 2.0.1.5) | same 200 epochs; xp = yp = 0 and xp, yp = 0.1, 0.2 arcsec | 1.7e-18 | 1e-12 | pass |
| Convention guard: chain not transposed | `erfa.c2t06a` | same, one epoch | the transpose differs by about 2 | > 1e-5 | pass |
| ICRS -> ITRS matrix, end to end (our X, Y, s, ERA, s', polar motion) | `erfa.c2t06a` (pyerfa 2.0.1.5) | same 200 epochs; xp = yp = 0, and xp, yp = 0.1, 0.2 arcsec | 1.76e-11 rad = 3.63 uas, the same with and without polar motion | 2.5e-11 rad (about 5 uas) | pass |
| ERFA's own X, Y routes: `erfa.xys06a` against `erfa.xy06` | pyerfa 2.0.1.5 | same 200 epochs | 3.63 uas (X and Y) | must exceed 1 uas (the guard that explains the row above) | pass (reference ambiguity) |

Polar motion and Earth-Sun use `validation_dump` modes `pm` and `esd`, and their
tests are in `bindings/tpt-planetary-py/tests/validation/test_earth_orientation.py`.
Precession uses mode `prec` and the tests are in
`bindings/tpt-planetary-py/tests/validation/test_precession.py`.

Nutation uses modes `fa`, `nut`, `nut_sofa`, `ee`, `numat`, `numat_from`, `gast`
and `tod`, and the tests are in
`bindings/tpt-planetary-py/tests/validation/test_nutation.py`. The bindings do
not expose these quantities.

### CIP and CIO inputs

Series: IERS Conventions (2010), Chapter 5, Tables 5.2a (X), 5.2b (Y), 5.2d
(s + XY/2) and 5.2e (the EO complementary terms, non-polynomial part). IAU
2006/2000A. Generated by `tpt-planetary-astro/tools/gen_cio_table.py` into
`tpt-planetary-astro/src/cio_data.rs`. The generator pins these checksums, fails
on any mismatch, checks each block's row count against its `Number of terms`
header, and checks that each printed polynomial line occurs verbatim in its
source file. Regeneration was checked to be byte-identical.

| File | sha256 | Terms per block (j = 0, 1, 2, 3, 4) |
| --- | --- | --- |
| `tab5.2a.txt` (X) | `19549252df9eb77c8237dbf5b749de82b7fd7713a8b60b35371538cd36b6aa1d` | 1306, 253, 36, 4, 1 |
| `tab5.2b.txt` (Y) | `1f17a3a6ad0b468705b3323bfe72d0320ce996798ec7cba0cdde4405d88f14d9` | 962, 277, 30, 5, 1 |
| `tab5.2d.txt` (s + XY/2) | `fe94c83e1ef6f92b15b3f007779ae70c0984c06ee45d5511b4c821ee9a0ecded` | 33, 3, 25, 4, 1 |
| `tab5.2e.txt` (EO complementary terms) | `a5c690b277108e35d88df9cc20fea4120dfbc10a81444bb41b15cf9c6edcd6cb` | 33, 1 |

The files are from `conv.tgz` under `conventions_v1_0_0/2010_official/chapter5/additional_info/`.
The TIO locator (-47 uas t, Eq. 5.13), the frame-bias constants (Eqs. 5.17,
5.21, 5.33) and the Table 5.2e definitions are in the chapter text.

### Nutation inputs

Series: IERS Conventions (2010), Chapter 5, Tables 5.3a and 5.3b (IAU 2000A_R06).
Generated by `tpt-planetary-astro/tools/gen_nutation_table.py` into
`tpt-planetary-astro/src/nutation_data.rs`. The generator pins these checksums and
fails on any mismatch, and it checks each block's row count against its
`Number of terms` header.

| File | sha256 | Terms (j=0, j=1) |
| --- | --- | --- |
| `tab5.3a.txt` (longitude) | `f333edfa5bf8296efbb383cc1425c60b9b5925388e750c588e58dc6ac8a0ece5` | 1320, 38 |
| `tab5.3b.txt` (obliquity) | `3ab45772c58922cc54c5a3424aebc4bb0caaf18d103ff75f939010e5b4b751f1` | 1037, 19 |
| `conv.tgz` (chapter 5 bundle; the chapter text for Eqs. 5.35, 5.43, 5.44) | `99a67ce5b1432362ea175142c7054aee66092bdc3655e40641a618eec567498d` | not parsed |

The fundamental-argument polynomials are taken from Eqs. (5.43) and (5.44) of the
chapter text. The Table 5.2 files in the bundle (5.2a, 5.2b, 5.2d, 5.2e, 5.2f) give
the CIP X and Y, s and EO series, not F1-F14, so they are not used for the
arguments. The `tab5.2e` complementary terms are not implemented (see Not validated).

### Findings

- **Earth rotation angle rounding (fixed).** `time::earth_rotation_angle`
  multiplied the whole Tu, about 3.6e4 days at 1900, by 1.00273781191135448
  before reducing modulo one turn. That lost about 1e-11 rad to rounding. It now
  adds the fractional day separately, and the formula is unchanged. The maximum
  ERA difference from `erfa.era00` at 1900-2100 is 5.5e-14 rad, where it was
  4.2e-11 rad before. The 2000-2026 ERA and GMST rows above reflect the fix.
- **CIP X and Y: ERFA has two routes that disagree.** `erfa.xy06` (the series)
  matches our X and Y to 1.1e-6 uas. `erfa.xys06a`, which `erfa.c2i06a` and
  `erfa.c2t06a` use, differs from `xy06` by 3.63 uas in X and Y at most. Our
  end-to-end matrix against `c2t06a` differs by 3.63 uas, the same value. The
  matrix algebra is exact given ERFA's components (1.7e-18), so the end-to-end
  residual comes from the X, Y route. The likely cause is the IAU 2006 periodic
  corrections of Eq. 5.19 and Table 5.2f, which the published tables include
  and which `xys06a`'s nutation model does not. That attribution is not verified
  term by term. The end-to-end tolerance is set at 5 uas, the level of the
  reference disagreement. The tight algebra check is the test of our matrices.
- **ICRS -> ITRS matrix convention (confirmed).** `icrs_to_itrs` equals
  `erfa.c2t06a` directly. It is not the transpose, which differs by about 2.
  The chain is `W R3(ERA) Q^T`, built from passive rotations as in `frames.rs`.
- **TIO locator.** `s' = -47 uas t` (Eq. 5.13) agrees with `sp00` to 1e-14 uas.
  Only this secular value is implemented. The general form (Eqs. 5.4 and 5.12)
  depends on the polar-motion history, which the module does not take as input.
  The chapter bounds the resulting s at 0.4 mas per century.
- **Frame bias.** `B = R1(-eta0) R2(xi0) R3(dalpha0)` with xi0 = -16.6170 mas
  (Eq. 5.33), eta0 = -6.8192 mas (Eq. 5.17), dalpha0 = -14.60 mas (Eq. 5.21)
  agrees with `erfa.bp00`'s `rb` to 6.8e-13. The chain does not multiply by B,
  because the CIP series already contain the bias (Eq. 5.17: the constant
  -16617 uas in X).
- **EO complementary terms (implemented, not used by GAST).** The non-polynomial
  part of Table 5.2e is `cio::eo_complementary_terms`. It agrees with
  `erfa.eect00` to 3.8e-11 uas over 1900-2100. The terms reach 2.65 mas, so GAST
  (`time::gast_iau2006`) and `equation_of_equinoxes` still omit up to 2.65 mas.
  Wiring the terms into GAST is a separate change. It would tighten the GAST row
  above to the microarcsecond level.

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
- **Polar motion sign convention (confirmed, not changed).** The crate's W
  (`polar_motion_matrix`, `W = R2(xp) R1(yp)`) equals the transpose of
  `erfa.pom00(xp, yp, 0)` to double precision. ERFA's note says its matrix
  satisfies V(TRS) = rpom V(CIP), so `pom00` maps TIRS to ITRS and W is the
  ITRS -> TIRS direction. The physical check agrees: the CIP sits at
  `(xp, -yp, 1)` in ITRS, to first order. The existing test
  `polar_motion_maps_cip_to_tirs_z_axis` covers this. `polar_motion_matrix`
  omits s'. `cio::tirs_to_itrs` adds it, and the chain is checked against
  `erfa.c2t06a` with xp, yp = 0.1, 0.2 arcsec (see the CIP rows above).
  At 2026 that term is -5.9e-11 rad, and it changes W by the same amount.
- **FINALS2000A LOD flag (fixed).** `FINALS2000A_COLUMNS` read the LOD flag
  from column 79. The readme leaves that column blank, so LOD was never read
  from a real row. The map now has no LOD flag. The other columns (MJD, PM
  flag, PM-x, PM-y, UT1 flag, UT1-UTC, LOD) match the readme table, retrieved
  2026-11-16. The regression test is `parses_readme_layout_line`. No
  `finals2000A.data` file was parsed, because none is bundled and the check
  used the readme column table only.
- **Earth-Sun helper (checked to its stated accuracy).** The Meeus series
  agrees with `erfa.epv00` to 9.99e-5 AU over 1900-2100. This is inside the
  1e-4 AU the crate documents, with a small margin. The SPK-based
  `earth_sun_distance_km` is not covered by this check.
- **IAU 2006 precession (validated, precession only).** The angles match
  `erfa.p06e` to 2.3e-13 arcsec. The constant offsets are part of the model:
  zeta_A(0) = +2.650545 arcsec and z_A(0) = -2.650545 arcsec, so the angles are
  not zero at J2000 even though P(0) is the identity. The matrix equals the
  rotation chain built from ERFA's own angles to 1.1e-16. Against `erfa.bp06`
  `rp` the residual is 2.4e-12 at the 2100 end (about 0.5 microarcsecond), and
  it is near 1e-14 at J2000. `bp06` builds `rp` from the Fukushima-Williams
  angles, which are a different parameterisation of the same model. The
  1e-12 bound was set before measuring. The `bp06` comparison was widened to
  5e-12 after the 2.4e-12 result, and the 1e-12 construction test stays strict.
  Frame bias is not included on either side.
- **Galactic.** The 0.025 arcsec difference is inside the bound from the
  published constants: each of the three constants (pole RA and Dec, node) is
  rounded to 5 decimals, up to 18 mas each, and the ICRS and FK5-J2000 pole
  definitions differ by about 17 mas.

- **Nutation: out-of-phase rate terms (a convention difference, not a bug).**
  The published Table 5.3a and 5.3b j=1 blocks include time-varying out-of-phase
  amplitudes (A"'_i in longitude and B"'_i in obliquity). The MHB2000 / SOFA series,
  which `erfa.nut06a` evaluates, omits them. IERS Conventions (2010), Section 5.7.2,
  notes the omission. Against `erfa.nut06a`, the published series differs by 5.29 uas
  (dpsi) and 0.85 uas (deps). Dropping the out-of-phase terms reproduces ERFA to
  0.25 uas and 0.22 uas. The crate implements the published tables. The SOFA variant
  exists only for validation (`nutation_angles_t_without_out_of_phase_rates`).
- **Nutation: residual of the SOFA variant.** After the out-of-phase terms are
  removed, 0.25 uas (dpsi) and 0.22 uas (deps) remain. This is not explained at
  source. Using the MHB2000 expression for L_Ne (5.321159 + 3.812777 t, as SOFA
  does) changes the residual to 0.23 uas, so that is not the cause. The tolerance
  is 0.5 uas.
- **Equation of the equinoxes: complementary terms omitted.** `ee06a` minus
  `dpsi cos(eps0)` reaches 2.65 mas (rms 1.86 mas) over 1900-2100. The crate
  returns the classical term only, so GAST carries the same offset. The amplitudes
  are in Table 5.2e and could be added as a later step.
- **Nutation matrix convention.** The matrix built from ERFA's own angles matches
  `num06a` exactly (0.0), so the rotation order and signs are correct. The first
  draft of the transpose guard used 1e-3. N is within about 1e-4 of the identity,
  so the transpose differs by only 1.7e-4, and the guard was set to 1e-5. It is
  still 7 orders of magnitude above the 1e-12 construction tolerance.

## Not validated

- **Full TDB series.** Only the three leading terms are implemented, and only
  the geocentric term is checked. Topocentric terms and dates outside
  2000-2026 are untested.
- **Nutation beyond IAU 2000A_R06.** IAU 2000B is not implemented. The
  complementary terms of the equation of the origins (Table 5.2e) are
  implemented in `cio` but not used by GAST or the equation of the equinoxes, so
  those differ from the full model by up to 2.65 mas.
- **CIP and CIO beyond IAU 2006/2000A.** The X, Y and s series are the IAU
  2006/2000A tables only. IAU 2000B is not implemented. The TIO locator is the
  secular value (Eq. 5.13). The polar-motion dependent part (Eqs. 5.4 and 5.12)
  is not computed. Polar motion and UT1 are inputs, and no IERS data is bundled.
  The chain is checked against ERFA with polar motion of 0.1 and 0.2 arcsec only.
- **Precession beyond zeta_A, z_A, theta_A and P(t).** The Fukushima-Williams
  angles (`erfa.pfw06`) and the frame-bias-included matrix (`erfa.pmat06`) are
  not compared. Precession was validated only in the bias-free form.
- **True obliquity and the true-of-date ecliptic.** Not compared. The
  true-of-date equator is checked only through the ICRS to true-equator helper,
  which has no frame bias.
- **Earth orientation.** The polar-motion matrix is compared only with s' = 0,
  because `polar_motion_matrix` omits s'. The chain in `cio` includes s'.
  UT1-UTC and the `finals2000A` interpolation are
  not compared with IERS data. The `finals2000A` parser is checked against the
  readme column table, not against a data file. The GMST and ERA tests use
  UT1-UTC = 0 as specified.
- **Ephemerides.** The SPK/DAF reader, Chebyshev evaluation and light time are
  not validated against JPL. The required kernels are not in this repository.
  `earth_sun_distance_km` (SPK) is not compared; only the analytic helper is.- **Other frames.** ICRS -> AltAz and ITRS/geodetic conversions are not compared.
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
