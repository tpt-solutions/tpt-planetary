"""IAU 2000A_R06 nutation, equation of the equinoxes and nutation matrix against ERFA.

References (pyerfa 2.0.1.5, used here as a test oracle only):
- ``erfa.nut06a``: nutation (dpsi, deps), IAU 2006/2000A.
- ``erfa.ee06a``: equation of the equinoxes.
- ``erfa.num06a``: nutation matrix N (mean of date -> true of date).
- ``erfa.obl06``: IAU 2006 mean obliquity eps0.
- ``erfa.gst06a``: GAST.
- ``erfa.fal03`` ... ``erfa.fapa03``: the fourteen fundamental arguments.
- ``erfa.bp06``: rp, the bias-free precession matrix (used to build N P).

The crate's values come from ``validation_dump`` (no bindings expose these):
``fa``, ``nut``, ``nut_sofa``, ``ee``, ``numat``, ``numat_from``, ``gast``, ``tod``.

Two model variants are compared, and the difference between them is the point
of the nutation tests:

- The IERS Conventions (2010) published model, Tables 5.3a/5.3b, which includes
  the time-varying out-of-phase amplitudes (A"'_i in longitude, B"'_i in
  obliquity). This is the crate's ``nutation_angles``.
- The MHB2000/SOFA series, which omits those out-of-phase rate terms. ERFA's
  ``nut06a`` evaluates this series. The crate exposes it as
  ``nutation_angles_t_without_out_of_phase_rates`` (``nut_sofa`` mode).

IERS Conventions (2010), Section 5.7.2, notes the omission. Its size is about
5 microarcseconds in dpsi at the 1900 and 2100 ends (the test below measures it).

Epochs: 300 TT epochs spread evenly over 1900-01-01 to 2100-01-01, split as
(2451545.0, days since J2000), the same grid as the precession tests.

Tolerances (set after measuring the residual decomposition, which is recorded
in ``docs/validation.md``):
- Fundamental arguments: 1e-11 rad. Measured max 1.6e-12 rad.
- SOFA-variant angles against ``nut06a``: 0.5 microarcsecond. Measured max
  0.25 uas in dpsi and 0.22 uas in deps. The residual is not explained at source.
- Published angles against ``nut06a``: the difference must equal the
  out-of-phase contribution to within 0.5 uas, and that contribution must be
  real (at least 4 uas), so this is not a loose bound.
- Equation of the equinoxes against ``ee06a``: 2.7 mas. The gap is the
  complementary terms, which the crate does not implement (measured max 2.65 mas).
- Matrix convention: the matrix built from ERFA's own angles must match
  ``num06a`` to 1e-12. Measured 0. The transpose must differ by more than 1e-5
  (measured 1.7e-4), which guards the convention.
- Matrix from the published angles against ``num06a``: 2.5e-11 (about 5 uas).
  Measured max 2.35e-11, the same out-of-phase effect as above.
- GAST against ``gst06a``: 2.7 mas, the complementary-term omission (measured
  max 2.65 mas, rms 1.86 mas).
- ICRS -> true equator of date against ``rn @ rp``: 6 uas, the published-model
  residual (measured max 3.1 uas).
"""

from __future__ import annotations

import erfa
import numpy as np

from _reference import run_validation_dump

J2000_JD = 2451545.0
DAYS_PER_CENTURY = 36525.0
RAD_PER_UAS = np.pi / (180.0 * 3600.0) * 1e-6
RAD_PER_MAS = RAD_PER_UAS * 1e3

FA_TOL_RAD = 1e-11
SOFA_ANGLE_TOL_UAS = 0.5
OUT_OF_PHASE_MIN_UAS = 4.0
EE_TOL_MAS = 2.7
EE_CONSISTENCY_TOL_RAD = 1e-12
MATRIX_CONVENTION_TOL = 1e-12
MATRIX_PUBLISHED_TOL = 2.5e-11
# N is close to the identity (entries differ from it by about 1e-4), so its transpose
# differs from N by about 1.7e-4. The guard only needs to be far above the 1e-12 tolerance.
TRANSPOSE_MIN_DIFF = 1e-5
GAST_TOL_MAS = 2.7
TOD_TOL_UAS = 6.0

