"""Leap seconds against ERFA (``erfa.dat``) and Astropy (``LeapSeconds``).

References: pyerfa ``erfa.dat`` (IERS leap-second history built into ERFA) and
``astropy.utils.iers.LeapSeconds.from_iers_leap_seconds()`` (the IERS
leap-seconds.list file bundled with Astropy, read offline).

Tolerance: TAI - UTC must be an exact integer number of seconds. The Python
binding computes TAI - UTC through two-part Julian dates, so the returned value
can differ from the integer by float rounding (about 4e-12 s measured). The
tests therefore require the rounded value to be exact and the residual to be
below 1 ns.

Points: every 1 January and 1 July from 1972 to 2017, plus the day before each
step, all at 00:00 UTC. The embedded table starts at 1972-01-01, so earlier
dates are checked separately for the documented error.

Python exposes only the entry count and the last value of the table, not the
entries, so the steps are checked through the conversion results.
"""

from __future__ import annotations

import datetime

import erfa
import pytest
import tpt_planetary as tp
from astropy.utils.iers import LeapSeconds

# Steps (TAI - UTC changes) in the IERS history, as (year, month, day).
STEP_DATES = [
    (1972, 1, 1), (1972, 7, 1), (1973, 1, 1), (1974, 1, 1), (1975, 1, 1),
    (1976, 1, 1), (1977, 1, 1), (1978, 1, 1), (1979, 1, 1), (1980, 1, 1),
    (1981, 7, 1), (1982, 7, 1), (1983, 7, 1), (1985, 7, 1), (1988, 1, 1),
    (1990, 1, 1), (1991, 1, 1), (1992, 7, 1), (1993, 7, 1), (1994, 7, 1),
    (1996, 1, 1), (1997, 7, 1), (1999, 1, 1), (2006, 1, 1), (2009, 1, 1),
    (2012, 7, 1), (2015, 7, 1), (2017, 1, 1),
]  # fmt: skip

RESIDUAL_TOL_S = 1e-9


def _check_dates() -> list[tuple[int, int, int]]:
    """Every 1 Jan and 1 Jul in 1972-2017, plus the day before each step (1972 onward)."""
    half_years = [(y, m, 1) for y in range(1972, 2018) for m in (1, 7)]
    half_years = [d for d in half_years if d <= (2017, 7, 1)]
    before = []
    for y, m, d in STEP_DATES:
        prev = datetime.date(y, m, d) - datetime.timedelta(days=1)
        if prev.year >= 1972:
            before.append((prev.year, prev.month, prev.day))
    return sorted(set(half_years) | set(STEP_DATES) | set(before))


def _tai_minus_utc_ours(y: int, m: int, d: int) -> float:
    utc = tp.JulianDate.from_calendar(y, m, d, 0.0)
    return tp.utc_to_tai(utc).seconds_since(utc)


def _astropy_steps():
    ap = LeapSeconds.from_iers_leap_seconds()
    return [(int(r["year"]), int(r["month"]), int(r["day"]), int(r["tai_utc"])) for r in ap]


def test_table_metadata_matches_erfa_history():
    info = tp.leap_second_table_info()
    # ERFA's built-in table has 28 steps from 1972 (10 s) to 2017 (37 s).
    assert info["entries"] == 28
    assert info["last_tai_minus_utc"] == 37.0
    assert erfa.dat(2017, 1, 1, 0.0) == info["last_tai_minus_utc"]


def test_astropy_table_has_erfa_steps():
    steps = _astropy_steps()
    assert [s[:3] for s in steps] == STEP_DATES
    assert [s[3] for s in steps] == list(range(10, 38))


@pytest.mark.parametrize("y,m,d", _check_dates())
def test_tai_minus_utc_matches_erfa_dat(y, m, d):
    ours = _tai_minus_utc_ours(y, m, d)
    ref = erfa.dat(y, m, d, 0.0)
    assert round(ours) == ref, f"{y}-{m:02d}-{d:02d}: ours {ours!r}, erfa.dat {ref}"
    assert abs(ours - ref) < RESIDUAL_TOL_S, f"{y}-{m:02d}-{d:02d}: residual {ours - ref!r} s"


def test_tai_minus_utc_matches_astropy_on_all_points():
    steps = _astropy_steps()
    for y, m, d in _check_dates():
        applicable = [s[3] for s in steps if (s[0], s[1], s[2]) <= (y, m, d)]
        ref = applicable[-1]
        ours = _tai_minus_utc_ours(y, m, d)
        assert round(ours) == ref and abs(ours - ref) < RESIDUAL_TOL_S, f"{y}-{m:02d}-{d:02d}"


def test_instant_before_table_is_rejected():
    # The embedded table starts at 1972-01-01; ERFA gives 10 s only from there.
    with pytest.raises(ValueError, match="precedes the leap-second table"):
        tp.utc_to_tai(tp.JulianDate.from_calendar(1971, 12, 31, 0.0))
