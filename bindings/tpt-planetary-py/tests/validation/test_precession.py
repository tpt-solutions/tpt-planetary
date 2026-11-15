"""IAU 2006 precession angles and matrix against ERFA.

References (pyerfa 2.0.1.5):
- ``erfa.p06e``: IAU 2006 precession angles. Its returned tuple is
  (eps0, psia, oma, bpa, bqa, pia, bpia, epsa, chia, za, zetaa, thetaa, ...),
  so the pure precession angles are indices 9 (za), 10 (zetaa) and 11
  (thetaa), in radians. These are bias-free angles.
- ``erfa.bp06``: returns (rb, rp, rbp). ``rp`` is the bias-free precession
  matrix; ``rbp`` includes the frame bias and is not used here.

``erfa.pfw06`` gives the Fukushima-Williams angles, which include the frame
bias, so it is not compared directly. Its bias-free content is the same
precession model that ``p06e`` and ``bp06`` expose.

Epochs: 200 TT epochs spread evenly over 1900-01-01 to 2100-01-01, split as
(2451545.0, days since J2000), the same grid as the Earth-Sun test.

The crate's values come from ``validation_dump`` mode ``prec``, which prints
``zeta_A z_A theta_A`` (arcseconds) and the nine elements of P(t) (row-major).
The Rust side takes t in Julian centuries, so t = days / 36525 here.

Tolerances (set before measuring):
- Angles: 1e-6 arcsec (1 microarcsecond). Both sides evaluate the same
  published polynomial, so the expected difference is double-precision rounding.
- Matrix against the p06e-angle construction: 1e-12 per element. Against
  bp06.rp: 5e-12 (see the test docstring; widened after measurement).
- Convention guard: the transposed matrix must differ from ``rp`` by more than
  1e-3 at the 1900 and 2100 ends, so a transpose error would not pass.
"""

from __future__ import annotations

import erfa
import numpy as np

from _reference import run_validation_dump

ARCSEC_PER_RAD = 180.0 * 3600.0 / np.pi
J2000_JD = 2451545.0
DAYS_PER_CENTURY = 36525.0

ANGLE_TOL_ARCSEC = 1e-6
MATRIX_TOL = 1e-12
BP06_MATRIX_TOL = 5e-12
TRANSPOSE_MIN_DIFF = 1e-3


def _epochs():
    # TT epochs, 1900-01-01 to 2100-01-01 (JD 2415020.5 to 2488069.5).
    days = np.linspace(2415020.5 - J2000_JD, 2488069.5 - J2000_JD, 200)
    return days


def _ours(days):
    t = (days / DAYS_PER_CENTURY).reshape(-1, 1)
    out = run_validation_dump("prec", t)
    angles = out[:, :3]
    matrices = out[:, 3:].reshape(-1, 3, 3)
    return angles, matrices


def _ref_angles_arcsec(days):
    rows = []
    for d in days:
        p = erfa.p06e(J2000_JD, d)
        za, zeta, theta = p[9], p[10], p[11]
        rows.append([zeta * ARCSEC_PER_RAD, za * ARCSEC_PER_RAD, theta * ARCSEC_PER_RAD])
    return np.array(rows)


def _ref_matrix(days):
    return np.array([erfa.bp06(J2000_JD, d)[1] for d in days])


def test_angles_match_erfa_p06e():
    days = _epochs()
    ours, _ = _ours(days)
    ref = _ref_angles_arcsec(days)
    max_diff = float(np.abs(ours - ref).max())
    assert max_diff <= ANGLE_TOL_ARCSEC, f"max angle diff = {max_diff:.3e} arcsec"


def _ref_matrix_from_p06e_angles(days):
    # R3(-z) R2(theta) R3(-zeta) built from ERFA's own angles, same convention as the crate.
    def rz(a):
        s, c = np.sin(a), np.cos(a)
        return np.array([[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]])

    def ry(a):
        s, c = np.sin(a), np.cos(a)
        return np.array([[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]])

    rows = []
    for d in days:
        p = erfa.p06e(J2000_JD, d)
        za, zeta, theta = p[9], p[10], p[11]
        rows.append(rz(-za) @ ry(theta) @ rz(-zeta))
    return np.array(rows)


def test_matrix_matches_erfa_p06e_angles():
    # Construction check: the crate's matrix equals the same rotation chain built
    # from ERFA's angles. This is the tight test of the matrix formula.
    days = _epochs()
    _, ours = _ours(days)
    ref = _ref_matrix_from_p06e_angles(days)
    max_diff = float(np.abs(ours - ref).max())
    assert max_diff <= MATRIX_TOL, f"max |P - R(p06e angles)| = {max_diff:.3e}"


def test_matrix_matches_erfa_bp06_rp():
    # bp06 builds rp from Fukushima-Williams angles, a different parameterisation
    # of the same P03 model. Its residual against the zeta/z/theta form is about
    # 2.4e-12 at the 2100 end (0.5 microarcsecond). Tolerance widened from the
    # 1e-12 set for the angle-based test after measurement; see docs/validation.md.
    days = _epochs()
    _, ours = _ours(days)
    ref = _ref_matrix(days)
    max_diff = float(np.abs(ours - ref).max())
    assert max_diff <= BP06_MATRIX_TOL, f"max |P - bp06.rp| = {max_diff:.3e}"


def test_matrix_convention_is_not_transposed():
    # Guards the convention: P maps J2000 to date, so its transpose must differ.
    days = np.array([2415020.5 - J2000_JD, 2488069.5 - J2000_JD])
    _, ours = _ours(days)
    ref = _ref_matrix(days)
    max_diff = float(np.abs(np.transpose(ours, (0, 2, 1)) - ref).max())
    assert max_diff > TRANSPOSE_MIN_DIFF, f"max |P^T - rp| = {max_diff:.3e}"
