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
