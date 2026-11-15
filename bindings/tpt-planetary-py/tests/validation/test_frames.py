"""ICRS <-> Galactic and ICRS -> J2000 mean ecliptic.

References:
- ``astropy.coordinates`` ``ICRS`` -> ``Galactic`` (pole and node taken from
  Astropy's own frame definition).
- ``astropy.coordinates`` ``BarycentricMeanEcliptic(equinox="J2000")`` and
  ``GeocentricMeanEcliptic(equinox="J2000")``.
- ``erfa.obl06``, ``erfa.rx`` and ``erfa.bp06``: an ERFA construction of the
  J2000 mean ecliptic, with and without the ICRS -> J2000 frame bias.

Points: 1000 seeded uniform points on the sphere for Galactic, 500 for the
ecliptic.

Tolerances (set before measuring):
- Galactic: 0.1 arcsec. The crate uses the published IAU 1958 / Hipparcos pole
  (192.85948, 27.12825) and node (122.93192), each rounded to the fifth decimal
  (up to 0.000005 deg = 18 mas each). The ICRS and FK5-J2000 pole definitions
  differ by about 17 mas (frame bias). The sum of these worst cases is below
  0.1 arcsec.
- Ecliptic against the ERFA definition without frame bias: 1 milliarcsecond.
  This is the same construction, so only rounding should remain.
- Ecliptic against Astropy's barycentric frame: 1 milliarcsecond. The crate
  documents MeanEcliptic as the J2000 obliquity rotation of ICRS with no frame
  bias, while Astropy's frame includes the ICRS -> J2000 bias. This test is
  expected to fail and is marked xfail with the measured value.
"""

from __future__ import annotations

import erfa
import numpy as np
import pytest
import tpt_planetary as tp
from astropy.coordinates import BarycentricMeanEcliptic, GeocentricMeanEcliptic, SkyCoord
import astropy.units as u

from _reference import run_validation_dump

ARCSEC_PER_RAD = 180.0 * 3600.0 / np.pi
GAL_TOL_ARCSEC = 0.1
ECL_TOL_ARCSEC = 1e-3

RNG = np.random.default_rng(20260102)


def _sphere_points(n):
    ra = RNG.uniform(0.0, 360.0, n)
    dec = np.degrees(np.arcsin(RNG.uniform(-1.0, 1.0, n)))
    return ra, dec


def _unit(lon_deg, lat_deg):
    lon, lat = np.radians(lon_deg), np.radians(lat_deg)
    return np.c_[np.cos(lat) * np.cos(lon), np.cos(lat) * np.sin(lon), np.sin(lat)]


def _angle_arcsec(v1, v2):
    cross = np.linalg.norm(np.cross(v1, v2), axis=1)
    dot = np.sum(v1 * v2, axis=1)
    return np.arctan2(cross, dot) * ARCSEC_PER_RAD


def test_icrs_to_galactic_matches_astropy():
    ra, dec = _sphere_points(1000)
    l_ours, b_ours = tp.icrs_to_galactic(ra, dec)
    ref = SkyCoord(ra * u.deg, dec * u.deg, frame="icrs").galactic
    diff = _angle_arcsec(_unit(l_ours, b_ours), _unit(ref.l.deg, ref.b.deg))
    print(f"\nICRS->Galactic vs astropy: max {diff.max():.4f} arcsec, mean {diff.mean():.4f} arcsec")
    assert diff.max() <= GAL_TOL_ARCSEC


def test_galactic_to_icrs_matches_astropy():
    l, b = _sphere_points(1000)
    ra_ours, dec_ours = tp.galactic_to_icrs(l, b)
    ref = SkyCoord(l * u.deg, b * u.deg, frame="galactic").icrs
    diff = _angle_arcsec(_unit(ra_ours, dec_ours), _unit(ref.ra.deg, ref.dec.deg))
    print(f"\nGalactic->ICRS vs astropy: max {diff.max():.4f} arcsec, mean {diff.mean():.4f} arcsec")
    assert diff.max() <= GAL_TOL_ARCSEC


def _ecliptic_points():
    ra, dec = _sphere_points(500)
    ours = run_validation_dump("ecl", np.c_[ra, dec])
    return ra, dec, ours


