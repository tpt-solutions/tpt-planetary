"""Tests for the SPK kernel bindings (open_spk, SpkKernel, et_from_tdb).

The kernel-dependent tests need a DE440-style kernel in TPT_SPK_KERNEL and are
skipped without one. The Jupiter direction is compared with the JPL Horizons
astrometric value for 2017-01-01 00:00 TT (see test_ephemeris_horizons.py).
"""

import math
import os

import pytest

import tpt_planetary as tp

KERNEL = os.environ.get("TPT_SPK_KERNEL")
needs_kernel = pytest.mark.skipif(not KERNEL, reason="set TPT_SPK_KERNEL to a DE440-style SPK kernel")


def test_et_from_tdb_is_seconds_past_j2000():
    assert tp.et_from_tdb(2451545.0, 0.0) == pytest.approx(0.0, abs=1e-9)
    assert tp.et_from_tdb(2451546.0, 0.0) == pytest.approx(86400.0, abs=1e-9)


def test_et_from_tdb_rejects_non_finite_parts():
    with pytest.raises(ValueError):
        tp.et_from_tdb(float("nan"), 0.0)


def test_missing_kernel_is_os_error(tmp_path):
    with pytest.raises(OSError):
        tp.open_spk(tmp_path / "missing.bsp")


def test_non_spk_file_is_rejected(tmp_path):
    bad = tmp_path / "bad.bsp"
    bad.write_bytes(b"not a DAF file" * 100)
    with pytest.raises(tp.TptPlanetaryError):
        tp.open_spk(bad)


@needs_kernel
def test_state_is_a_pair_of_three_tuples():
    kernel = tp.open_spk(KERNEL)
    pos, vel = kernel.state(5, 0, 0.0)
    assert len(pos) == 3 and len(vel) == 3
    assert all(math.isfinite(x) for x in pos + vel)
    # Jupiter is several AU from the barycentre's origin, so the distance is large.
    assert math.hypot(*pos) > 1e8


@needs_kernel
def test_astrometric_jupiter_matches_horizons_at_2017():
    kernel = tp.open_spk(KERNEL)
    et = tp.et_from_tdb(2457754.5, 0.0)  # 2017-01-01 00:00 TDB (the TT value differs by 2 ms)
    d = kernel.astrometric(5, 399, et)
    # Horizons: RA 199.80329, Dec -6.97280 (1e-5 degree output precision).
    d_ra = (d["ra_deg"] - 199.80329 + 180.0) % 360.0 - 180.0
    assert abs(d_ra * math.cos(math.radians(-6.97280)) * 3600.0) < 0.05
    assert abs((d["dec_deg"] + 6.97280) * 3600.0) < 0.05
    assert d["light_time_s"] == pytest.approx(2767.9, abs=0.5)


@needs_kernel
def test_apparent_differs_from_astrometric_by_aberration_scale():
    kernel = tp.open_spk(KERNEL)
    et = tp.et_from_tdb(2457754.5, 0.0)
    a = kernel.astrometric(5, 399, et)
    p = kernel.apparent(5, 399, et)
    d_ra = (p["ra_deg"] - a["ra_deg"] + 180.0) % 360.0 - 180.0
    sep = math.hypot(d_ra * math.cos(math.radians(a["dec_deg"])), p["dec_deg"] - a["dec_deg"]) * 3600.0
    # Annual aberration is at most about 20.5 arcseconds.
    assert 0.1 < sep < 21.0
    assert p["light_time_s"] == pytest.approx(a["light_time_s"], rel=1e-12)
