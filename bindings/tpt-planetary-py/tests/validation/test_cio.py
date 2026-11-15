"""CIP coordinates X and Y, CIO and TIO locators, frame bias, EO complementary
terms, and the CIO-based ICRS to ITRS matrix, against ERFA.

References (pyerfa 2.0.1.5, used here as a test oracle only):
- ``erfa.xy06``: X, Y of the CIP (IAU 2006/2000A), including frame bias.
- ``erfa.s06``: CIO locator s, given X and Y.
- ``erfa.sp00``: TIO locator s'.
- ``erfa.bp00``: frame bias matrix rb (GCRS to mean J2000 equator and equinox).
- ``erfa.eect00``: complementary terms of the equation of the equinoxes.
- ``erfa.era00``: Earth rotation angle.
- ``erfa.c2t06a``: GCRS to ITRS matrix (CIO-based, with polar motion).

The crate's values come from ``validation_dump`` (no bindings expose these):
``xy``, ``s``, ``sp``, ``bias``, ``eect``, ``era``, ``c2t``.

Epochs: 200 TT epochs spread evenly over 1900-01-01 to 2100-01-01, split as
(2451545.0, days since J2000). UT1 is given as the same two-part date as the
TT epoch (UT1 - TT = 0 by construction), which is the convention used for the
ERA check and for the chain: the test isolates the matrix algebra from any
UT1-UTC model.

Polar motion: zero, and a nonzero case with xp = 0.1 arcsec, yp = 0.2 arcsec.

Matrix convention: ERFA's ``c2t06a`` returns the matrix that maps GCRS vectors
to ITRS vectors. The crate's ``icrs_to_itrs`` returns the same matrix (not its
transpose); the transpose is checked to differ by far more than the tolerance.

Tolerances. Each is set from the measured residual, not fitted to the data:
- X, Y, s, s', EO complementary terms: 1 microarcsecond. Measured at most
  about 1e-6 microarcseconds for X and Y, 3e-9 for s, and below 1e-9 for s'
  and the complementary terms.
- Frame bias: 1e-11 rad. Measured 7e-13.
- ERA: 1e-12 rad. Measured 6e-14 rad (after the rounding fix in
  `time::earth_rotation_angle`; before it, 4e-11 rad at 1900).
- Matrix algebra, given ERFA's own X, Y, s, ERA and s': 1e-12. Measured
  2e-18, which shows the convention, ERA, s' and polar-motion sign are exact.
- End-to-end chain (our X, Y, s from the tables): 2.5e-11 rad, about 5
  microarcseconds. Measured 1.76e-11 rad (3.6 uas), the same size as the
  disagreement between ERFA's own routes to X and Y (see
  `test_erfa_routes_disagree_by_more_than_the_xy_tolerance`). ERFA's
  `c2t06a` takes X, Y and s from `xys06a`, while `xy06` gives the series
  value that matches ours to 1e-6 uas. The end-to-end tolerance is therefore
  set at the level of that reference ambiguity, and the tight algebra check
  above is the test of our matrices.
"""

from __future__ import annotations

import erfa
import numpy as np

from _reference import run_validation_dump

J2000_JD = 2451545.0
RAD_PER_UAS = np.pi / (180.0 * 3600.0) * 1e-6
RAD_PER_ARCSEC = np.pi / (180.0 * 3600.0)

XY_TOL_UAS = 1.0
S_TOL_UAS = 1.0
SP_TOL_UAS = 1.0
EECT_TOL_UAS = 1.0
BIAS_TOL = 1e-11
MATRIX_ALGEBRA_TOL = 1e-12
END_TO_END_TOL = 2.5e-11
ERA_TOL_RAD = 1e-12
TRANSPOSE_MIN_DIFF = 1e-5
POLAR_XP_ARCSEC = 0.1
POLAR_YP_ARCSEC = 0.2


def _epochs():
    """200 TT epochs: days since J2000 for 1900-01-01 .. 2100-01-01."""
    return np.linspace(2415020.5 - J2000_JD, 2488069.5 - J2000_JD, 200)


def _pairs(days):
    return np.stack([np.full_like(days, J2000_JD), days], axis=1)


def _chain_rows(days, xp_arcsec, yp_arcsec):
    tt = _pairs(days)
    xp = np.full(len(days), xp_arcsec * RAD_PER_ARCSEC)
    yp = np.full(len(days), yp_arcsec * RAD_PER_ARCSEC)
    return np.column_stack([tt, tt, xp, yp])


def test_cip_xy_matches_erfa_xy06():
    days = _epochs()
    ours = run_validation_dump("xy", _pairs(days))
    ref = np.array([erfa.xy06(J2000_JD, d) for d in days])
    diff_uas = np.abs(ours - ref) / RAD_PER_UAS
    assert diff_uas[:, 0].max() <= XY_TOL_UAS, f"max |dX| = {diff_uas[:, 0].max():.3e} uas"
    assert diff_uas[:, 1].max() <= XY_TOL_UAS, f"max |dY| = {diff_uas[:, 1].max():.3e} uas"


def test_cio_locator_matches_erfa_s06():
    days = _epochs()
    ours = run_validation_dump("s", _pairs(days))[:, 0]
    ref = np.array([erfa.s06(J2000_JD, d, *erfa.xy06(J2000_JD, d)) for d in days])
    diff_uas = np.abs(ours - ref) / RAD_PER_UAS
    assert diff_uas.max() <= S_TOL_UAS, f"max |ds| = {diff_uas.max():.3e} uas"


def test_tio_locator_matches_erfa_sp00():
    days = _epochs()
    ours = run_validation_dump("sp", _pairs(days))[:, 0]
    ref = np.array([erfa.sp00(J2000_JD, d) for d in days])
    diff_uas = np.abs(ours - ref) / RAD_PER_UAS
    assert diff_uas.max() <= SP_TOL_UAS, f"max |ds'| = {diff_uas.max():.3e} uas"