FA_NAMES = [
    "fal03",
    "falp03",
    "faf03",
    "fad03",
    "faom03",
    "fame03",
    "fave03",
    "fae03",
    "fama03",
    "faju03",
    "fasa03",
    "faur03",
    "fane03",
    "fapa03",
]


def _epochs():
    """TT epochs as (hi, lo) pairs: (JD(TT) of J2000, days since J2000)."""
    days = np.linspace(2415020.5 - J2000_JD, 2488069.5 - J2000_JD, 300)
    return days


def _pairs(days):
    return np.stack([np.full_like(days, J2000_JD), days], axis=1)


def _wrap(x):
    """Wrap an angle difference to [-pi, pi)."""
    return (x + np.pi) % (2.0 * np.pi) - np.pi


def _nut_ours(days):
    return run_validation_dump("nut", _pairs(days))


def _nut_sofa(days):
    return run_validation_dump("nut_sofa", (days / DAYS_PER_CENTURY).reshape(-1, 1))


def _nut_ref(days):
    return np.array([erfa.nut06a(J2000_JD, d) for d in days])


def test_fundamental_arguments_match_erfa():
    days = _epochs()
    ours = run_validation_dump("fa", (days / DAYS_PER_CENTURY).reshape(-1, 1))
    ref = np.array(
        [[getattr(erfa, name)(d / DAYS_PER_CENTURY) for name in FA_NAMES] for d in days]
    )
    diff = np.abs(_wrap(ours - ref))
    assert diff.max() <= FA_TOL_RAD, f"max fundamental-argument diff = {diff.max():.3e} rad"


def test_sofa_variant_angles_match_erfa_nut06a():
    # The MHB2000/SOFA series, which pyerfa's nut06a evaluates, without the
    # out-of-phase rate terms. Measured residual 0.25 uas (dpsi) and 0.22 uas (deps).
    days = _epochs()
    ours = _nut_sofa(days)
    ref = _nut_ref(days)
    diff_uas = np.abs(ours - ref) / RAD_PER_UAS
    assert diff_uas[:, 0].max() <= SOFA_ANGLE_TOL_UAS, (
        f"max |dpsi| diff = {diff_uas[:, 0].max():.3f} uas"
    )
    assert diff_uas[:, 1].max() <= SOFA_ANGLE_TOL_UAS, (
        f"max |deps| diff = {diff_uas[:, 1].max():.3f} uas"
    )


def test_published_angles_differ_from_erfa_only_by_out_of_phase_terms():
    # Published minus ERFA must equal the omitted out-of-phase term (published
    # minus SOFA-variant) to within the SOFA-variant residual, and that term must
    # be real, so the discrepancy is not hidden by a loose bound.
    days = _epochs()
    published = _nut_ours(days)
    sofa = _nut_sofa(days)
    ref = _nut_ref(days)
    out_of_phase_uas = np.abs(published - sofa) / RAD_PER_UAS
    gap_after_removal_uas = np.abs((published - ref) - (published - sofa)) / RAD_PER_UAS
    assert out_of_phase_uas[:, 0].max() >= OUT_OF_PHASE_MIN_UAS, (
        f"out-of-phase dpsi contribution is smaller than expected: "
        f"{out_of_phase_uas[:, 0].max():.3f} uas"
    )
    assert gap_after_removal_uas.max() <= SOFA_ANGLE_TOL_UAS, (
        f"residual after removing out-of-phase terms = {gap_after_removal_uas.max():.3f} uas"
    )


def test_equation_of_equinoxes_matches_ee06a_without_complementary_terms():
    days = _epochs()
    ours = run_validation_dump("ee", _pairs(days)).ravel()
    ref = np.array([erfa.ee06a(J2000_JD, d) for d in days])
    diff_mas = np.abs(ours - ref) / RAD_PER_MAS
    # The gap is the complementary terms of the equation of the origins. Measured
    # max 2.65 mas. See docs/validation.md.
    assert diff_mas.max() <= EE_TOL_MAS, f"max |EE - ee06a| = {diff_mas.max():.3f} mas"