def test_mean_ecliptic_matches_erfa_construction_without_bias():
    ra, dec, ours = _ecliptic_points()
    eps = erfa.obl06(2451545.0, 0.0)
    rot = erfa.rx(eps, np.eye(3))
    r = _unit(ra, dec) @ rot.T
    ref_lon = np.degrees(np.arctan2(r[:, 1], r[:, 0])) % 360.0
    ref_lat = np.degrees(np.arcsin(np.clip(r[:, 2], -1.0, 1.0)))
    dlon = np.abs(np.angle(np.exp(1j * np.radians(ours[:, 0] - ref_lon)))) * ARCSEC_PER_RAD
    dlat = np.abs(ours[:, 1] - ref_lat) * 3600.0
    print(f"\necliptic vs ERFA (no bias): lon max {dlon.max():.3e} arcsec, lat max {dlat.max():.3e} arcsec")
    assert max(dlon.max(), dlat.max()) <= ECL_TOL_ARCSEC


@pytest.mark.xfail(
    strict=True,
    reason=(
        "MeanEcliptic is the ICRS-axes J2000 obliquity rotation with no frame bias "
        "(documented in the crate). Astropy's BarycentricMeanEcliptic includes the "
        "ICRS -> J2000 frame bias (about 17 mas). Measured: lon 0.256 arcsec "
        "(largest near the ecliptic poles), lat 0.022 arcsec. An ERFA construction "
        "with the bp06 bias matches Astropy to below 1e-5 arcsec, so the gap is "
        "definitional, not a coding error; see docs/validation.md."
    ),
)
def test_mean_ecliptic_matches_astropy_barycentric_to_1mas():
    ra, dec, ours = _ecliptic_points()
    ref = SkyCoord(ra * u.deg, dec * u.deg, frame="icrs").transform_to(
        BarycentricMeanEcliptic(equinox="J2000")
    )
    dlon = np.abs(np.angle(np.exp(1j * np.radians(ours[:, 0] - ref.lon.deg)))) * ARCSEC_PER_RAD
    dlat = np.abs(ours[:, 1] - ref.lat.deg) * 3600.0
    print(f"\necliptic vs astropy BarycentricMeanEcliptic: lon max {dlon.max():.4f}, lat max {dlat.max():.4f} arcsec")
    assert max(dlon.max(), dlat.max()) <= ECL_TOL_ARCSEC


@pytest.mark.xfail(
    strict=True,
    reason=(
        "GeocentricMeanEcliptic is a geocentric frame (Astropy applies the observer's "
        "velocity aberration at obstime), while MeanEcliptic is a barycentric "
        "direction. Measured offset is shown in the test output."
    ),
)
def test_mean_ecliptic_matches_astropy_geocentric_to_1mas():
    ra, dec, ours = _ecliptic_points()
    ref = SkyCoord(ra * u.deg, dec * u.deg, frame="icrs").transform_to(
        GeocentricMeanEcliptic(equinox="J2000")
    )
    dlon = np.abs(np.angle(np.exp(1j * np.radians(ours[:, 0] - ref.lon.deg)))) * ARCSEC_PER_RAD
    dlat = np.abs(ours[:, 1] - ref.lat.deg) * 3600.0
    print(f"\necliptic vs astropy GeocentricMeanEcliptic: lon max {dlon.max():.3f}, lat max {dlat.max():.3f} arcsec")
    assert max(dlon.max(), dlat.max()) <= ECL_TOL_ARCSEC


def test_erfa_ecliptic_with_frame_bias_matches_astropy_barycentric():
    """Explains the xfail above: adding the ERFA frame bias reproduces Astropy."""
    ra, dec = _sphere_points(500)
    eps = erfa.obl06(2451545.0, 0.0)
    rb, _, _ = erfa.bp06(2451545.0, 0.0)
    r = _unit(ra, dec) @ (erfa.rx(eps, np.eye(3)) @ rb).T
    ref_lon = np.degrees(np.arctan2(r[:, 1], r[:, 0])) % 360.0
    ref_lat = np.degrees(np.arcsin(np.clip(r[:, 2], -1.0, 1.0)))
    ap = SkyCoord(ra * u.deg, dec * u.deg, frame="icrs").transform_to(
        BarycentricMeanEcliptic(equinox="J2000")
    )
    dlon = np.abs(np.angle(np.exp(1j * np.radians(ref_lon - ap.lon.deg)))) * ARCSEC_PER_RAD
    dlat = np.abs(ref_lat - ap.lat.deg) * 3600.0
    print(f"\nERFA with bp06 bias vs astropy barycentric: lon max {dlon.max():.3e}, lat max {dlat.max():.3e} arcsec")
    assert max(dlon.max(), dlat.max()) <= ECL_TOL_ARCSEC
