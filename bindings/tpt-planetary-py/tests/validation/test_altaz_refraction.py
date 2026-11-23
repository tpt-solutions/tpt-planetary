"""Refracted Alt/Az of ICRS directions against ERFA and Astropy.

The crate's value comes from ``validation_dump altaz_refracted``
(``frames::icrs_to_altaz_refracted_at``): the geometric Alt/Az of
``test_altaz`` (no aberration) with the refraction model of ``refco`` applied to
the geometric altitude. Inputs are those of ``test_altaz``, so the geometric
residuals of that test are shared; the checks here are on the refraction itself.

Reference conventions (test oracles only):

- ERFA without annual aberration, with refraction: ``erfa.apco`` with zero Earth
  velocity and the refraction constants ``refa, refb`` from ``erfa.refco`` (the
  same as ``apco`` takes for ``phpa``, ``tc``, ``rh``, ``wl``), then
  ``erfa.atciq`` and ``erfa.atioq``. The geometric reference is the same with
  zero refraction constants.
- Astropy: ``AltAz`` with ``pressure``, ``temperature``, ``relative_humidity``
  and ``obswl`` set, against the same frame with ``pressure=0``. Astropy applies
  annual aberration as well, so its refraction shift is compared with a
  tolerance that allows for that.

Checks use directions at true altitudes of at least 15 degrees (zenith distance
at most 75 degrees), where the refraction model is accurate to a few
milliarcseconds. Below that the model is outside its stated range.

Tolerances, with the largest residuals measured over the 12 cases, two
atmospheres and directions at altitude 15 degrees or more:
- Refraction shift, ours vs ERFA: 0.01 arcsec (measured 0.0027). Both sides use
  the same constants, so the difference is the geometric residual's effect on
  the shift.
- Refracted altitude, ours vs ERFA: 0.5 arcsec (measured 0.27). This is the
  no-aberration residual of ``test_altaz``, which is the same size here.
- Refraction shift, ours vs Astropy: 0.15 arcsec (measured 0.072). Astropy's
  geometric position includes annual aberration, which moves the zenith distance
  and so the shift.
"""

from __future__ import annotations

import erfa
import numpy as np
import pytest
from astropy import units as u
from astropy.coordinates import AltAz, EarthLocation, SkyCoord
from astropy.time import Time

from _reference import run_validation_dump
from test_altaz import (
    CASES,
    CASE_IDS,
    RAD_PER_ARCSEC,
    _erfa_zero_aberration,
    _ours,
    _sep_arcsec,
)

# (pressure hPa, temperature C, relative humidity, wavelength um)
ATMOSPHERES = [
    (1013.25, 15.0, 0.5, 0.55),
    (700.0, -5.0, 0.8, 0.7),
]
MIN_ALTITUDE_DEG = 15.0

SHIFT_VS_ERFA_TOL_ARCSEC = 0.01
REFRACTED_VS_ERFA_TOL_ARCSEC = 0.5
SHIFT_VS_ASTROPY_TOL_ARCSEC = 0.15


def _ours_refracted(case, atmosphere):
    n = len(case.ra)
    ones = np.ones(n)
    pressure, temperature, humidity, wavelength = atmosphere
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
            ones * pressure,
            ones * temperature,
            ones * humidity,
            ones * wavelength,
        ]
    )
    out = run_validation_dump("altaz_refracted", rows)
    return out[:, 0], out[:, 1]


def _erfa_refracted(case, atmosphere):
    """ERFA apco (no aberration) with refraction constants from refco."""
    pressure, temperature, humidity, wavelength = atmosphere
    refa, refb = erfa.refco(pressure, temperature, humidity, wavelength)
    tt1, tt2 = case.tt
    pvh, pvb = erfa.epv00(tt1, tt2)
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
        refa,
        refb,
    )
    rc = np.radians(case.ra)
    dc = np.radians(case.dec)
    zeros = np.zeros_like(rc)
    ri, di = erfa.atciq(rc, dc, zeros, zeros, zeros, zeros, astrom)
    aob, zob, *_ = erfa.atioq(ri, di, astrom)
    return np.degrees(aob) % 360.0, 90.0 - np.degrees(zob)


