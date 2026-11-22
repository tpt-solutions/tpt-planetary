"""Tests for the SGP4/SDP4 binding (tp.Satellite).

The states are the reference rows committed in
tpt-planetary-astro/tests/fixtures/sgp4_deep_ref.tsv (from the sgp4 package,
see tools/gen_sgp4_deep_ref.py): deep-space element sets at fixed times, with
the error code where the model reports one.
"""

from pathlib import Path

import numpy as np
import pytest

import tpt_planetary as tp

FIXTURE = Path(__file__).resolve().parents[3] / "tpt-planetary-astro" / "tests" / "fixtures" / "sgp4_deep_ref.tsv"


def rows():
    out = []
    for line in FIXTURE.read_text(encoding="utf-8").splitlines():
        if not line or line.startswith("#"):
            continue
        f = line.split("\t")
        out.append((f[0], f[1], float(f[2]), f[3], [float(x) for x in f[4:]]))
    return out


def test_deep_space_states_match_the_reference():
    checked = 0
    for l1, l2, minutes, status, want in rows():
        sat = tp.Satellite.from_tle(l1, l2)
        if status == "ok":
            pos, vel = sat.propagate(minutes)
            np.testing.assert_allclose(np.concatenate([pos, vel]), want, rtol=0, atol=1e-6)
            checked += 1
    assert checked > 200


def test_error_rows_raise_tptplanetary_error():
    errors = 0
    for l1, l2, minutes, status, _ in rows():
        if status != "ok":
            sat = tp.Satellite.from_tle(l1, l2)
            with pytest.raises(tp.TptPlanetaryError):
                sat.propagate(minutes)
            errors += 1
    assert errors > 0


def test_epoch_is_a_two_part_julian_date():
    l1, l2 = rows()[0][:2]
    day, frac = tp.Satellite.from_tle(l1, l2).epoch
    assert 2_400_000 < day + frac < 2_500_000


def test_bad_checksum_is_a_value_error():
    l1, l2 = rows()[0][:2]
    bad = l1[:68] + ("0" if l1[68] != "0" else "1") + l1[69:]
    with pytest.raises(ValueError):
        tp.Satellite.from_tle(bad, l2)
