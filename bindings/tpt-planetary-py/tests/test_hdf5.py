"""Tests for the HDF5 bindings (open_hdf5 and Hdf5File).

The fixtures are the files written by h5py in tpt-planetary-array/tests/fixtures/hdf5
(see tools/gen_hdf5_fixtures.py), with the values h5py wrote in expected.json.
"""

import json
from pathlib import Path

import numpy as np
import pytest

import tpt_planetary as tp

FIXTURES = Path(__file__).resolve().parents[3] / "tpt-planetary-array" / "tests" / "fixtures" / "hdf5"
FILES = ["basic.h5", "chunked.h5", "scales.h5", "latest.h5"]


@pytest.fixture(scope="module")
def expected():
    return json.loads((FIXTURES / "expected.json").read_text(encoding="utf-8"))


def _file_with(path):
    for name in FILES:
        f = tp.open_hdf5(FIXTURES / name)
        try:
            if f.kind(path) == "dataset":
                return f
        except FileNotFoundError:
            continue
    raise AssertionError(f"{path} not found")


def test_every_dataset_matches_h5py(expected):
    for path, values in expected.items():
        got = _file_with(path).read(path)
        np.testing.assert_allclose(np.ravel(got), values, rtol=1e-14, atol=0)


def test_groups_and_shapes():
    f = tp.open_hdf5(FIXTURES / "basic.h5")
    assert f.kind("/") == "group"
    assert sorted(f.list("/")) == ["cube", "outer", "scalar", "vector"]
    assert f.shape("/cube") == (2, 3, 4)
    assert f.shape("/scalar") == ()
    assert f.read("/scalar") == pytest.approx(3.25)


def test_attributes_come_back_as_text_and_numbers():
    f = tp.open_hdf5(FIXTURES / "basic.h5")
    root = f.attributes("/")
    assert root["title"] == "basic fixture"
    assert root["count"] == pytest.approx(7.0)
    vec = f.attributes("/vector")
    assert vec["units"] == "m s-1"
    np.testing.assert_array_equal(vec["valid"], [1.0, 2.0])


def test_missing_path_and_non_hdf5_file(tmp_path):
    f = tp.open_hdf5(FIXTURES / "basic.h5")
    with pytest.raises(FileNotFoundError):
        f.read("/no/such/dataset")
    bad = tmp_path / "bad.h5"
    bad.write_bytes(b"not hdf5" * 50)
    with pytest.raises(tp.TptPlanetaryError):
        tp.open_hdf5(bad)


def test_missing_file_is_os_error(tmp_path):
    with pytest.raises(OSError):
        tp.open_hdf5(tmp_path / "missing.h5")