def test_eo_complementary_terms_match_erfa_eect00():
    days = _epochs()
    ours = run_validation_dump("eect", _pairs(days))[:, 0]
    ref = np.array([erfa.eect00(J2000_JD, d) for d in days])
    diff_uas = np.abs(ours - ref) / RAD_PER_UAS
    assert diff_uas.max() <= EECT_TOL_UAS, f"max |d eect| = {diff_uas.max():.3e} uas"


def test_frame_bias_matrix_matches_erfa_bp00():
    # The bias matrix does not depend on time; one row suffices.
    ours = run_validation_dump("bias", np.array([[0.0]]))[0].reshape(3, 3)
    ref = erfa.bp00(J2000_JD, 0.0)[0]
    assert np.abs(ours - ref).max() <= BIAS_TOL, f"max |dB| = {np.abs(ours - ref).max():.3e}"
    # Guard: the matrix must not be the identity (it carries about 7e-8 rad).
    assert np.abs(ours - np.eye(3)).max() > 1e-8


def test_era_matches_erfa_era00():
    days = _epochs()
    ours = run_validation_dump("era", _pairs(days))[:, 0]
    ref = np.array([erfa.era00(J2000_JD, d) for d in days])
    wrapped = (ours - ref + np.pi) % (2.0 * np.pi) - np.pi
    assert np.abs(wrapped).max() <= ERA_TOL_RAD, f"max |dERA| = {np.abs(wrapped).max():.3e} rad"


def _erfa_components(days, xp_arcsec, yp_arcsec):
    """ERFA's own X, Y, s (xys06a), ERA, s' and polar motion, per epoch."""
    xp = xp_arcsec * RAD_PER_ARCSEC
    yp = yp_arcsec * RAD_PER_ARCSEC
    rows = []
    for d in days:
        x, y, s = erfa.xys06a(J2000_JD, d)
        rows.append([x, y, s, erfa.era00(J2000_JD, d), erfa.sp00(J2000_JD, d), xp, yp])
    return np.array(rows)


def _c2t06a_refs(days, xp_arcsec, yp_arcsec):
    xp = xp_arcsec * RAD_PER_ARCSEC
    yp = yp_arcsec * RAD_PER_ARCSEC
    return np.array([erfa.c2t06a(J2000_JD, d, J2000_JD, d, xp, yp) for d in days])


def test_matrix_algebra_matches_erfa_c2t06a_given_erfa_components():
    # Our chain from ERFA's X, Y, s, ERA and s' must reproduce c2t06a. This
    # isolates the matrix algebra and conventions from the series.
    days = _epochs()
    for xp_arcsec, yp_arcsec in [(0.0, 0.0), (POLAR_XP_ARCSEC, POLAR_YP_ARCSEC)]:
        ours = run_validation_dump(
            "c2t_xys", _erfa_components(days, xp_arcsec, yp_arcsec)
        ).reshape(-1, 3, 3)
        ref = _c2t06a_refs(days, xp_arcsec, yp_arcsec)
        diff = np.abs(ours - ref).max()
        assert diff <= MATRIX_ALGEBRA_TOL, (
            f"max |dM| (xp, yp = {xp_arcsec}, {yp_arcsec} arcsec) = {diff:.3e}"
        )


def test_icrs_to_itrs_matches_erfa_c2t06a_zero_polar_motion():
    days = _epochs()
    ours = run_validation_dump("c2t", _chain_rows(days, 0.0, 0.0)).reshape(-1, 3, 3)
    diff = np.abs(ours - _c2t06a_refs(days, 0.0, 0.0)).max()
    assert diff <= END_TO_END_TOL, f"max |dM| (xp=yp=0, end to end) = {diff:.3e}"


def test_icrs_to_itrs_matches_erfa_c2t06a_with_polar_motion():
    days = _epochs()
    rows = _chain_rows(days, POLAR_XP_ARCSEC, POLAR_YP_ARCSEC)
    ours = run_validation_dump("c2t", rows).reshape(-1, 3, 3)
    ref = _c2t06a_refs(days, POLAR_XP_ARCSEC, POLAR_YP_ARCSEC)
    diff = np.abs(ours - ref).max()
    assert diff <= END_TO_END_TOL, f"max |dM| (xp, yp = 0.1, 0.2 arcsec, end to end) = {diff:.3e}"


def test_matrix_convention_is_erfa_not_transpose():
    # The matrix is ERFA's GCRS -> ITRS matrix, not its transpose.
    days = _epochs()[:1]
    ref = _c2t06a_refs(days, POLAR_XP_ARCSEC, POLAR_YP_ARCSEC)[0]
    ours = run_validation_dump("c2t_xys", _erfa_components(days, POLAR_XP_ARCSEC, POLAR_YP_ARCSEC))[0]
    ours = ours.reshape(3, 3)
    assert np.abs(ours - ref).max() <= MATRIX_ALGEBRA_TOL
    assert np.abs(ours - ref.T).max() > TRANSPOSE_MIN_DIFF


def test_erfa_routes_disagree_by_more_than_the_xy_tolerance():
    # Documents why the end-to-end tolerance is not 1 uas: ERFA's xys06a (used by
    # c2t06a) and xy06 (the series) differ by several microarcseconds.
    days = _epochs()
    xys = np.array([erfa.xys06a(J2000_JD, d)[:2] for d in days])
    xy = np.array([erfa.xy06(J2000_JD, d) for d in days])
    diff_uas = np.abs(xys - xy).max() / RAD_PER_UAS
    assert diff_uas > XY_TOL_UAS, f"ERFA routes differ by only {diff_uas:.3f} uas"
