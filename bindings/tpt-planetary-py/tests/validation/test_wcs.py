"""Celestial WCS (TAN, SIN, AIT) against Astropy's wcslib (``astropy.wcs.WCS``).

Reference: ``astropy.wcs.WCS`` (wcslib) with ``wcs_pix2world`` and
``wcs_world2pix`` using FITS origin 1, no distortion.

Headers exercised (LONPOLE and LATPOLE left out, so the defaults of
Calabretta & Greisen 2002, Table 2, apply):
- TAN, RA/Dec, CDELT with a PC identity, reference at high declination;
- SIN with a CD matrix containing a rotation, reference in the southern sky;
- AIT with a reference at dec -60, where the default LONPOLE is 180.

Points: 2000 seeded random pixels inside the image for pixel -> world, and
2000 seeded random sky positions for world -> pixel (within 6 deg of CRVAL for
TAN and SIN, anywhere in the domain for AIT). Positions outside a projection's
domain must be NaN in both implementations.

Tolerances (set before measuring):
- pixel -> world: 1e-9 degrees (3.6 microarcseconds). Both sides evaluate the
  same closed-form projection in double precision.
- world -> pixel: 1e-6 pixels, for the same reason.
"""

from __future__ import annotations

import numpy as np
import pytest
import tpt_planetary as tp
from astropy import units as u
from astropy.io import fits
from astropy.wcs import WCS

PIX_TOL = 1e-6
DEG_TOL = 1e-9
RNG = np.random.default_rng(20260103)

HEADERS = {
    "TAN_high_dec": {
        "CTYPE1": "RA---TAN", "CTYPE2": "DEC--TAN",
        "CRVAL1": 150.0, "CRVAL2": 80.0,
        "CRPIX1": 512.5, "CRPIX2": 512.5,
        "CDELT1": -0.0005, "CDELT2": 0.0005,
        "PC1_1": 1.0, "PC1_2": 0.0, "PC2_1": 0.0, "PC2_2": 1.0,
        "CUNIT1": "deg", "CUNIT2": "deg",
    },
    "SIN_rotated_CD": {
        "CTYPE1": "RA---SIN", "CTYPE2": "DEC--SIN",
        "CRVAL1": 210.0, "CRVAL2": -35.0,
        "CRPIX1": 300.0, "CRPIX2": 200.0,
        "CD1_1": -0.0004 * np.cos(np.radians(30.0)), "CD1_2": -0.0004 * np.sin(np.radians(30.0)),
        "CD2_1": 0.0004 * np.sin(np.radians(30.0)), "CD2_2": 0.0004 * np.cos(np.radians(30.0)),
    },
    "AIT_south": {
        "CTYPE1": "GLON-AIT", "CTYPE2": "GLAT-AIT",
        "CRVAL1": 0.0, "CRVAL2": -60.0,
        "CRPIX1": 180.5, "CRPIX2": 90.5,
        "CDELT1": -1.0, "CDELT2": 1.0,
    },
}
# GLON/GLAT are not celestial CTYPEs for the RA/Dec reader, so use RA/DEC names
# for AIT too; the projection code is what matters for the test.
HEADERS["AIT_south"]["CTYPE1"] = "RA---AIT"
HEADERS["AIT_south"]["CTYPE2"] = "DEC--AIT"


def _ours(header):
    return tp.Wcs.from_header(header)


def _astropy(header):
    cards = [(k, v) for k, v in header.items()]
    return WCS(fits.Header(cards))


def _angle_deg(lon1, lat1, lon2, lat2):
    """Great-circle separation in degrees; NaN where either input is NaN."""
    a = np.radians
    l1, b1, l2, b2 = a(lon1), a(lat1), a(lon2), a(lat2)
    cross = np.hypot(
        np.cos(b2) * np.sin(l2 - l1),
        np.cos(b1) * np.sin(b2) - np.sin(b1) * np.cos(b2) * np.cos(l2 - l1),
    )
    dot = np.sin(b1) * np.sin(b2) + np.cos(b1) * np.cos(b2) * np.cos(l2 - l1)
    return np.degrees(np.arctan2(cross, dot))


def _pixel_points(header):
    nx = header["CRPIX1"] * 2.0
    ny = header["CRPIX2"] * 2.0
    x = RNG.uniform(0.5, nx, 2000)
    y = RNG.uniform(0.5, ny, 2000)
    return x, y


