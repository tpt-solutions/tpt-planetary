"""Niño-3.4 indices from a Zarr field, against a direct NumPy computation.

The reference uses the definitions of ``tpt_planetary_array::ops::climate``:
the anomaly is each value minus the mean of its calendar month over the full
years of the record, and a box mean weights each latitude by cos(latitude).
"""

import numpy as np
import pytest

import tpt_planetary as tp

LAT = np.array([-10.0, -5.0, 0.0, 5.0, 10.0])
LON = np.array([180.0, 190.0, 200.0, 210.0, 220.0, 230.0, 240.0, 250.0])


@pytest.fixture
def field():
    # Two years of monthly data with a seasonal cycle and a trend-free pattern.
    rng = np.random.default_rng(7)
    t = np.arange(24, dtype=np.float64)
    seasonal = np.cos(2 * np.pi * t / 12.0)[:, None, None]
    spatial = rng.normal(size=(1, LAT.size, LON.size))
    return seasonal * (1.0 + 0.1 * spatial) + 0.2 * spatial


def _box_mean(values, lat_sel, lon_sel):
    w = np.cos(np.radians(LAT))[:, None] * np.ones((1, LON.size))
    mask = np.zeros((LAT.size, LON.size), dtype=bool)
    mask[np.ix_(lat_sel, lon_sel)] = True
    ww = np.where(mask, w, 0.0)
    return (values * ww).sum(axis=(1, 2)) / ww.sum()


def _anomaly(values):
    clim = (values[:12] + values[12:]) / 2.0
    return values - np.concatenate([clim, clim])


def _box_indices(lat_lo, lat_hi, lon_lo, lon_hi):
    lat_sel = np.flatnonzero((LAT >= lat_lo) & (LAT <= lat_hi))
    lon_sel = np.flatnonzero((LON >= lon_lo) & (LON <= lon_hi))
    return lat_sel, lon_sel


def test_nino34_matches_numpy(zarr_v2, field):
    store = zarr_v2(field, (5, 5, 4))
    arr = tp.open_zarr(store)
    anom = _anomaly(field)
    nino = _box_mean(anom, *_box_indices(-5.0, 5.0, 190.0, 240.0))
    got = tp.nino34(arr, LAT.tolist(), LON.tolist())
    assert got.shape == (24,)
    np.testing.assert_allclose(got, nino, rtol=1e-12, atol=1e-12)


def test_nino34_relative_subtracts_the_tropical_mean(zarr_v2, field):
    store = zarr_v2(field, (5, 5, 4))
    arr = tp.open_zarr(store)
    anom = _anomaly(field)
    nino = _box_mean(anom, *_box_indices(-5.0, 5.0, 190.0, 240.0))
    tropics = _box_mean(anom, *_box_indices(-20.0, 20.0, 0.0, 360.0))
    got = tp.nino34_relative(arr, LAT.tolist(), LON.tolist())
    np.testing.assert_allclose(got, nino - tropics, rtol=1e-12, atol=1e-12)


def test_empty_box_is_an_error(zarr_v2):
    # A grid of longitudes 0..20 has no point in 190..240, so the box is empty.
    west = np.ones((24, LAT.size, 3))
    store = zarr_v2(west, (5, 5, 3))
    arr = tp.open_zarr(store)
    with pytest.raises(ValueError, match="no longitude grid points"):
        tp.nino34(arr, LAT.tolist(), [0.0, 10.0, 20.0])


def test_coordinate_length_mismatch_is_an_error(zarr_v2, field):
    store = zarr_v2(field, (5, 5, 4))
    arr = tp.open_zarr(store)
    with pytest.raises(ValueError, match="coordinate lengths"):
        tp.nino34(arr, [-5.0, 0.0], LON.tolist())
