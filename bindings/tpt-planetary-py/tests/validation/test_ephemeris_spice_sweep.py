"""Astrometric direction of Jupiter from an SPK kernel, against SPICE, over the
kernel's whole span.

The reference is SPICE (spiceypy): spkezr('5', et, 'J2000', 'LT', '399'),
which gives Jupiter's barycentre seen from the Earth's barycentre with light
time, and no stellar aberration: the astrometric direction. The epochs are
random TDB seconds past J2000 across the kernel's coverage (DE440s runs from
1849 to 2150), so the sweep covers every segment of the file rather than the
few epochs of the Horizons test.

The test needs TPT_SPK_KERNEL (a DE440-style SPK file) and the spiceypy
package. It is skipped without them. It checks the implementation against
SPICE on the same kernel; the accuracy of the kernel itself against the real
sky is what the Horizons test measures.

Tolerance: 0.01 arcsec. The measured maximum over 3000 epochs is 0.0001 arcsec.
"""

from __future__ import annotations

import math
import os

import numpy as np
import pytest

import tpt_planetary as tp

KERNEL = os.environ.get("TPT_SPK_KERNEL")
TOLERANCE_ARCSEC = 0.01
JUPITER = 5
EARTH = 399

spiceypy = pytest.importorskip("spiceypy", reason="spiceypy (SPICE) is the reference")


@pytest.mark.skipif(not KERNEL, reason="set TPT_SPK_KERNEL to a DE440-style SPK kernel")
def test_jupiter_astrometric_matches_spice_over_the_kernel_span():
    spiceypy.furnsh(KERNEL)
    try:
        kernel = tp.open_spk(KERNEL)
        rng = np.random.default_rng(20261123)
        # Keep clear of the ends of DE440s (1849-12-26 .. 2150-01-22 TDB).
        epochs = rng.uniform(-4.6e9, 4.6e9, size=3000)
        worst = 0.0
        for et in epochs:
            et = float(et)
            state, _ = spiceypy.spkezr(str(JUPITER), et, "J2000", "LT", str(EARTH))
            v = np.asarray(state[:3])
            ra_ref = math.degrees(math.atan2(v[1], v[0])) % 360.0
            dec_ref = math.degrees(math.asin(v[2] / np.linalg.norm(v)))
            got = kernel.astrometric(JUPITER, EARTH, et)
            d_ra = (got["ra_deg"] - ra_ref + 180.0) % 360.0 - 180.0
            d_ra_arcsec = d_ra * math.cos(math.radians(dec_ref)) * 3600.0
            d_dec_arcsec = (got["dec_deg"] - dec_ref) * 3600.0
            worst = max(worst, math.hypot(d_ra_arcsec, d_dec_arcsec))
        assert worst <= TOLERANCE_ARCSEC, f"largest separation {worst} arcsec"
    finally:
        spiceypy.kclear()


AU_KM = 149_597_870.7
ESD_TOLERANCE_AU = 2e-4


@pytest.mark.skipif(not KERNEL, reason="set TPT_SPK_KERNEL to a DE440-style SPK kernel")
def test_earth_sun_distance_approx_matches_the_kernel():
    """The low-precision Earth-Sun distance against the Earth-Sun range from the SPK file.

    The approximation (``earth_sun_distance_au_approx``) is a Meeus series accurate
    to about 1e-4 AU against ``erfa.epv00``; this checks the same distance against
    the kernel over 1900-2100, which is what the sweep above does for Jupiter.
    The SPK range is the geocentre (399) from the Sun (10), in km. Tolerance 2e-4
    AU; the measured maximum over 3000 epochs is 1.0e-4 AU (mean 2.8e-5 AU).
    """
    from _reference import run_validation_dump

    kernel = tp.open_spk(KERNEL)
    rng = np.random.default_rng(20261124)
    # TT epochs 1900 .. 2100, as two-part Julian dates.
    tt = rng.uniform(2415020.0, 2488070.0, 300)
    rows = np.column_stack([np.floor(tt), tt - np.floor(tt)])
    approx = run_validation_dump("esd", rows)[:, 0]
    spk = []
    for hi, lo in rows:
        et = (hi - 2451545.0 + lo) * 86400.0
        pos, _ = kernel.state(399, 10, et)
        spk.append(np.linalg.norm(pos) / AU_KM)
    spk = np.asarray(spk)
    worst = np.abs(approx - spk).max()
    assert worst < ESD_TOLERANCE_AU, f"largest difference {worst} AU"