def _world_points(header, ref):
    n = 2000
    if header["CTYPE1"].endswith("AIT"):
        lon = RNG.uniform(0.0, 360.0, n)
        lat = np.degrees(np.arcsin(RNG.uniform(-1.0, 1.0, n)))
    else:
        ra0, dec0 = ref.wcs.crval
        ang = RNG.uniform(0.0, 2 * np.pi, n)
        rad = np.radians(6.0) * np.sqrt(RNG.uniform(0.0, 1.0, n))
        # Offset from the reference point by (rad, ang) on the sphere.
        d0 = np.radians(dec0)
        dec = np.arcsin(np.sin(d0) * np.cos(rad) + np.cos(d0) * np.sin(rad) * np.cos(ang))
        lon = (np.radians(ra0) + np.arctan2(
            np.sin(ang) * np.sin(rad) * np.cos(d0),
            np.cos(rad) - np.sin(d0) * np.sin(dec),
        )) % (2 * np.pi)
        lat = np.degrees(dec)
        lon = np.degrees(lon)
    return lon, lat


@pytest.mark.parametrize("name", list(HEADERS))
def test_pixel_to_world_matches_wcslib(name):
    header = HEADERS[name]
    ours = _ours(header)
    ref = _astropy(header)
    x, y = _pixel_points(header)
    lon_o, lat_o = ours.pixel_to_world(x, y)
    lon_r, lat_r = ref.wcs_pix2world(x, y, 1)
    finite_o = np.isfinite(lon_o)
    finite_r = np.isfinite(lat_r)
    assert np.array_equal(finite_o, finite_r), f"{name}: NaN masks differ"
    sep = _angle_deg(lon_o[finite_o], lat_o[finite_o], lon_r[finite_r], lat_r[finite_r])
    print(f"\n{name} pixel->world vs wcslib: max {sep.max():.3e} deg, "
          f"{finite_o.sum()} finite of {x.size}")
    assert sep.max() <= DEG_TOL


@pytest.mark.parametrize("name", list(HEADERS))
def test_world_to_pixel_matches_wcslib(name):
    header = HEADERS[name]
    ours = _ours(header)
    ref = _astropy(header)
    lon, lat = _world_points(header, ref)
    x_o, y_o = ours.world_to_pixel(lon, lat)
    x_r, y_r = ref.wcs_world2pix(lon, lat, 1)
    finite_o = np.isfinite(x_o)
    finite_r = np.isfinite(x_r)
    if header["CTYPE1"].endswith("AIT"):
        # Points on the Hammer edge sit on the antimeridian cut, where the sign
        # of x is a convention and the two implementations may pick opposite
        # signs. They are excluded here. The header is diagonal, so the
        # intermediate coordinates are CDELT * (pixel - CRPIX).
        xi = header["CDELT1"] * (x_r - header["CRPIX1"]) / (180.0 / np.pi)
        yi = header["CDELT2"] * (y_r - header["CRPIX2"]) / (180.0 / np.pi)
        metric = xi**2 / 8.0 + yi**2 / 2.0
        finite_r = finite_r & (np.abs(metric - 1.0) > 1e-6)
        finite_o = finite_o & (np.abs(metric - 1.0) > 1e-6)
    assert np.array_equal(finite_o, finite_r), f"{name}: NaN masks differ"
    dpix = np.hypot(x_o[finite_o] - x_r[finite_r], y_o[finite_o] - y_r[finite_r])
    print(f"\n{name} world->pixel vs wcslib: max {dpix.max():.3e} px, "
          f"{finite_o.sum()} finite of {lon.size}")
    assert dpix.max() <= PIX_TOL


def test_default_lonpole_for_southern_ait_is_180():
    # With theta0 = 0 and CRVAL2 < 0, the C&G default LONPOLE is 180 deg.
    header = HEADERS["AIT_south"]
    ours = _ours(header)
    ref = _astropy(header)
    assert ref.wcs.lonpole == 180.0
    x, y = ours.world_to_pixel(np.array([0.0]), np.array([-60.0]))
    xr, yr = ref.wcs_world2pix(np.array([0.0]), np.array([-60.0]), 1)
    assert np.hypot(x[0] - xr[0], y[0] - yr[0]) <= PIX_TOL
