"""The ``icrs_to_altaz`` binding against the ``validation_dump`` example.

The accuracy of the Alt/Az chain and of refraction is checked against ERFA and
Astropy in ``test_altaz.py`` and ``test_altaz_refraction.py``. This test checks
that the Python function passes its inputs to the same Rust code: for the same
directions, epoch, observer and atmosphere, the binding and ``validation_dump``
agree to 1e-9 degrees.
"""

from __future__ import annotations

import numpy as np
import pytest

import tpt_planetary as tp
from _reference import run_validation_dump

OBSERVER = (-70.6, -33.5, 2400.0)  # lon deg, lat deg, height m
TT = (2_451_545.0, 0.0008)
UT1 = (2_451_545.0, -0.0002)
POLAR = (0.1, 0.25)  # xp, yp arcsec


def _directions(n: int = 50):
    rng = np.random.default_rng(7)
    ra = rng.uniform(0.0, 360.0, n)
    dec = np.degrees(np.arcsin(rng.uniform(-1.0, 1.0, n)))
    return ra, dec


def _binding(ra, dec, pressure, temperature=15.0, humidity=0.5, wavelength=0.55):
    return tp.icrs_to_altaz(
        ra,
        dec,
        OBSERVER[0],
        OBSERVER[1],
        OBSERVER[2],
        tp.JulianDate(*TT),
        tp.JulianDate(*UT1),
        POLAR[0],
        POLAR[1],
        pressure,
        temperature,
        humidity,
        wavelength,
    )


def test_geometric_matches_validation_dump():
    ra, dec = _directions()
    az, alt = _binding(ra, dec, 0.0)
    rows = np.column_stack(
        [
            ra,
            dec,
            np.full_like(ra, OBSERVER[0]),
            np.full_like(ra, OBSERVER[1]),
            np.full_like(ra, TT[0]),
            np.full_like(ra, TT[1]),
            np.full_like(ra, UT1[0]),
            np.full_like(ra, UT1[1]),
            np.full_like(ra, POLAR[0]),
            np.full_like(ra, POLAR[1]),
        ]
    )
    ref = run_validation_dump("altaz", rows)
    assert np.abs(az - ref[:, 0]).max() < 1e-9
    assert np.abs(alt - ref[:, 1]).max() < 1e-9


@pytest.mark.parametrize("pressure", [1013.25, 700.0])
def test_refracted_matches_validation_dump(pressure):
    ra, dec = _directions()
    az, alt = _binding(ra, dec, pressure, temperature=-5.0, humidity=0.8, wavelength=0.7)
    n = len(ra)
    rows = np.column_stack(
        [
            ra,
            dec,
            np.full(n, OBSERVER[0]),
            np.full(n, OBSERVER[1]),
            np.full(n, TT[0]),
            np.full(n, TT[1]),
            np.full(n, UT1[0]),
            np.full(n, UT1[1]),
            np.full(n, POLAR[0]),
            np.full(n, POLAR[1]),
            np.full(n, pressure),
            np.full(n, -5.0),
            np.full(n, 0.8),
            np.full(n, 0.7),
        ]
    )
    ref = run_validation_dump("altaz_refracted", rows)
    assert np.abs(az - ref[:, 0]).max() < 1e-9
    assert np.abs(alt - ref[:, 1]).max() < 1e-9


def test_refraction_raises_the_altitude():
    ra, dec = _directions()
    _, geometric = _binding(ra, dec, 0.0)
    _, refracted = _binding(ra, dec, 1013.25)
    keep = geometric > 15.0
    shift = (refracted - geometric)[keep]
    assert (shift >= 0.0).all()
    # Shifts are in degrees. Refraction at 15 degrees altitude is about 4.5
    # arcminutes (0.075 degrees), so every shift above 15 degrees is below that.
    assert shift.max() < 0.08


def test_mismatched_arrays_are_an_error():
    with pytest.raises(ValueError, match="same length"):
        _binding(np.zeros(3), np.zeros(2), 0.0)
