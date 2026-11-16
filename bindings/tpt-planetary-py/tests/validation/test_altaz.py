"""Alt/Az of ICRS directions for a WGS84 geodetic observer, against ERFA and
Astropy.

The crate's value comes from ``validation_dump altaz`` (``frames::icrs_to_altaz_at``):
the CIO-based ICRS to ITRS matrix, then the observer's local vertical. It
applies no refraction, no aberration, no light deflection and no parallax.

Reference conventions (pyerfa 2.0.1.5 and Astropy, test oracles only):

- Astropy: ``SkyCoord(ra, dec, frame="icrs").transform_to(AltAz(...))`` with
  ``pressure=0``. Astropy applies annual and diurnal aberration (and light
  deflection) in this transform, and offers no option to turn them off, so it
  is not the no-aberration reference.
- ERFA full: ``erfa.atco13`` with ``phpa=0``. This is the route Astropy uses,
  with aberration. It agrees with Astropy to the milliarcsecond level, which
  is checked below.
- ERFA without annual aberration: ``erfa.apco`` with the Earth's barycentric
  velocity set to zero, then ``erfa.atciq`` and ``erfa.atioq``. This is the
  reference for our no-aberration value. The diurnal aberration from the
  observer's rotation is still present in ``apco`` (at most about 0.32
  arcsec at the equator), so the residual against this reference is that
  size, not zero.

Inputs that both sides share: TT and UT1 two-part Julian dates, and polar
motion ``xp, yp``, all taken from Astropy's own bundled IERS-B tables. UT1-UTC
and polar motion are not zero there, and the test does not force them to
zero. The observer height is not passed to the crate's dump: it does not
enter the result for a direction at infinity.

Epochs: 2000-01-01 12:00, 2010-07-01 00:00 and 2020-03-15 06:00 UTC.
Directions: 200 uniform on the sphere per epoch and observer, from a fixed
seed.

Tolerances (set from the measured residuals, not fitted to the data):
- ERFA full vs Astropy: 1 milliarcsecond.
- Ours vs ERFA without annual aberration: 0.5 arcsec. The residual is the
  diurnal aberration of the observer's rotation (about 0.27 arcsec at these
  latitudes) plus light deflection by the Sun, which is larger for directions
  close to the Sun. The largest residual measured, 0.37 arcsec, is at 3 degrees
  from the Sun; the mean is 0.21 arcsec. The matrix chain itself contributes a
  few microarcseconds.
- Ours vs Astropy: differs from ERFA-full minus ERFA-no-annual-aberration (the
  annual aberration displacement) by at most 0.5 arcsec.
"""

from __future__ import annotations

import erfa
import numpy as np
import pytest
from astropy import units as u
from astropy.coordinates import AltAz, EarthLocation, SkyCoord
from astropy.time import Time
from astropy.utils import iers

from _reference import run_validation_dump

iers.conf.auto_download = False
iers.conf.auto_max_age = None

EPOCHS_UTC = ["2000-01-01T12:00:00", "2010-07-01T00:00:00", "2020-03-15T06:00:00"]
# (geodetic longitude deg east, geodetic latitude deg, height m)
OBSERVERS = [(-70.6, -33.5, 2400.0), (151.2, -33.9, 58.0)]
N_DIRECTIONS = 200
SEED = 20260315

FULL_VS_ASTROPY_TOL_ARCSEC = 1e-3
OURS_VS_ERFA_ZERO_AB_TOL_ARCSEC = 0.5
OURS_VS_ASTROPY_TOL_ARCSEC = 0.5
ANNUAL_ABERRATION_MAX_ARCSEC = 25.0

RAD_PER_ARCSEC = np.pi / (180.0 * 3600.0)


def _unit(az_deg, alt_deg):
    """Unit vector for an azimuth (north through east) and altitude, degrees."""
    az = np.radians(az_deg)
    alt = np.radians(alt_deg)
    return np.stack(
        [np.cos(alt) * np.cos(az), np.cos(alt) * np.sin(az), np.sin(alt)], axis=-1
    )


