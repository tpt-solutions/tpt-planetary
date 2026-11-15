"""Tests for ICRS/Galactic transforms, ERA/GMST and WCS (TAN, SIN, AIT)."""

import math

import numpy as np
import pytest

import tpt_planetary as tp

# Galactic centre (Sgr A*) and Galactic north pole, ICRS, degrees (IAU 1958/ICRS).
GC_RA, GC_DEC = 266.40498829, -28.93617776
NGP_RA, NGP_DEC = 192.85948, 27.12825

# ICRS -> Galactic rotation matrix (Hipparcos / Murray 1989), independent of the crate.
T_ICRS_TO_GAL = np.array(
    [
        [-0.0548755604162154, -0.8734370902348850, -0.4838350155487132],
        [0.4941094278755837, -0.4448296299600112, 0.7469822444972189],
        [-0.8676661490190047, -0.1980763734312015, 0.4559837761750669],
    ]
)


def _unit(ra_deg, dec_deg):
    ra, dec = np.deg2rad(ra_deg), np.deg2rad(dec_deg)
    return np.stack([np.cos(dec) * np.cos(ra), np.cos(dec) * np.sin(ra), np.sin(dec)], axis=-1)


def _angdiff_deg(a, b):
    return (a - b + 180.0) % 360.0 - 180.0


def test_galactic_centre_maps_to_l0_b0():
    l, b = tp.icrs_to_galactic(np.array([GC_RA]), np.array([GC_DEC]))
    assert abs(_angdiff_deg(l[0], 0.0)) < 1e-5
    assert abs(b[0]) < 1e-5


def test_galactic_north_pole_maps_to_b90():
    _, b = tp.icrs_to_galactic(np.array([NGP_RA]), np.array([NGP_DEC]))
    assert b[0] == pytest.approx(90.0, abs=1e-5)


def test_icrs_to_galactic_matches_independent_matrix():
    rng = np.random.default_rng(7)
    ra = rng.uniform(0, 360, 500)
    dec = rng.uniform(-89.9, 89.9, 500)
    l, b = tp.icrs_to_galactic(ra, dec)
    v = _unit(ra, dec) @ T_ICRS_TO_GAL.T
    exp_b = np.rad2deg(np.arcsin(np.clip(v[:, 2], -1, 1)))
    exp_l = np.rad2deg(np.arctan2(v[:, 1], v[:, 0])) % 360.0
    assert np.max(np.abs(_angdiff_deg(l, exp_l))) < 1e-8
    assert np.max(np.abs(b - exp_b)) < 1e-8


def test_icrs_galactic_round_trip():
    rng = np.random.default_rng(3)
    ra = rng.uniform(0, 360, 200)
    dec = rng.uniform(-80, 80, 200)
    l, b = tp.icrs_to_galactic(ra, dec)
    ra2, dec2 = tp.galactic_to_icrs(l, b)
    assert np.max(np.abs(_angdiff_deg(ra2, ra))) < 1e-9
    assert np.max(np.abs(dec2 - dec)) < 1e-9


def test_frame_transform_returns_float64_arrays_of_same_length():
    l, b = tp.icrs_to_galactic(np.zeros(4), np.zeros(4))
    assert l.dtype == np.float64 and b.dtype == np.float64
    assert l.shape == (4,) and b.shape == (4,)


def test_frame_transform_rejects_length_mismatch():
    with pytest.raises(ValueError):
        tp.icrs_to_galactic(np.zeros(3), np.zeros(2))


def test_era_at_j2000_epoch():
    # ERA at UT1 2000-01-01 12:00 is 280.46061837 degrees (IAU 2000).
    era = tp.earth_rotation_angle(tp.JulianDate.from_calendar(2000, 1, 1, 43_200.0))
    expected = math.radians(280.46061837)
    assert math.remainder(era - expected, 2 * math.pi) == pytest.approx(0.0, abs=1e-8)


def test_gmst_close_to_era_at_j2000():
    ut1 = tp.JulianDate.from_calendar(2000, 1, 1, 43_200.0)
    tt = tp.JulianDate.from_calendar(2000, 1, 1, 43_200.0 + 64.184)
    gmst = tp.gmst_iau2006(ut1, tt)
    era = tp.earth_rotation_angle(ut1)
    # GMST and ERA differ only by a small precession term at J2000.
    assert math.remainder(gmst - era, 2 * math.pi) == pytest.approx(0.0, abs=1e-6)


# --- WCS -------------------------------------------------------------------

def _tan_header(crpix=(100.5, 80.5), crval=(180.0, 60.0), cdelt=(-0.001, 0.001), ctype=("RA---TAN", "DEC--TAN")):
    return {
        "CRPIX1": crpix[0],
        "CRPIX2": crpix[1],
        "CRVAL1": crval[0],
        "CRVAL2": crval[1],
        "CDELT1": cdelt[0],
        "CDELT2": cdelt[1],
        "CTYPE1": ctype[0],
        "CTYPE2": ctype[1],
    }


def _gnomonic_inverse(xi_deg, eta_deg, a0_deg, d0_deg):
    """Standard TAN inverse (Calabretta & Greisen 2002, eq. 54-56), in numpy."""
    xi, eta = np.deg2rad(xi_deg), np.deg2rad(eta_deg)
    a0, d0 = np.deg2rad(a0_deg), np.deg2rad(d0_deg)
    ra = a0 + np.arctan2(xi, np.cos(d0) - eta * np.sin(d0))
    dec = np.arctan2(eta * np.cos(d0) + np.sin(d0), np.hypot(xi, np.cos(d0) - eta * np.sin(d0)))
    return np.rad2deg(ra) % 360.0, np.rad2deg(dec)


