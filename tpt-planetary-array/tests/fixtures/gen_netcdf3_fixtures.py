"""Generates NetCDF-3 interop fixtures with netCDF4-python (the netCDF C library).

Run from the repository root with a Python that has `netCDF4` and `numpy`:

    python tpt-planetary-array/tests/fixtures/gen_netcdf3_fixtures.py

Writes, into `netcdf3/`:
- `netcdf4lib_classic.nc` and `netcdf4lib_64bit.nc`: CDF-1 and CDF-2 files with
  a record (unlimited) dimension, several numeric types, CF time units with a
  calendar, and global and variable attributes.
- `netcdf4lib_values.json`: the values the Rust test must read back, as JSON
  lists keyed by variable name, plus the attribute text checked.

The files are produced by the netCDF library, not by tpt-planetary, so they
test the reader against an independent writer.
"""

import json
import os

import netCDF4
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "netcdf3")


def write(path, fmt):
    ds = netCDF4.Dataset(path, "w", format=fmt)
    ds.title = "interop fixture"
    ds.institution = "netCDF4-python"
    time = ds.createDimension("time", None)
    lat = ds.createDimension("lat", 3)
    lon = ds.createDimension("lon", 4)

    t = ds.createVariable("time", "f8", ("time",))
    t.units = "days since 2000-01-01"
    t.calendar = "noleap"
    t[:] = np.array([0.0, 31.0, 59.5, 365.0])

    la = ds.createVariable("lat", "f4", ("lat",))
    la.units = "degrees_north"
    la[:] = np.array([-10.0, 0.0, 10.5], dtype="f4")

    lo = ds.createVariable("lon", "i4", ("lon",))
    lo.units = "degrees_east"
    lo[:] = np.arange(4, dtype="i4") * 90

    temp = ds.createVariable("temp", "f8", ("time", "lat", "lon"))
    temp.units = "K"
    temp.long_name = "air temperature"
    grid = np.arange(4 * 3 * 4, dtype="f8").reshape(4, 3, 4)
    temp[:, :, :] = 250.0 + grid * 0.25

    code = ds.createVariable("code", "i2", ("lat", "lon"))
    code.scale_note = "small integers"
    code[:, :] = (np.arange(12, dtype="i2") - 6).reshape(3, 4)

    flag = ds.createVariable("flag", "i1", ("time",))
    flag[:] = np.array([1, -1, 0, 127], dtype="i1")

    ds.close()


def expected_values():
    grid = np.arange(4 * 3 * 4, dtype="f8").reshape(4, 3, 4)
    return {
        "time": [0.0, 31.0, 59.5, 365.0],
        "lat": [-10.0, 0.0, 10.5],
        "lon": [0.0, 90.0, 180.0, 270.0],
        "temp": (250.0 + grid * 0.25).reshape(-1).tolist(),
        "code": (np.arange(12) - 6).tolist(),
        "flag": [1, -1, 0, 127],
    }


def main():
    os.makedirs(OUT, exist_ok=True)
    write(os.path.join(OUT, "netcdf4lib_classic.nc"), "NETCDF3_CLASSIC")
    write(os.path.join(OUT, "netcdf4lib_64bit.nc"), "NETCDF3_64BIT_OFFSET")
    with open(os.path.join(OUT, "netcdf4lib_values.json"), "w") as f:
        json.dump(
            {
                "netCDF4": netCDF4.__version__,
                "title": "interop fixture",
                "values": expected_values(),
            },
            f,
            indent=1,
        )
    print("wrote fixtures to", OUT)


if __name__ == "__main__":
    main()