def _sep_arcsec(a_az, a_alt, b_az, b_alt):
    """Angular separation of two (az, alt) directions, in arcseconds."""
    u_a = _unit(a_az, a_alt)
    u_b = _unit(b_az, b_alt)
    cross = np.linalg.norm(np.cross(u_a, u_b), axis=-1)
    dot = np.sum(u_a * u_b, axis=-1)
    return np.degrees(np.arctan2(cross, dot)) * 3600.0


def _directions(seed, n):
    rng = np.random.default_rng(seed)
    ra = rng.uniform(0.0, 360.0, n)
    dec = np.degrees(np.arcsin(rng.uniform(-1.0, 1.0, n)))
    return ra, dec


class _Case:
    """One observer at one epoch, with the shared inputs and 200 directions."""

    def __init__(self, observer, tt, ut1, utc, xp_arcsec, yp_arcsec, ra, dec):
        self.lon_deg, self.lat_deg, self.height_m = observer
        self.tt = tt
        self.ut1 = ut1
        self.utc = utc
        self.xp_arcsec = xp_arcsec
        self.yp_arcsec = yp_arcsec
        self.ra = ra
        self.dec = dec


def _build_cases():
    t = Time(EPOCHS_UTC, scale="utc")
    pm = iers.IERS_Auto.open().pm_xy(t)
    xp = pm[0].to_value(u.arcsec)
    yp = pm[1].to_value(u.arcsec)
    ra, dec = _directions(SEED, N_DIRECTIONS)
    cases = []
    for j in range(len(EPOCHS_UTC)):
        for observer in OBSERVERS:
            cases.append(
                _Case(
                    observer,
                    (t.tt[j].jd1, t.tt[j].jd2),
                    (t.ut1[j].jd1, t.ut1[j].jd2),
                    (t.utc[j].jd1, t.utc[j].jd2),
                    float(xp[j]),
                    float(yp[j]),
                    ra,
                    dec,
                )
            )
    return cases


CASES = _build_cases()
CASE_IDS = [
    f"epoch{i // len(OBSERVERS)}-obs{i % len(OBSERVERS)}" for i in range(len(CASES))
]


def _ours(case):
    n = len(case.ra)
    ones = np.ones(n)
    rows = np.column_stack(
        [
            case.ra,
            case.dec,
            ones * case.lon_deg,
            ones * case.lat_deg,
            ones * case.tt[0],
            ones * case.tt[1],
            ones * case.ut1[0],
            ones * case.ut1[1],
            ones * case.xp_arcsec,
            ones * case.yp_arcsec,
        ]
    )
    out = run_validation_dump("altaz", rows)
    return out[:, 0], out[:, 1]


def _erfa_zero_aberration(case):
    """ERFA apco with zero Earth velocity: no annual aberration."""
    tt1, tt2 = case.tt
    pvh, pvb = erfa.epv00(tt1, tt2)
    # Barycentric Earth state with zero velocity (pyerfa's structured pv
    # type); heliocentric position for light deflection.
    ebpv = np.array(pvb, copy=True)
    ebpv["v"] = 0.0
    ehp = np.array(pvh["p"], dtype=float)
    x, y = erfa.xy06(tt1, tt2)
    s = erfa.s06(tt1, tt2, x, y)
    theta = erfa.era00(*case.ut1)
    sp = erfa.sp00(tt1, tt2)
    astrom = erfa.apco(
        tt1,
        tt2,
        ebpv,
        ehp,
        x,
        y,
        s,
        theta,
        np.radians(case.lon_deg),
        np.radians(case.lat_deg),
        case.height_m,
        case.xp_arcsec * RAD_PER_ARCSEC,
        case.yp_arcsec * RAD_PER_ARCSEC,
        sp,
        0.0,
        0.0,
    )
    rc = np.radians(case.ra)
    dc = np.radians(case.dec)
    zeros = np.zeros_like(rc)
    ri, di = erfa.atciq(rc, dc, zeros, zeros, zeros, zeros, astrom)
    aob, zob, *_ = erfa.atioq(ri, di, astrom)
    return np.degrees(aob) % 360.0, 90.0 - np.degrees(zob)