def test_tan_reference_pixel_maps_to_crval():
    w = tp.Wcs.from_header(_tan_header())
    assert w.crval[0] == pytest.approx(180.0, abs=1e-9)
    assert w.crval[1] == pytest.approx(60.0, abs=1e-9)
    lon, lat = w.pixel_to_world(np.array([100.5]), np.array([80.5]))
    assert lon[0] == pytest.approx(180.0, abs=1e-9)
    assert lat[0] == pytest.approx(60.0, abs=1e-9)


def test_tan_matches_independent_gnomonic_formula():
    hdr = _tan_header()
    w = tp.Wcs.from_header(hdr)
    x = np.linspace(0.5, 200.5, 41)
    y = np.linspace(0.5, 160.5, 41)
    xx, yy = np.meshgrid(x, y)
    xx, yy = xx.ravel(), yy.ravel()
    lon, lat = w.pixel_to_world(xx, yy)
    xi = hdr["CDELT1"] * (xx - hdr["CRPIX1"])
    eta = hdr["CDELT2"] * (yy - hdr["CRPIX2"])
    exp_lon, exp_lat = _gnomonic_inverse(xi, eta, 180.0, 60.0)
    assert np.max(np.abs(_angdiff_deg(lon, exp_lon))) < 1e-7
    assert np.max(np.abs(lat - exp_lat)) < 1e-7


@pytest.mark.parametrize("code", ["TAN", "SIN", "AIT"])
def test_round_trip_pixel_world_pixel(code):
    crval = (45.0, 20.0) if code != "AIT" else (120.0, 10.0)
    hdr = {
        "CRPIX1": 50.0,
        "CRPIX2": 50.0,
        "CRVAL1": crval[0],
        "CRVAL2": crval[1],
        "CDELT1": -0.5,
        "CDELT2": 0.5,
        "CTYPE1": f"RA---{code}",
        "CTYPE2": f"DEC--{code}",
    }
    w = tp.Wcs.from_header(hdr)
    rng = np.random.default_rng(11)
    x = rng.uniform(20, 80, 300)
    y = rng.uniform(20, 80, 300)
    lon, lat = w.pixel_to_world(x, y)
    assert np.all(np.isfinite(lon)) and np.all(np.isfinite(lat))
    x2, y2 = w.world_to_pixel(lon, lat)
    np.testing.assert_allclose(x2, x, atol=1e-7)
    np.testing.assert_allclose(y2, y, atol=1e-7)


def test_sin_pixel_outside_domain_is_nan():
    hdr = {
        "CRPIX1": 1.0,
        "CRPIX2": 1.0,
        "CRVAL1": 0.0,
        "CRVAL2": 0.0,
        "CDELT1": 1.0,
        "CDELT2": 1.0,
        "CTYPE1": "RA---SIN",
        "CTYPE2": "DEC--SIN",
    }
    w = tp.Wcs.from_header(hdr)
    lon, lat = w.pixel_to_world(np.array([1.0, 500.0]), np.array([1.0, 500.0]))
    # The reference point is at lon 0; the crate may return 360 - epsilon or 360.0
    # for a tiny negative longitude, so accept either wrap.
    assert min(lon[0], 360.0 - lon[0]) == pytest.approx(0.0, abs=1e-9)
    assert lat[0] == pytest.approx(0.0, abs=1e-9)
    assert np.isnan(lon[1]) and np.isnan(lat[1])


def test_ait_centre_round_trip():
    hdr = {
        "CRPIX1": 10.0,
        "CRPIX2": 10.0,
        "CRVAL1": 90.0,
        "CRVAL2": 0.0,
        "CDELT1": -1.0,
        "CDELT2": 1.0,
        "CTYPE1": "RA---AIT",
        "CTYPE2": "DEC--AIT",
    }
    w = tp.Wcs.from_header(hdr)
    lon, lat = w.pixel_to_world(np.array([10.0]), np.array([10.0]))
    assert lon[0] == pytest.approx(90.0, abs=1e-9)
    assert lat[0] == pytest.approx(0.0, abs=1e-9)


def test_wcs_header_errors_are_value_errors():
    base = _tan_header()
    no_ctype = {k: v for k, v in base.items() if not k.startswith("CTYPE")}
    with pytest.raises(ValueError):
        tp.Wcs.from_header(no_ctype)
    with pytest.raises(ValueError):
        tp.Wcs.from_header({**base, "CTYPE1": "RA---ZEA", "CTYPE2": "DEC--ZEA"})
    with pytest.raises(ValueError):
        tp.Wcs.from_header({**base, "CTYPE2": "DEC--SIN"})  # mismatched projections
    with pytest.raises(ValueError):
        tp.Wcs.from_header({**base, "CRPIX12345": 1.0})  # keyword too long
    with pytest.raises(ValueError):
        tp.Wcs.from_header({**base, "CRVAL1": float("nan")})
    with pytest.raises(ValueError):
        tp.Wcs.from_header({**base, "CRVAL1": [1, 2]})
    with pytest.raises(ValueError):
        tp.Wcs.from_header({**base, "CD1_1": 0.0, "CD1_2": 0.0, "CD2_1": 0.0, "CD2_2": 0.0})
