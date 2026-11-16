# Time and frames

Time and frame code is split between two crates:

- `tpt-planetary-core` (`no_std`): Julian dates, calendar conversion, the
  leap-second table, and the typed time scales `Utc`, `Tai` and `Tt`.
- `tpt-planetary-astro` (needs `std` for the trigonometric parts): TDB and
  TT helpers, Earth rotation angle and sidereal time, typed sky coordinates
  with ICRS, Galactic, mean ecliptic and Alt/Az frames, and an ephemeris
  (SPK) reader.

## Time scales (core)

An `Epoch<S>` is an instant on the scale `S`. The scale is a type parameter,
so comparing or subtracting epochs on different scales does not compile.
Conversions go through `to_scale`, and take the leap-second table as an
argument.

The example below converts 2017-01-01 00:00:00 UTC to TAI and TT. It was run
in this workspace, and the output is shown after the code:

```rust
use tpt_planetary_core::calendar::CivilDate;
use tpt_planetary_core::leap::EMBEDDED;
use tpt_planetary_core::scale::{CivilTime, Epoch, Tai, Tt, Utc};

let utc = Epoch::<Utc>::from_civil(
    CivilTime { date: CivilDate::new(2017, 1, 1), hour: 0, minute: 0, second: 0.0 },
    &EMBEDDED,
).unwrap();
let tai: Epoch<Tai> = utc.to_scale(&EMBEDDED).unwrap();
let tt: Epoch<Tt> = tai.to_scale(&EMBEDDED).unwrap();
println!("TAI-UTC at 2017-01-01 = {:.3} s", tai.jd().seconds_since(utc.jd()));
println!("TT-TAI = {:.3} s", tt.jd().seconds_since(tai.jd()));
```

```text
TAI-UTC at 2017-01-01 = 37.000 s
TT-TAI = 32.184 s
```

Julian dates are two-part values (`JulianDate` in `tpt_planetary_core::jd`),
stored as an integer day and a fraction. The integer day is the Julian Day
Number of the civil day beginning at noon.

Calendar conversion uses Meeus's algorithm in exact integer arithmetic. It
supports the Gregorian and Julian calendars.

### Leap-second table caveats

- The embedded table is `EMBEDDED`, version 1. It lists the TAI - UTC steps
  from 1972-01-01 (10 s) to 2017-01-01 (37 s). Its 28 entries were checked
  line by line against the IERS `leap-seconds.list` file.
- The table has a `valid_until` date, which is **2027-06-28**. The value comes
  from the expiry line of that IERS file. After that date, UTC conversions
  return `Error::LeapSecondTableExpired`. They do not guess.
- A new leap second is announced only a few months in advance, so the table
  has to be refreshed before each release.

The expiry behaviour, from the same run:

```rust
use tpt_planetary_core::calendar::CivilDate;
use tpt_planetary_core::leap::EMBEDDED;
use tpt_planetary_core::scale::{CivilTime, Epoch, Utc};

println!("valid_until = {:?} version={}", EMBEDDED.valid_until(), EMBEDDED.version());
let late = Epoch::<Utc>::from_civil(
    CivilTime { date: CivilDate::new(2028, 1, 1), hour: 0, minute: 0, second: 0.0 },
    &EMBEDDED,
);
println!("2028 utc from_civil -> {:?}", late.map(|e| e.jd()));
```

```text
valid_until = CivilDate { year: 2027, month: 6, day: 28 } version=1
2028 utc from_civil -> Err(LeapSecondTableExpired)
```

## TDB and TT (astro)

`tpt_planetary_astro::time` converts between TT and TDB with the leading
three terms of the Fairhead and Bretagnon series. The bound on the
truncation is about 1.7 ms: the amplitudes of the three terms sum to
1.6929 ms.

- The three terms were transcribed from memory of the ERFA `dtdb` leading
  terms. They have **not** been checked against the source tables.
- The full series is not implemented, so the result is an approximation.
- Date pairs are passed as `(hi, lo)` with `hi + lo` the Julian date.

The example below continues from the TT epoch above, using the same run. The
values are printed for inspection, not as validated results:

```rust
use tpt_planetary_astro::ephemeris::earth_sun_distance_au_approx;
use tpt_planetary_astro::time::{tdb_to_tt, tt_to_tdb};

let jd = tt.jd();
let (hi, lo) = tt_to_tdb(jd.day(), jd.fraction());
println!("TDB-TT at 2017 = {:.6e} s", ((hi - jd.day()) + (lo - jd.fraction())) * 86400.0);
let (bh, bl) = tdb_to_tt(hi, lo);
println!("round trip err = {:.3e} s", ((bh - jd.day()) + (bl - jd.fraction())) * 86400.0);
println!("earth-sun AU at TT 2017 = {:.6}", earth_sun_distance_au_approx(jd.day(), jd.fraction()));
```

```text
TDB-TT at 2017 = -4.607892e-5 s
round trip err = 0.000e0 s
earth-sun AU at TT 2017 = 0.983305
```