def _angle_sep_arcsec(az1, alt1, az2, alt2):
    return _sep_arcsec(az1, alt1, az2, alt2)


def _astropy_atmosphere(case, atmosphere):
    """Astropy AltAz for the case's directions; ``atmosphere=None`` turns refraction off."""
    t = Time(case.utc[0], case.utc[1], format="jd", scale="utc")
    loc = EarthLocation.from_geodetic(
        lon=case.lon_deg * u.deg, lat=case.lat_deg * u.deg, height=case.height_m * u.m
    )
    if atmosphere is None:
        frame = AltAz(obstime=t, location=loc, pressure=0 * u.hPa)
    else:
        pressure, temperature, humidity, wavelength = atmosphere
        frame = AltAz(
            obstime=t,
            location=loc,
            pressure=pressure * u.hPa,
            temperature=temperature * u.deg_C,
            relative_humidity=humidity,
            obswl=wavelength * u.micron,
        )
    sc = SkyCoord(ra=case.ra * u.deg, dec=case.dec * u.deg, frame="icrs")
    aa = sc.transform_to(frame)
    return aa.az.to_value(u.deg), aa.alt.to_value(u.deg)


@pytest.mark.parametrize("atmosphere", ATMOSPHERES, ids=["std", "high"])
@pytest.mark.parametrize("case", CASES, ids=CASE_IDS)
def test_refraction_shift_matches_erfa(case, atmosphere):
    """The refraction shift of the altitude agrees with ERFA's, direction by direction."""
    az_geo, alt_geo = _ours(case)
    az_ref, alt_ref = _ours_refracted(case, atmosphere)
    erfa_az_geo, erfa_alt_geo = _erfa_zero_aberration(case)
    erfa_az_ref, erfa_alt_ref = _erfa_refracted(case, atmosphere)
    keep = alt_geo >= MIN_ALTITUDE_DEG
    ours_shift = (alt_ref - alt_geo)[keep] * 3600.0
    erfa_shift = (erfa_alt_ref - erfa_alt_geo)[keep] * 3600.0
    worst = np.abs(ours_shift - erfa_shift).max()
    assert worst < SHIFT_VS_ERFA_TOL_ARCSEC, f"largest shift difference {worst} arcsec"
    # Refraction changes only the altitude: the azimuth is the geometric one.
    daz = (az_ref - az_geo + 180.0) % 360.0 - 180.0
    assert np.abs(daz).max() < 1e-9, "refraction changed the azimuth"


@pytest.mark.parametrize("atmosphere", ATMOSPHERES, ids=["std", "high"])
@pytest.mark.parametrize("case", CASES, ids=CASE_IDS)
def test_refracted_altitude_matches_erfa(case, atmosphere):
    az_ref, alt_ref = _ours_refracted(case, atmosphere)
    erfa_az, erfa_alt = _erfa_refracted(case, atmosphere)
    keep = erfa_alt >= MIN_ALTITUDE_DEG
    sep = _angle_sep_arcsec(az_ref[keep], alt_ref[keep], erfa_az[keep], erfa_alt[keep])
    assert sep.max() < REFRACTED_VS_ERFA_TOL_ARCSEC, f"max {sep.max()} arcsec"


@pytest.mark.parametrize("atmosphere", ATMOSPHERES, ids=["std", "high"])
@pytest.mark.parametrize("case", CASES, ids=CASE_IDS)
def test_refraction_shift_matches_astropy(case, atmosphere):
    """Astropy's refraction shift (pressure on minus off) against ours."""
    az_geo, alt_geo = _ours(case)
    az_ref, alt_ref = _ours_refracted(case, atmosphere)
    ast_geo = _astropy_atmosphere(case, None)
    ast_ref = _astropy_atmosphere(case, atmosphere)
    keep = alt_geo >= MIN_ALTITUDE_DEG
    ours_shift = (alt_ref - alt_geo)[keep] * 3600.0
    astropy_shift = (ast_ref[1] - ast_geo[1])[keep] * 3600.0
    worst = np.abs(ours_shift - astropy_shift).max()
    assert worst < SHIFT_VS_ASTROPY_TOL_ARCSEC, f"largest shift difference {worst} arcsec"
