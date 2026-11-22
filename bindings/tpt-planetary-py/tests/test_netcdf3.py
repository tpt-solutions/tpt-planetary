"""Tests for opening and reading NetCDF-3 files.

The fixtures are files written by netCDF4 (classic and 64-bit offset) that the
Rust interop tests already check value by value. Here they are checked for
structure: the variables, their shapes, and that each read gives float64 data
of the declared size.
"""

from pathlib import Path

import numpy as np
import pytest

import tpt_planetary as tp

FIXTURES = Path(__file__).resolve().parents[3] / "tpt-planetary-array" / "tests" / "fixtures" / "netcdf3"


@pytest.mark.parametrize("name", ["netcdf4lib_classic.nc", "netcdf4lib_64bit.nc"])
def test_every_variable_reads_with_its_declared_shape(name):
    f = tp.open_netcdf3(FIXTURES / name)
    assert f.variables, "fixture has no variables"
    for var in f.variables:
        shape = f.shape(var)
        value = f.read(var)
        if shape == ():
            assert isinstance(value, float)
        else:
            assert isinstance(value, np.ndarray)
            assert value.dtype == np.float64
            assert value.shape == shape


def test_dimensions_are_a_dict_of_lengths():
    f = tp.open_netcdf3(FIXTURES / "netcdf4lib_classic.nc")
    dims = f.dimensions
    assert isinstance(dims, dict)
    assert all(isinstance(v, int) and v >= 0 for v in dims.values())


def test_str_and_pathlike_are_accepted():
    path = FIXTURES / "netcdf4lib_classic.nc"
    assert tp.open_netcdf3(str(path)).variables == tp.open_netcdf3(path).variables


def test_missing_file_is_os_error(tmp_path):
    with pytest.raises(OSError):
        tp.open_netcdf3(tmp_path / "nope.nc")


def test_non_netcdf_file_is_rejected(tmp_path):
    bad = tmp_path / "bad.nc"
    bad.write_bytes(b"not a netcdf file at all")
    with pytest.raises(tp.TptPlanetaryError):
        tp.open_netcdf3(bad)


def test_unknown_variable_is_file_not_found():
    f = tp.open_netcdf3(FIXTURES / "netcdf4lib_classic.nc")
    with pytest.raises(FileNotFoundError):
        f.read("no_such_variable")


def _first_multidimensional_variable(f):
    for var in f.variables:
        if len(f.shape(var)) >= 2:
            return var
    pytest.skip("fixture has no variable of rank 2 or more")


@pytest.mark.parametrize("name", ["netcdf4lib_classic.nc", "netcdf4lib_64bit.nc"])
def test_selections_match_numpy_indexing(name):
    f = tp.open_netcdf3(FIXTURES / name)
    var = _first_multidimensional_variable(f)
    full = f.read(var)
    shape = full.shape
    selections = [
        (0,) + tuple(slice(None) for _ in shape[1:]),
        tuple(slice(None, None, 2) for _ in shape),
        tuple(slice(1, None) for _ in shape),
        tuple(slice(-1, None) for _ in shape),
        (shape[0] - 1,) + tuple(slice(0, 1) for _ in shape[1:]),
    ]
    for sel in selections:
        got = f.read(var, sel)
        expected = full[sel]
        assert isinstance(got, np.ndarray)
        assert got.shape == expected.shape
        np.testing.assert_array_equal(got, expected)


def test_integer_selections_on_a_vector_give_a_float():
    f = tp.open_netcdf3(FIXTURES / "netcdf4lib_classic.nc")
    for var in f.variables:
        shape = f.shape(var)
        if len(shape) == 1 and shape[0] > 0:
            value = f.read(var, shape[0] - 1)
            assert isinstance(value, float)
            assert value == f.read(var)[-1]
            return
    pytest.skip("fixture has no non-empty vector variable")