The round-trip check is in `tpt-planetary-cli/tests/dataflow.rs` with a
100 ns tolerance. The Earth-Sun value is the low-precision Meeus series in
AU. The 2017 value is close to the known January perihelion, but the series
has not been checked against an ephemeris.

Earth rotation angle, GMST and GAST are also in `astro::time`. The UT1 scale
type is not implemented, so the caller supplies UT1 as a Julian date pair.
`astro::eop` parses `finals2000A`-format text and provides `EopTable`, which
interpolates UT1-UTC and polar motion and converts UTC to UT1. Its column
positions are unverified, and no EOP data file is bundled.

## Frames (astro)

`SkyCoord<F>` holds a longitude and latitude in radians and carries its frame
as a type parameter. Frame transforms are implemented for these pairs:

- ICRS and Galactic (fixed rotation from the J2000 Galactic pole constants);
- ICRS and mean ecliptic of J2000 (IAU 1976 obliquity);
- ICRS to true ecliptic, with an obliquity supplied by the caller;
- ICRS to Alt/Az for an `Observer` and a local sidereal time (the older,
  apparent-RA helper, `icrs_to_altaz`);
- ICRS, CIRS, TIRS and ITRS, in all twelve ordered pairs, with the epoch and
  Earth orientation supplied (see [Typed CIRS, TIRS and ITRS](#typed-cirs-tirs-and-itrs));
- ICRS to Alt/Az for a `GeodeticObserver` on WGS84, at a given epoch (see
  [Alt/Az from a geodetic observer](#altaz-from-a-geodetic-observer)).

The example below converts M31's ICRS position to Galactic, round-trips it,
and computes its altitude and azimuth at an observer for local sidereal time
zero. The numbers were printed by this run:

```rust
use tpt_planetary_astro::frames::{Galactic, Icrs, Observer, SkyCoord, icrs_to_altaz};

let m31 = SkyCoord::<Icrs>::from_degrees(10.6847, 41.2687);
let gal: SkyCoord<Galactic> = m31.transform_to();
println!("galactic lon={:.4} lat={:.4}", gal.lon_deg(), gal.lat_deg());
let back: SkyCoord<Icrs> = gal.transform_to();
println!("roundtrip ra={:.6} dec={:.6}", back.lon_deg(), back.lat_deg());
let obs = Observer::from_degrees(51.5, -0.13);
let altaz = icrs_to_altaz(m31, obs, 0.0);
println!("altaz az={:.4} alt={:.4}", altaz.lon_deg(), altaz.lat_deg());
```

```text
galactic lon=121.1743 lat=-21.5734
roundtrip ra=10.684700 dec=41.268700
altaz az=140.2279 alt=77.4183
```

The Galactic longitude and latitude of M31 agree with commonly published
values to the digits shown. That is a sanity check on the orientation of the
rotation, not a validation against Astropy. The Galactic pole constants were
written from memory and are on the open list in
[Status and limitations](status-and-limitations.md).

Known gaps in the frame layer:

- The "ICRS to ecliptic at epoch" helper rotates by the epoch obliquity but
  keeps the J2000 equinox, so it is not the mean ecliptic of date. Its name is
  on the open list.
- IAU 2006 precession (`tpt_planetary_astro::precession`) is implemented as a
  mean-equator-of-date transform with no nutation and no frame bias. Nutation,
  aberration and proper motion are not implemented. The `Observer` used by
  `icrs_to_altaz` has no polar motion, and its height is ignored.
- The polar-motion sign convention is checked against `erfa.pom00` and
  `erfa.c2t06a` (see [Validation](validation.md)).

## CIO-based ICRS to ITRS chain (astro::cio)

`tpt_planetary_astro::cio` implements the IAU 2006/2000A celestial-to-terrestrial
chain of IERS Conventions (2010), Chapter 5, as matrices:

- `cip_xy(tt_hi, tt_lo)`: the CIP coordinates X and Y in the GCRS (Table 5.2a
  and Table 5.2b, Eq. 5.16). They include the frame bias at J2000.0.
- `cio_locator(tt_hi, tt_lo)`: the CIO locator s, from Table 5.2d (s + XY/2)
  minus XY/2.
- `tio_locator(tt_hi, tt_lo)`: the TIO locator s' = -47 uas t (Eq. 5.13).
- `frame_bias_matrix()`: ICRS to the mean equator and equinox of J2000.0 (Eqs.
  5.17, 5.21, 5.33). The chain does not apply it, because the X and Y series
  already include the bias.
- `icrs_to_cirs`, `cirs_to_tirs`, `tirs_to_itrs`, and the composite
  `icrs_to_itrs(tt_hi, tt_lo, ut1_hi, ut1_lo, xp, yp)`, which returns the
  matrix `M` with `v_ITRS = M v_ICRS`. The angles are in radians.
- `eo_complementary_terms(tt_hi, tt_lo)`: the non-polynomial part of Table 5.2e,
  the complementary terms of the equation of the origins.

Rotations are passive, as in `frames.rs`. The matrices are the transposes of
the chapter's active `Q`, `R` and `W` as printed, and ERFA's `c2t06a` (the
GCRS to ITRS matrix) equals `icrs_to_itrs` directly.

Accuracy against pyerfa over 1900-2100 (see [Validation](validation.md)):

- X, Y and s agree with `erfa.xy06` and `erfa.s06` to about 1 uas or better.
- Given ERFA's own components, the chain matches `erfa.c2t06a` to 2e-18.
- End to end, `icrs_to_itrs` differs from `erfa.c2t06a` by 3.6 uas. That is
  the difference between ERFA's `xys06a` and `xy06` routes to X and Y. The
  series used here matches `xy06`.

Not included: IAU 2000B; the polar-motion history term of s' (Eqs. 5.4 and
5.12, bounded by the chapter at 0.4 mas per century); UT1 and polar motion,
which are inputs (no IERS data is bundled); The module needs `std`.

## Typed CIRS, TIRS and ITRS

`frames` has the marker types `Cirs`, `Tirs` and `Itrs`. They implement the
`Frame` trait, like `Icrs`. Their coordinates are directions, so a `SkyCoord` holds a
right ascension or longitude and a latitude as for ICRS.

Transforms that depend on the epoch take a `TransformContext`, which carries
the TT and UT1 two-part Julian dates and the polar coordinates `xp`, `yp`:

```rust
use tpt_planetary_astro::frames::{Cirs, Icrs, Itrs, SkyCoord, TransformContext};

// TT, UT1 (both as (hi, lo)), and polar motion in arcseconds.
let ctx = TransformContext::new((2_451_545.0, 0.0), (2_451_545.0, 0.0), 0.1, 0.2);
let star = SkyCoord::<Icrs>::from_degrees(10.6847, 41.2687);
let itrs = star.transform_at::<Itrs>(&ctx);
let back = itrs.transform_at::<Icrs>(&ctx);
```

`transform_at` applies a rotation: `ICRS -> CIRS` is `Q`, `CIRS -> TIRS` is the Earth
rotation `R3(ERA)`, and `TIRS -> ITRS` is the polar motion with the TIO locator.
Each step is orthogonal, so the inverse of each transform is its transpose. The
composition `ICRS -> CIRS -> TIRS -> ITRS` equals `cio::icrs_to_itrs` at the
same epoch, and a round trip returns the input to rounding error. The existing
`FrameTransform` methods, which take no epoch, are unchanged.

The TIO locator is the secular value `-47 uas t` (see `cio::tio_locator`). The
polar-motion history term of Eq. 5.4 is not computed, so the transforms do not
take a polar-motion time series. The inputs are only `xp` and `yp` at one
epoch. No EOP table is read.

## Alt/Az from a geodetic observer

`frames::icrs_to_altaz_at(coord, observer, ctx)` gives the altitude and azimuth
of an ICRS direction seen from a `GeodeticObserver`. The observer is a longitude
east, a geodetic latitude and a height on WGS84. The direction is rotated to
ITRS with the CIO chain, and the altitude and azimuth are then taken from the
ellipsoid normal and the local east and north. Azimuth is measured from north
through east.

The example below uses J2000.0 TT and UT1, zero polar motion, and an observer at
London. It prints the values the crate returns (from `validation_dump altaz`,
which calls the same function):

```rust
use tpt_planetary_astro::frames::{GeodeticObserver, Icrs, SkyCoord, TransformContext, icrs_to_altaz_at};

let ctx = TransformContext::new((2_451_545.0, 0.0), (2_451_545.0, 0.0), 0.0, 0.0);
let m31 = SkyCoord::<Icrs>::from_degrees(10.6847, 41.2687);
let london = GeodeticObserver::from_degrees(-0.13, 51.5, 0.0);
let altaz = icrs_to_altaz_at(m31, london, &ctx);
println!("altaz az={:.4} alt={:.4}", altaz.lon_deg(), altaz.lat_deg());
```

```text
altaz az=61.1401 alt=30.8827
```

Not applied: atmospheric refraction, annual and diurnal aberration, light
deflection and parallax. The observer's height does not change the result for a
direction at infinity. Polar motion and UT1 are the caller's inputs. The
validation against Astropy and ERFA, and the size of the excluded effects, are
in [Validation](../validation.md).

## Ephemerides

`tpt_planetary_astro::ephemeris` reads NAIF DAF/SPK files, such as JPL DE440,
from a byte slice. It evaluates Chebyshev segments for position and velocity,
and it applies one iteration of light-time correction. The helper
`earth_sun_distance_km` needs a kernel file with the Earth and Sun segments.
The workspace does not bundle a kernel, and the successful kernel path has not
been validated against JPL Horizons. See
[Status and limitations](status-and-limitations.md).
