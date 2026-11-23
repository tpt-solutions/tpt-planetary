"""Apparent directions of Jupiter and Saturn against JPL Horizons.

The reference values are JPL Horizons apparent right ascension and declination
(QUANTITIES 2: airless, geocentric, light-time corrected, with the Sun's light
deflection and the annual aberration applied), fetched on 2026-11-22 at TIME_TYPE
TT. Horizons gives these with respect to the Earth's true equator and equinox of
date, so the crate's ICRF apparent direction is rotated to that frame with the
IAU 2006/2000A precession-nutation matrix (ERFA's pnm06a) before comparison.

Like the astrometric test, this needs a DE440-style kernel in TPT_SPK_KERNEL and
is skipped without one. Tolerance: 0.15 arcsec per coordinate. Horizons rounds
to 1e-5 degree (0.036 arcsec); the measured maximum difference is 0.11 arcsec,
at 1950. The aberration is ERFA's eraAb, which includes the relativistic terms,
and it changed the residual by less than 0.001 arcsec. The remaining RA offset
of about +0.05 arcsec is reproduced by an independent chain: pyerfa's ld, ab
and pnm06a applied to the kernel vectors agree with the crate to 1e-4 arcsec.
So the residual comes from the model, not the code. Light deflection is
ruled out as its cause: without it the RA residuals are 0.02 to 0.09 arcsec,
no closer overall, and the offset on Jupiter at 2025 changes sign. The cause
of the common +0.05 arcsec is not identified; Horizons' own model is not
visible from outside.
"""

from __future__ import annotations

import json
import math
import os
import subprocess

import erfa
import numpy as np
import pytest
from _reference import REPO_ROOT

KERNEL = os.environ.get("TPT_SPK_KERNEL")
TOLERANCE_ARCSEC = 0.15

# (NAIF id, epoch as ISO 8601 TT, Horizons apparent RA deg, Horizons apparent Dec deg)
HORIZONS_APPARENT = [
    (5, "2017-01-01T00:00:00", 200.02266, -7.05998),
    (5, "2025-06-15T12:00:00", 91.39146, 23.27787),
    (6, "2025-06-15T12:00:00", 2.05291, -1.47198),
    (5, "1950-01-01T00:00:00", 309.04838, -19.21912),
    (6, "1950-01-01T00:00:00", 171.08346, 6.02799),
]


def apparent_icrf(body: int, epoch: str) -> tuple[float, float]:
    """ICRF apparent RA and Dec in degrees from the CLI's --apparent query."""
    proc = subprocess.run(
        [
            "cargo",
            "run",
            "--quiet",
            "-p",
            "tpt-planetary-cli",
            "--",
            "--format",
            "json",
            "ephemeris",
            "query",
            str(body),
            epoch,
            "--kernel",
            KERNEL,
            "--apparent",
        ],
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
        check=False,
    )
    assert proc.returncode == 0, proc.stderr
    doc = json.loads(proc.stdout)
    return float(doc["ra_deg"]), float(doc["dec_deg"])


def to_true_of_date(ra_deg: float, dec_deg: float, epoch: str) -> tuple[float, float]:
    """Rotate an ICRS direction to the true equator and equinox of date."""
    y, mo, d = int(epoch[0:4]), int(epoch[5:7]), int(epoch[8:10])
    hh, mm, ss = int(epoch[11:13]), int(epoch[14:16]), int(epoch[17:19])
    jd1, jd2 = erfa.dtf2d("TT", y, mo, d, hh, mm, ss)
    rc = erfa.pnm06a(jd1, jd2)
    ra, dec = math.radians(ra_deg), math.radians(dec_deg)
    v = np.array([math.cos(dec) * math.cos(ra), math.cos(dec) * math.sin(ra), math.sin(dec)])
    w = rc @ v
    return math.degrees(math.atan2(w[1], w[0])) % 360.0, math.degrees(math.asin(w[2]))


@pytest.mark.skipif(not KERNEL, reason="set TPT_SPK_KERNEL to a DE440-style SPK kernel")
@pytest.mark.parametrize("body, epoch, ra_ref, dec_ref", HORIZONS_APPARENT)
def test_apparent_direction_matches_horizons(body, epoch, ra_ref, dec_ref):
    ra, dec = apparent_icrf(body, epoch)
    ra, dec = to_true_of_date(ra, dec, epoch)
    d_ra = (ra - ra_ref + 180.0) % 360.0 - 180.0
    d_ra_arcsec = d_ra * math.cos(math.radians(dec_ref)) * 3600.0
    d_dec_arcsec = (dec - dec_ref) * 3600.0
    assert abs(d_ra_arcsec) <= TOLERANCE_ARCSEC, f"RA off by {d_ra_arcsec} arcsec"
    assert abs(d_dec_arcsec) <= TOLERANCE_ARCSEC, f"Dec off by {d_dec_arcsec} arcsec"
