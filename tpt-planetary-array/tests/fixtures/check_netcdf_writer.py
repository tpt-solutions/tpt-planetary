"""Checks a NetCDF-3 file written by tpt-planetary with netCDF4-python and xarray.

Run with a Python that has `netCDF4`, `xarray` and `numpy`:

    python tpt-planetary-array/tests/fixtures/check_netcdf_writer.py

It checks the checked-in file `netcdf3/tpt_from_zarr_v2_zlib.nc`, which was
produced by `tptplan array convert tests/fixtures/zarr/v2_zlib out.nc`, against
the expected values of that Zarr fixture (`zarr/v2_zlib.expected.raw`). A
passing run shows that the writer's output is read correctly by the netCDF
library and by xarray, which is an independent check of the file format.
"""

import os
import sys

import netCDF4
import numpy as np
import xarray as xr

HERE = os.path.dirname(os.path.abspath(__file__))


def main(path=None):
    path = path or os.path.join(HERE, "netcdf3", "tpt_from_zarr_v2_zlib.nc")
    expected = np.frombuffer(
        open(os.path.join(HERE, "zarr", "v2_zlib.expected.raw"), "rb").read(), dtype="<f8"
    ).reshape(12, 10)

    with netCDF4.Dataset(path) as ds:
        assert ds.file_format in ("NETCDF3_CLASSIC", "NETCDF3_64BIT_OFFSET"), ds.file_format
        variables = list(ds.variables)
        assert len(variables) == 1, variables
        name = variables[0]
        got = np.asarray(ds.variables[name][:])
        assert got.shape == expected.shape, (got.shape, expected.shape)
        assert np.array_equal(got, expected), "netCDF4 values differ"
        print("netCDF4 OK:", name, got.shape, ds.file_format)

    da = xr.open_dataset(path)[name]
    assert np.array_equal(da.values, expected), "xarray values differ"
    print("xarray OK:", name, da.dims, da.shape)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else None))
