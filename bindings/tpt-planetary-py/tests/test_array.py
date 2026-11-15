"""Tests for opening, reading and reducing local Zarr v2 arrays.

Every array is written by hand with the conftest helper (raw chunk files and a
``.zarray`` document) and compared against the same NumPy data.
"""

import threading
import time

import numpy as np
import pytest

import tpt_planetary as tp


@pytest.fixture
def grid():
    # 5 x 7 with chunks 2 x 3: partial chunks on both edges.
    return np.arange(35, dtype=np.float64).reshape(5, 7) * 0.5 - 3.0


@pytest.fixture
def store(zarr_v2, grid):
    return zarr_v2(grid, (2, 3))


def test_metadata_properties(store):
    arr = tp.open_zarr(store)
    assert arr.shape == (5, 7)
    assert arr.ndim == 2
    assert arr.chunks == (2, 3)
    assert arr.dtype == "<f8"
    assert arr.path == ""


def test_open_accepts_str_and_pathlike(store):
    assert tp.open_zarr(str(store)).shape == (5, 7)


def test_full_read_matches_numpy(store, grid):
    out = tp.open_zarr(store).read()
    assert out.dtype == np.float64
    np.testing.assert_array_equal(out, grid)


@pytest.mark.parametrize(
    "sel",
    [
        (slice(1, 4), slice(None)),
        (slice(None, None, 2), slice(1, 6, 3)),
        (slice(-3, -1), slice(-4, None)),
        (2, slice(None)),
        (slice(0, 5), 6),
        (slice(10, 20), slice(None)),  # clipped like NumPy
    ],
)
def test_slices_match_numpy(store, grid, sel):
    out = tp.open_zarr(store).read(sel)
    np.testing.assert_array_equal(out, grid[sel])


def test_single_index_removes_dimension(store, grid):
    out = tp.open_zarr(store).read(1)
    np.testing.assert_array_equal(out, grid[1])
    assert out.shape == (7,)


def test_missing_trailing_dimensions_mean_everything(store, grid):
    np.testing.assert_array_equal(tp.open_zarr(store).read((slice(0, 2),)), grid[0:2])


def test_all_indices_return_a_float(store, grid):
    value = tp.open_zarr(store).read((3, 4))
    assert isinstance(value, float)
    assert value == grid[3, 4]


def test_read_with_none_selection(store, grid):
    np.testing.assert_array_equal(tp.open_zarr(store).read(None), grid)


def test_integer_store_reads_as_float64(zarr_v2):
    data = np.arange(12, dtype="<i4").reshape(3, 4)
    arr = tp.open_zarr(zarr_v2(data, (2, 2), name="ints"))
    assert arr.dtype == "<i4"
    out = arr.read()
    assert out.dtype == np.float64
    np.testing.assert_array_equal(out, data.astype(np.float64))


@pytest.mark.parametrize(
    "sel",
    [
        (slice(None, None, 0),),  # zero step
        (slice(None, None, -1),),  # negative step
        (1, 2, 3),  # too many indices
        (True,),  # bool is not an index
        ("a",),  # string index
        (None, 1),  # None inside a tuple
        (...,),  # ellipsis unsupported
    ],
)
def test_bad_selections_are_value_errors(store, sel):
    with pytest.raises(ValueError):
        tp.open_zarr(store).read(sel)


def test_out_of_range_index_is_value_error(store):
    with pytest.raises(ValueError):
        tp.open_zarr(store).read(99)


def test_reductions_match_numpy(store, grid):
    arr = tp.open_zarr(store)
    assert arr.sum() == pytest.approx(float(grid.sum()), rel=1e-12)
    assert arr.mean() == pytest.approx(float(grid.mean()), rel=1e-12)
    assert arr.min() == float(grid.min())
    assert arr.max() == float(grid.max())


def test_reductions_on_larger_multichunk_array(zarr_v2):
    rng = np.random.default_rng(42)
    data = rng.normal(size=(300, 257))
    arr = tp.open_zarr(zarr_v2(data, (64, 50), name="big"))
    assert arr.sum() == pytest.approx(float(data.sum()), rel=1e-9)
    assert arr.mean() == pytest.approx(float(data.mean()), rel=1e-9)
    assert arr.min() == float(data.min())
    assert arr.max() == float(data.max())


def test_one_dimensional_and_zero_size(zarr_v2):
    data = np.linspace(-1, 1, 11)
    arr = tp.open_zarr(zarr_v2(data, (4,), name="line"))
    np.testing.assert_array_equal(arr.read(slice(2, 9, 3)), data[2:9:3])

    empty = tp.open_zarr(zarr_v2(np.zeros((0, 3)), (2, 3), name="empty"))
    assert empty.mean() is None
    assert empty.min() is None
    assert empty.max() is None
    assert empty.sum() == 0.0
    assert empty.read().shape == (0, 3)


def test_missing_path_is_file_not_found(tmp_path):
    with pytest.raises(FileNotFoundError):
        tp.open_zarr(tmp_path / "does-not-exist.zarr")


def test_directory_without_metadata_is_os_error(tmp_path):
    (tmp_path / "plain").mkdir()
    with pytest.raises(OSError):
        tp.open_zarr(tmp_path / "plain")


def test_corrupt_metadata_is_library_error(tmp_path):
    root = tmp_path / "bad"
    root.mkdir()
    (root / ".zarray").write_text("{not json", encoding="utf-8")
    with pytest.raises((tp.TptPlanetaryError, ValueError)):
        tp.open_zarr(root)


def test_reduction_releases_the_gil(zarr_v2):
    # A reduction over ~32 MB should let a pure-Python thread make progress.
    # Best-effort check: the ticker must advance while the reduction runs.
    rng = np.random.default_rng(0)
    data = rng.normal(size=(2000, 2000))
    arr = tp.open_zarr(zarr_v2(data, (250, 250), name="gil"))

    ticks = 0
    stop = threading.Event()

    def ticker():
        nonlocal ticks
        while not stop.is_set():
            ticks += 1
            time.sleep(0)

    t = threading.Thread(target=ticker)
    t.start()
    deltas = []
    try:
        for _ in range(3):
            before = ticks
            arr.sum()
            deltas.append(ticks - before)
    finally:
        stop.set()
        t.join()
    # If the GIL were held for the whole reduction, the ticker could not run
    # while it executed and every delta would be zero.
    assert max(deltas) > 0


@pytest.mark.parametrize(
    "sel",
    [
        (2**62,),
        (-(2**62),),
        (slice(None, None, 2**62),),
        (slice(-(2**63), None),),
        (slice(0, 2**63 - 1),),
        (slice(2**63 - 1, 2**63 - 1, 1),),
        (slice(None, None, 2**63 - 1),),
    ],
)
def test_huge_indices_do_not_panic(store, sel):
    # Either a NumPy-like clipped result or a ValueError; never a PanicException.
    try:
        out = tp.open_zarr(store).read(sel)
    except ValueError:
        return
    assert isinstance(out, (np.ndarray, float))
