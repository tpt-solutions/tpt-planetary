"""Astrometric positions of Jupiter and Saturn against JPL Horizons.

The reference values are JPL Horizons astrometric right ascension and
declination (geocentric, ICRF, light-time corrected, no stellar aberration),
fetched on 2026-11-22 with TIME_TYPE TT at the epochs listed below. Horizons
prints RA and Dec to 1e-5 degree (0.036 arcsec), so the reference itself is
good to about 0.02 arcsec per coordinate.

The crate reads a DE440-style SPK kernel, which is not bundled (it is 33 MB
and is user-supplied). Set TPT_SPK_KERNEL to its path, for example de440s.bsp
from https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/planets/. The
tests are skipped without it. The epochs are read as TDB by the CLI; TT and
TDB differ by under 2 ms, which moves these directions by well under 0.001
arcsec.

Tolerance: 0.05 arcsec per coordinate, which is the Horizons output precision
plus margin. The measured maximum is 0.014 arcsec.
"""

from __future__ import annotations

import json
import math
import os
import subprocess

import pytest
from _reference import REPO_ROOT

KERNEL = os.environ.get("TPT_SPK_KERNEL")
TOLERANCE_ARCSEC = 0.05

# (NAIF id, epoch as ISO 8601 TT, Horizons RA deg, Horizons Dec deg)
HORIZONS = [
    (5, "2017-01-01T00:00:00", 199.80329, -6.97280),
    (5, "2025-06-15T12:00:00", 91.00977, 23.27844),
    (6, "2025-06-15T12:00:00", 1.72703, -1.61384),
    (5, "1950-01-01T00:00:00", 309.76922, -19.03902),
    (6, "1950-01-01T00:00:00", 171.72705, 5.75298),
]


def astrometric(body: int, epoch: str) -> tuple[float, float]:
    """RA and Dec in degrees from the CLI's astrometric query."""
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
            "--astrometric",
        ],
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
        check=False,
    )
    assert proc.returncode == 0, proc.stderr
    doc = json.loads(proc.stdout)
    return float(doc["ra_deg"]), float(doc["dec_deg"])


@pytest.mark.skipif(not KERNEL, reason="set TPT_SPK_KERNEL to a DE440-style SPK kernel")
@pytest.mark.parametrize("body, epoch, ra_ref, dec_ref", HORIZONS)
def test_astrometric_direction_matches_horizons(body, epoch, ra_ref, dec_ref):
    ra, dec = astrometric(body, epoch)
    d_ra = (ra - ra_ref + 180.0) % 360.0 - 180.0
    d_ra_arcsec = d_ra * math.cos(math.radians(dec_ref)) * 3600.0
    d_dec_arcsec = (dec - dec_ref) * 3600.0
    assert abs(d_ra_arcsec) <= TOLERANCE_ARCSEC, f"RA off by {d_ra_arcsec} arcsec"
    assert abs(d_dec_arcsec) <= TOLERANCE_ARCSEC, f"Dec off by {d_dec_arcsec} arcsec"