def test_equation_of_equinoxes_is_dpsi_cos_eps0():
    # The crate's EE is dpsi * cos(eps0) exactly, with eps0 the IAU 2006 mean obliquity.
    days = _epochs()
    ee = run_validation_dump("ee", _pairs(days)).ravel()
    nut = _nut_ours(days)
    eps0 = np.array([erfa.obl06(J2000_JD, d) for d in days])
    expected = nut[:, 0] * np.cos(eps0)
    assert np.abs(ee - expected).max() <= EE_CONSISTENCY_TOL_RAD


def test_matrix_convention_matches_num06a():
    # Build N from ERFA's own angles and compare to num06a. This checks the
    # rotation convention independently of the nutation series.
    days = _epochs()
    ref_nut = _nut_ref(days)
    eps0 = np.array([erfa.obl06(J2000_JD, d) for d in days])
    rows = np.column_stack([ref_nut[:, 0], ref_nut[:, 1], eps0])
    ours = run_validation_dump("numat_from", rows).reshape(-1, 3, 3)
    ref = np.array([erfa.num06a(J2000_JD, d) for d in days])
    max_diff = float(np.abs(ours - ref).max())
    assert max_diff <= MATRIX_CONVENTION_TOL, f"max |N - num06a| = {max_diff:.3e}"
    # The transpose must not match, so a transposed convention would fail.
    transposed = float(np.abs(np.transpose(ours, (0, 2, 1)) - ref).max())
    assert transposed > TRANSPOSE_MIN_DIFF, f"max |N^T - num06a| = {transposed:.3e}"


def test_matrix_from_published_angles_matches_num06a_within_out_of_phase_effect():
    days = _epochs()
    ours = run_validation_dump("numat", _pairs(days)).reshape(-1, 3, 3)
    ref = np.array([erfa.num06a(J2000_JD, d) for d in days])
    max_diff = float(np.abs(ours - ref).max())
    assert max_diff <= MATRIX_PUBLISHED_TOL, f"max |N - num06a| = {max_diff:.3e}"


def test_gast_matches_gst06a_without_complementary_terms():
    days = _epochs()
    pairs = np.stack([np.full_like(days, J2000_JD), days, np.full_like(days, J2000_JD), days], axis=1)
    ours = run_validation_dump("gast", pairs).ravel()
    ref = np.array([erfa.gst06a(J2000_JD, d, J2000_JD, d) for d in days])
    diff_mas = np.abs(_wrap(ours - ref)) / RAD_PER_MAS
    assert diff_mas.max() <= GAST_TOL_MAS, f"max |GAST - gst06a| = {diff_mas.max():.3f} mas"


def test_icrs_to_true_equator_of_date_matches_erfa():
    # Compare with rn @ rp (N times the bias-free precession matrix), which is
    # the ERFA rotation without the frame bias.
    rng = np.random.default_rng(7)
    days = _epochs()[::25]
    rows = []
    for d in days:
        for _ in range(4):
            rows.append([rng.uniform(0.0, 360.0), rng.uniform(-89.0, 89.0), J2000_JD, d])
    rows = np.array(rows)
    ours = run_validation_dump("tod", rows)
    worst = 0.0
    for (ra, dec, hi, lo), (lon_deg, lat_deg) in zip(rows, ours):
        ra_r, dec_r = np.radians(ra), np.radians(dec)
        v = np.array([np.cos(dec_r) * np.cos(ra_r), np.cos(dec_r) * np.sin(ra_r), np.sin(dec_r)])
        m = erfa.num06a(hi, lo) @ erfa.bp06(hi, lo)[1]
        w = m @ v
        lon = np.arctan2(w[1], w[0])
        lat = np.arcsin(np.clip(w[2], -1.0, 1.0))
        worst = max(
            worst,
            abs(_wrap(np.radians(lon_deg) - lon)) * np.cos(lat),
            abs(np.radians(lat_deg) - lat),
        )
    assert worst / RAD_PER_UAS <= TOD_TOL_UAS, f"max diff = {worst / RAD_PER_UAS:.3f} uas"
