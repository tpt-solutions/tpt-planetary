"""TT -> TDB, Earth Rotation Angle, GMST (IAU 2006) and mean obliquity.

References:
- ``erfa.dtdb`` (pyerfa): full Fairhead & Bretagnon TDB - TT series.
- ``erfa.era00``: ERA from UT1 (IAU 2000).
- ``erfa.gmst06``: GMST from UT1 and TT (IAU 2006).
- ``erfa.obl06``: IAU 2006 mean obliquity.

Points: TDB at 400 seeded random TT epochs in 2000-2026; ERA and GMST at 200
seeded random UT1 = UTC epochs in 2000-2026 with a stated UT1 - UTC of 0 and
TT = UT1 + 69.184 s; obliquity at J2000.0, +1 and +10 Julian centuries.

Tolerances (set before measuring):
- TDB: 1 microsecond against erfa.dtdb. The crate evaluates the full
  Fairhead & Bretagnon series, so the difference is at the level of rounding.
- ERA, GMST, obliquity: 1 milliarcsecond. Both sides use the same published
  formula, so the difference should be at the level of double-precision
  rounding; 1 mas is a loose bound for that.
"""

from __future__ import annotations

import erfa
import numpy as np
import pytest
import tpt_planetary as tp

from _reference import run_validation_dump

ARCSEC_PER_RAD = 180.0 * 3600.0 / np.pi
MAS = 1e-3  # arcseconds per milliarcsecond
TDB_VALIDATION_TOL_US = 1.0
ANGLE_TOL_ARCSEC = 1.0 * MAS

RNG = np.random.default_rng(20260101)


def _wrap_rad(x):
    return np.angle(np.exp(1j * x))


def _tdb_points():
    days = RNG.uniform(0.0, 26 * 365.25, 400)
    hi = 2451544.5 + np.floor(days)
    lo = days - np.floor(days)
    fixed_hi = np.array([2451544.5, 2455197.5, 2458849.5, 2460676.5])
    fixed_lo = np.array([0.5, 0.0, 0.25, 0.75])
    return np.r_[hi, fixed_hi], np.r_[lo, fixed_lo]


def _max_tdb_error_us():
    hi, lo = _tdb_points()
    ours = run_validation_dump("tdb", np.c_[hi, lo])[:, 0]
    ref = np.array([erfa.dtdb(h, l, 0.0, 0.0, 0.0, 0.0) for h, l in zip(hi, lo)])
    diff_us = (ours - ref) * 1e6
    return float(np.abs(diff_us).max()), float(diff_us.mean()), float(diff_us.std())


def test_tdb_minus_tt_matches_erfa_dtdb_to_1us():
    max_us, mean_us, std_us = _max_tdb_error_us()
    print(f"\nTDB-TT vs erfa.dtdb: max |diff| {max_us:.3e} us, mean {mean_us:.3e} us, std {std_us:.3e} us")
    assert max_us <= TDB_VALIDATION_TOL_US, f"max |diff| = {max_us:.3f} us"


def _era_gmst_points():
    """UT1 (= UTC) epochs as (day, fraction) pairs, and TT = UT1 + 69.184 s."""
    ut_day = 2451544.5 + np.floor(RNG.uniform(0.0, 26 * 365.25, 200))
    ut_frac = RNG.uniform(0.0, 1.0, 200)
    tt = ut_frac + 69.184 / 86400.0
    tt_day = ut_day + np.floor(tt)
    tt_frac = tt - np.floor(tt)
    return ut_day, ut_frac, tt_day, tt_frac


def test_earth_rotation_angle_matches_erfa_era00():
    ut_day, ut_frac, _, _ = _era_gmst_points()
    ours = np.array(
        [tp.earth_rotation_angle(tp.JulianDate(d, f)) for d, f in zip(ut_day, ut_frac)]
    )
    ref = np.array([erfa.era00(d, f) for d, f in zip(ut_day, ut_frac)])
    diff_arcsec = np.abs(_wrap_rad(ours - ref)) * ARCSEC_PER_RAD
    print(f"\nERA vs erfa.era00: max |diff| {diff_arcsec.max():.3e} arcsec")
    assert diff_arcsec.max() <= ANGLE_TOL_ARCSEC


def test_gmst_iau2006_matches_erfa_gmst06():
    ut_day, ut_frac, tt_day, tt_frac = _era_gmst_points()
    ours = np.array(
        [
            tp.gmst_iau2006(tp.JulianDate(d, f), tp.JulianDate(td, t))
            for d, f, td, t in zip(ut_day, ut_frac, tt_day, tt_frac)
        ]
    )
    ref = np.array(
        [erfa.gmst06(d, f, td, t) for d, f, td, t in zip(ut_day, ut_frac, tt_day, tt_frac)]
    )
    diff = np.abs(_wrap_rad(ours - ref))
    diff_arcsec = diff * ARCSEC_PER_RAD
    diff_ms_time = diff / (2 * np.pi) * 86400.0 * 1e3
    print(
        f"\nGMST vs erfa.gmst06: max |diff| {diff_arcsec.max():.3e} arcsec "
        f"({diff_ms_time.max():.3e} ms of time)"
    )
    assert diff_arcsec.max() <= ANGLE_TOL_ARCSEC


OBL_EPOCHS = [(2451545.0, 0.0), (2451545.0 + 36525.0, 0.0), (2451545.0 + 365250.0, 0.0)]


@pytest.mark.parametrize("label,epoch", list(zip(["J2000.0", "+1 century", "+10 centuries"], OBL_EPOCHS)))
def test_mean_obliquity_matches_erfa_obl06(label, epoch):
    ours = run_validation_dump("obl", np.array([epoch]))[0, 0]
    ref = erfa.obl06(*epoch) * ARCSEC_PER_RAD
    print(f"\nobliquity {label}: ours {ours!r} arcsec, erfa.obl06 {ref!r} arcsec, diff {ours - ref:.3e}")
    assert abs(ours - ref) <= ANGLE_TOL_ARCSEC
