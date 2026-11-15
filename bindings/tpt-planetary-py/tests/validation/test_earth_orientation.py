"""Polar-motion matrix and low-precision Earth-Sun distance.

References:
- ``erfa.pom00`` (pyerfa 2.0.1.5): polar-motion matrix, IAU 2000. Its
  documented sense is V(TRS) = rpom * V(CIP), so ``rpom`` maps TIRS to ITRS
  and the ITRS -> TIRS matrix W of the crate is ``rpom.T``.
- ``erfa.epv00`` (pyerfa 2.0.1.5): heliocentric Earth position, valid
  1900-2100. The distance is the norm of the returned heliocentric position.

Points: 200 seeded random (xp, yp) in [-1, 1] arcsec plus fixed points with
both signs; the TIO locator s' is 0 on both sides. Earth-Sun distance at 200
TT epochs spread evenly over 1900-01-01 to 2100-01-01.

The crate's matrix comes from ``validation_dump`` (modes ``pm`` and ``esd``).

Tolerances (set before measuring):
- Polar motion: 1e-12 (dimensionless matrix elements). This is about 0.2
  microarcsecond of rotation. Both sides evaluate the same closed form, so the
  expected difference is double-precision rounding.
- Polar motion sign: the untransposed comparison must differ by more than
  1e-6. A sign or axis-order error in the crate would give a difference of
  order 1e-6 for 0.3 arcsec, so this guards the convention.
- Earth-Sun distance: 1e-4 AU, the accuracy stated in the crate's doc comment
  for ``earth_sun_distance_au_approx`` (Meeus, low-precision series).
"""

from __future__ import annotations

import erfa
import numpy as np

from _reference import run_validation_dump

AU_KM = 149_597_870.7
PM_TOL = 1e-12
PM_SIGN_MIN_DIFF = 1e-6
ESD_TOL_AU = 1e-4

RNG = np.random.default_rng(20260102)
ARCSEC_TO_RAD = np.pi / (180.0 * 3600.0)


def _pm_points():
    rand = RNG.uniform(-1.0, 1.0, size=(200, 2))
    fixed = np.array([[0.2, 0.3], [-0.2, 0.3], [0.2, -0.3], [-0.2, -0.3], [0.0, 0.0]])
    return np.r_[fixed, rand]


def _ours_pm(points):
    return run_validation_dump("pm", points).reshape(-1, 3, 3)


def _ref_pm(points):
    return np.array(
        [erfa.pom00(x * ARCSEC_TO_RAD, y * ARCSEC_TO_RAD, 0.0) for x, y in points]
    )


def test_polar_motion_matches_erfa_pom00_transpose():
    pts = _pm_points()
    ours = _ours_pm(pts)
    ref_t = np.transpose(_ref_pm(pts), (0, 2, 1))
    max_diff = float(np.abs(ours - ref_t).max())
    assert max_diff <= PM_TOL, f"max |W - pom00^T| = {max_diff:.3e}"


def test_polar_motion_untransposed_differs_from_pom00():
    # Guards the convention: W is the transpose of pom00, not pom00 itself.
    pts = _pm_points()
    ours = _ours_pm(pts)
    ref = _ref_pm(pts)
    max_diff = float(np.abs(ours - ref).max())
    assert max_diff > PM_SIGN_MIN_DIFF, f"max |W - pom00| = {max_diff:.3e}"


def _esd_epochs():
    # TT epochs, 1900-01-01 to 2100-01-01, split as (2451545.0, days since J2000).
    days = np.linspace(2415020.5 - 2451545.0, 2488069.5 - 2451545.0, 200)
    return np.c_[np.full(days.shape, 2451545.0), days]


def test_earth_sun_distance_approx_matches_erfa_epv00():
    epochs = _esd_epochs()
    ours = run_validation_dump("esd", epochs)[:, 0]
    ref = np.array(
        [np.linalg.norm(erfa.epv00(h, l)[0][0]) for h, l in epochs]
    )
    max_diff = float(np.abs(ours - ref).max())
    assert max_diff <= ESD_TOL_AU, (
        f"max |d - epv00| = {max_diff:.3e} AU ({max_diff * AU_KM:.1f} km)"
    )