def _erfa_full(case):
    """ERFA atco13 with aberration and no refraction: the route Astropy takes."""
    rc = np.radians(case.ra)
    dc = np.radians(case.dec)
    zeros = np.zeros_like(rc)
    utc1, utc2 = case.utc
    ut1_1, ut1_2 = case.ut1
    dut1 = ((ut1_1 - utc1) + (ut1_2 - utc2)) * 86400.0
    aob, zob, *_ = erfa.atco13(
        rc,
        dc,
        zeros,
        zeros,
        zeros,
        zeros,
        utc1,
        utc2,
        dut1,
        np.radians(case.lon_deg),
        np.radians(case.lat_deg),
        case.height_m,
        case.xp_arcsec * RAD_PER_ARCSEC,
        case.yp_arcsec * RAD_PER_ARCSEC,
        0.0,
        15.0,
        0.0,
        1.0,
    )
    return np.degrees(aob) % 360.0, 90.0 - np.degrees(zob)


def _astropy(case):
    t = Time(case.utc[0], case.utc[1], format="jd", scale="utc")
    loc = EarthLocation.from_geodetic(
        lon=case.lon_deg * u.deg, lat=case.lat_deg * u.deg, height=case.height_m * u.m
    )
    frame = AltAz(obstime=t, location=loc, pressure=0 * u.hPa)
    sc = SkyCoord(ra=case.ra * u.deg, dec=case.dec * u.deg, frame="icrs")
    aa = sc.transform_to(frame)
    return aa.az.to_value(u.deg), aa.alt.to_value(u.deg)


@pytest.mark.parametrize("case", CASES, ids=CASE_IDS)
def test_erfa_full_matches_astropy(case):
    """Sanity check of the reference: Astropy's AltAz is ERFA's atco13."""
    az_e, alt_e = _erfa_full(case)
    az_a, alt_a = _astropy(case)
    sep = _sep_arcsec(az_e, alt_e, az_a, alt_a)
    assert sep.max() < FULL_VS_ASTROPY_TOL_ARCSEC, f"max {sep.max()} arcsec"


@pytest.mark.parametrize("case", CASES, ids=CASE_IDS)
def test_ours_matches_erfa_without_annual_aberration(case):
    az_o, alt_o = _ours(case)
    az_z, alt_z = _erfa_zero_aberration(case)
    sep = _sep_arcsec(az_o, alt_o, az_z, alt_z)
    assert sep.max() < OURS_VS_ERFA_ZERO_AB_TOL_ARCSEC, f"max {sep.max()} arcsec"


@pytest.mark.parametrize("case", CASES, ids=CASE_IDS)
def test_ours_vs_astropy_differs_by_aberration_only(case):
    """Ours and Astropy differ by the annual aberration that Astropy applies.

    The separation between ERFA with and without annual aberration is the
    aberration displacement of each direction. Ours minus Astropy must match
    that displacement to within the no-aberration tolerance.
    """
    az_o, alt_o = _ours(case)
    az_a, alt_a = _astropy(case)
    az_f, alt_f = _erfa_full(case)
    az_z, alt_z = _erfa_zero_aberration(case)
    aberration = _sep_arcsec(az_f, alt_f, az_z, alt_z)
    assert aberration.max() < ANNUAL_ABERRATION_MAX_ARCSEC
    diff = _sep_arcsec(az_o, alt_o, az_a, alt_a)
    assert np.max(np.abs(diff - aberration)) < OURS_VS_ASTROPY_TOL_ARCSEC
