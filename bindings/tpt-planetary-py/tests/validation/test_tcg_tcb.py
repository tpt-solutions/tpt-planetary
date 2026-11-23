"""TCG and TCB offsets from TT and TDB, against ERFA's tttcg, tcgtt, tdbtcb and tcbtdb.

The crate's value comes from ``validation_dump`` (``time::tt_to_tcg``,
``tcg_to_tt``, ``tdb_to_tcb`` and ``tcb_to_tdb``), which prints the offset in
seconds from the input epoch. The reference is pyerfa 2.0.1.5, the IAU SOFA
implementation of the same relations (IAU 2000 Resolution B1.9 for TCG and IAU
2006 Resolution B3 for TCB, with TDB0 = -6.55e-5 s).

Epochs: 300 random two-part Julian dates from 1800 to 2200, from a fixed seed.

Tolerance: 1e-10 s in each direction. The measured maximum difference from
ERFA is about 1e-11 s, which is the float rounding of two-part dates.
"""

from __future__ import annotations

import erfa
import numpy as np
import pytest

from _reference import run_validation_dump

TOLERANCE_SECONDS = 1e-10
N = 300
SEED = 20261123


def _epochs():
    rng = np.random.default_rng(SEED)
    jd = rng.uniform(2378496.5, 2561208.5, N)  # 1800 .. 2200
    hi = np.floor(jd)
    return np.column_stack([hi, jd - hi])


def _seconds(a, b):
    """Offset of two two-part dates, b - a, in seconds."""
    return ((b[:, 0] - a[:, 0]) + (b[:, 1] - a[:, 1])) * 86400.0


def _erfa_pair(func, rows):
    out1, out2 = func(rows[:, 0], rows[:, 1])
    return np.column_stack([out1, out2])


@pytest.mark.parametrize("mode", ["tt_tcg", "tcg_tt", "tdb_tcb", "tcb_tdb"])
def test_offsets_match_erfa(mode):
    rows = _epochs()
    if mode == "tt_tcg":
        ref = _erfa_pair(erfa.tttcg, rows)
    elif mode == "tcg_tt":
        # Input: TCG; output: TT - TCG. ERFA's tcgtt gives TT from TCG.
        rows = _erfa_pair(erfa.tttcg, _epochs())
        ref = _erfa_pair(erfa.tcgtt, rows)
    elif mode == "tdb_tcb":
        ref = _erfa_pair(erfa.tdbtcb, rows)
    else:
        rows = _erfa_pair(erfa.tdbtcb, _epochs())
        ref = _erfa_pair(erfa.tcbtdb, rows)
    ours = run_validation_dump(mode, rows)[:, 0]
    expected = _seconds(rows, ref)
    assert np.abs(ours - expected).max() < TOLERANCE_SECONDS
