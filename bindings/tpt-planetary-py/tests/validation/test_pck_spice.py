"""Body orientation from a binary PCK against SPICE's pxform.

The reference is SPICE (spiceypy, which ships CSPICE): pxform('J2000',
'ITRF93', et) for the Earth orientation kernel earth_latest_high_prec.bpc
(NAIF), at epochs from 2000 to 2026, the kernel's coverage. The crate evaluates the kernel's type-2
Chebyshev records, converts the 3-1-3 Euler angles with the ECLIPJ2000
obliquity, and returns the 3 x 3 matrix.

The test needs environment variables TPT_BPC_KERNEL (the .bpc file) and TPT_LSK
(the NAIF leap-second kernel naif0012.tls, for SPICE's time conversion), and the
spiceypy package. It is skipped without them.

Tolerance: 1e-12 per matrix element. The measured maximum is about 1e-15.
"""

from __future__ import annotations

import os

import numpy as np
import pytest

import tpt_planetary as tp

BPC = os.environ.get("TPT_BPC_KERNEL")
LSK = os.environ.get("TPT_LSK")
ITRF93 = 3000
TOLERANCE = 1e-12

spiceypy = pytest.importorskip("spiceypy", reason="spiceypy (SPICE) is the reference")
needs_kernels = pytest.mark.skipif(
    not (BPC and LSK), reason="set TPT_BPC_KERNEL and TPT_LSK to the NAIF kernels"
)


@pytest.fixture(scope="module")
def spice():
    spiceypy.furnsh(LSK)
    spiceypy.furnsh(BPC)
    yield spiceypy
    spiceypy.kclear()


@needs_kernels
def test_itrf93_matrix_matches_spice_pxform(spice):
    kernel = tp.open_spk(BPC)
    # Epochs across the kernel's coverage (2000 to 2027), in TDB seconds past J2000.
    epochs = [spice.str2et(f"{year}-{month:02d}-15T12:00:00") for year in range(2000, 2027, 2) for month in (2, 9)]
    worst = 0.0
    for et in epochs:
        ours = np.array(kernel.orientation(ITRF93, et))
        ref = np.array(spice.pxform("J2000", "ITRF93", et))
        worst = max(worst, float(np.abs(ours - ref).max()))
    print(f"\nITRF93 matrix vs SPICE pxform over {len(epochs)} epochs: max |diff| {worst:.3e}")
    assert worst < TOLERANCE, f"max difference {worst}"


@needs_kernels
def test_matrix_is_a_rotation(spice):
    kernel = tp.open_spk(BPC)
    et = spice.str2et("2017-01-01T00:00:00")
    m = np.array(kernel.orientation(ITRF93, et))
    assert np.abs(m @ m.T - np.eye(3)).max() < 1e-14
    assert abs(np.linalg.det(m) - 1.0) < 1e-14
