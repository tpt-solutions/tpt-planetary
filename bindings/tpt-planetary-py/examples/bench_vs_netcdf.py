"""Time open, slice and mean on one 2-D float64 field, in several libraries.

Writes the same field twice into a temporary directory:

* a NetCDF-3 classic file (``CDF-1``) through netCDF4 (``NETCDF3_CLASSIC``);
* a Zarr v2 directory written by hand (``.zarray`` plus chunk files).

Then it times three operations, each run from a fresh open:

* ``open``: open the dataset and read its shape and dtype, no data;
* ``slice``: open, then read the block ``[200:520, 300:900]`` into NumPy;
* ``mean``: open, then the mean over the whole field.

Libraries:

* ``netCDF4 + numpy``: reads the NetCDF-3 file;
* ``xarray + numpy``: reads the NetCDF-3 file through the netCDF4 engine;
* ``tpt_planetary``: reads the Zarr v2 directory. The Python bindings cannot
  read NetCDF-3, so this is the Zarr path, not the same file.

Each operation runs one warm-up and then ``--runs`` timed runs (default 5);
the median is reported. Run it with the project venv:

    python bindings/tpt-planetary-py/examples/bench_vs_netcdf.py --runs 5
"""

from __future__ import annotations

import argparse
import gc
import json
import platform
import statistics
import tempfile
import time
from pathlib import Path

import netCDF4
import numpy as np
import xarray as xr

import tpt_planetary as tp

BLOCK = (slice(200, 520), slice(300, 900))


def write_netcdf3(path: Path, data: np.ndarray) -> None:
    with netCDF4.Dataset(path, "w", format="NETCDF3_CLASSIC") as ds:
        ds.createDimension("lat", data.shape[0])
        ds.createDimension("lon", data.shape[1])
        var = ds.createVariable("sst", "f8", ("lat", "lon"))
        var[:] = data


def write_zarr_v2(root: Path, data: np.ndarray, chunks: tuple[int, int]) -> None:
    """Zarr v2 array with uncompressed little-endian float64 chunks."""
    root.mkdir(parents=True, exist_ok=True)
    data = np.ascontiguousarray(data, dtype="<f8")
    meta = {
        "zarr_format": 2,
        "shape": list(data.shape),
        "chunks": list(chunks),
        "dtype": "<f8",
        "compressor": None,
        "fill_value": "NaN",
        "order": "C",
        "filters": None,
    }
    (root / ".zarray").write_text(json.dumps(meta))
    grid = [-(-s // c) for s, c in zip(data.shape, chunks)]
    for idx in np.ndindex(*grid):
        sl = tuple(slice(i * c, min((i + 1) * c, s)) for i, c, s in zip(idx, chunks, data.shape))
        block = data[sl]
        if block.shape != chunks:
            padded = np.full(chunks, np.nan, dtype="<f8")
            padded[tuple(slice(0, n) for n in block.shape)] = block
            block = padded
        (root / ".".join(map(str, idx))).write_bytes(block.tobytes(order="C"))


def op_netcdf4_open(path: Path):
    with netCDF4.Dataset(path) as ds:
        var = ds.variables["sst"]
        return var.shape, var.dtype


def op_netcdf4_slice(path: Path):
    with netCDF4.Dataset(path) as ds:
        return np.asarray(ds.variables["sst"][BLOCK[0], BLOCK[1]])


def op_netcdf4_mean(path: Path):
    with netCDF4.Dataset(path) as ds:
        return float(np.asarray(ds.variables["sst"][:]).mean())


def op_xarray_open(path: Path):
    with xr.open_dataset(path, engine="netcdf4") as ds:
        return ds["sst"].shape, ds["sst"].dtype


def op_xarray_slice(path: Path):
    with xr.open_dataset(path, engine="netcdf4") as ds:
        return np.asarray(ds["sst"].isel(lat=BLOCK[0], lon=BLOCK[1]).values)


def op_xarray_mean(path: Path):
    with xr.open_dataset(path, engine="netcdf4") as ds:
        return float(ds["sst"].values.mean())


def op_tpt_open(path: Path):
    arr = tp.open_zarr(path)
    return arr.shape, arr.dtype


def op_tpt_slice(path: Path):
    return tp.open_zarr(path).read(BLOCK)


def op_tpt_mean(path: Path):
    return tp.open_zarr(path).mean()


def time_op(fn, path: Path, runs: int) -> tuple[float, float, float]:
    fn(path)  # warm-up: populates the OS file cache
    samples = []
    for _ in range(runs):
        gc.collect()
        t0 = time.perf_counter()
        fn(path)
        samples.append(time.perf_counter() - t0)
    return statistics.median(samples), min(samples), max(samples)


def machine_info() -> dict[str, str]:
    return {
        "platform": platform.platform(),
        "processor": platform.processor(),
        "python": platform.python_version(),
        "numpy": np.__version__,
        "xarray": xr.__version__,
        "netCDF4": netCDF4.__version__,
        "tpt_planetary": tp.__version__,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--nlat", type=int, default=720)
    parser.add_argument("--nlon", type=int, default=1440)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--workdir", type=Path, default=None,
                        help="directory for the generated files (default: a temporary directory)")
    args = parser.parse_args()

    rng = np.random.default_rng(12345)
    field = rng.normal(15.0, 10.0, size=(args.nlat, args.nlon))
    reference_mean = float(field.mean())
    nbytes = field.nbytes

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(args.workdir) if args.workdir else Path(tmp)
        root.mkdir(parents=True, exist_ok=True)
        nc_path = root / "field.nc"
        zarr_path = root / "field.zarr"
        write_netcdf3(nc_path, field)
        write_zarr_v2(zarr_path, field, chunks=(180, 360))

        # Correctness: every library must agree on the mean before timing.
        means = {
            "netCDF4": op_netcdf4_mean(nc_path),
            "xarray": op_xarray_mean(nc_path),
            "tpt_planetary": op_tpt_mean(zarr_path),
        }
        for name, value in means.items():
            assert abs(value - reference_mean) <= 1e-9 * abs(reference_mean), (name, value)

        rows = []
        cases = [
            ("netCDF4 + numpy (NetCDF-3)", op_netcdf4_open, op_netcdf4_slice, op_netcdf4_mean, nc_path),
            ("xarray + numpy (NetCDF-3)", op_xarray_open, op_xarray_slice, op_xarray_mean, nc_path),
            ("tpt_planetary (Zarr v2)", op_tpt_open, op_tpt_slice, op_tpt_mean, zarr_path),
        ]
        for name, open_fn, slice_fn, mean_fn, path in cases:
            row = {"library": name}
            for op, fn in (("open", open_fn), ("slice", slice_fn), ("mean", mean_fn)):
                med, lo, hi = time_op(fn, path, args.runs)
                row[op] = (med, lo, hi)
            rows.append(row)

    info = machine_info()
    print(f"field: float64 {args.nlat} x {args.nlon} = {nbytes / 1e6:.1f} MB; runs = {args.runs}")
    for key, value in info.items():
        print(f"{key}: {value}")
    print()
    print("| library | open (ms) | slice (ms) | mean (ms) |")
    print("|---|---:|---:|---:|")
    for row in rows:
        cells = [f"{row[op][0] * 1e3:.2f} ({row[op][1] * 1e3:.2f}-{row[op][2] * 1e3:.2f})"
                 for op in ("open", "slice", "mean")]
        print(f"| {row['library']} | " + " | ".join(cells) + " |")
    print("\ncells: median (min-max) over the timed runs, each run includes a fresh open")


if __name__ == "__main__":
    main()
