#!/usr/bin/env python3
"""Write the HDF5 test fixtures and their expected values.

Files go to tpt-planetary-array/tests/fixtures/hdf5/:
  basic.h5      contiguous datasets of several types, nested groups, attributes
  chunked.h5    chunked datasets: deflate, deflate with shuffle, a fill value,
                and a dataset with missing chunks
  scales.h5     netCDF-4 style dimension scales (variables with DIMENSION_LIST)
  latest.h5     libver='latest' (superblock v3, version 2 object headers, links)
  expected.json the values each dataset must read back as (float64)

Written with h5py (HDF5 1.14 library, default libver, so the files use the
superblock v0 and object header v1 layouts that the reader supports).

Usage: python tools/gen_hdf5_fixtures.py     (requires h5py and numpy)
"""

import json
from pathlib import Path

import h5py
import numpy as np

OUT = Path(__file__).resolve().parent.parent / "tpt-planetary-array" / "tests" / "fixtures" / "hdf5"


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    expected: dict[str, list] = {}
    rng = np.random.default_rng(20260201)

    with h5py.File(OUT / "basic.h5", "w") as f:
        # Fixed-length strings (variable-length ones are not read by the reader).
        f.attrs.create("title", data=np.bytes_(b"basic fixture"))
        f.attrs["count"] = np.int32(7)
        grp = f.create_group("outer/inner")
        grp.attrs["scale"] = 2.5
        dtypes = {
            "i8": np.int8, "i16": np.int16, "i32": np.int32, "i64": np.int64,
            "u8": np.uint8, "u16": np.uint16, "u32": np.uint32, "u64": np.uint64,
            "f32": np.float32, "f64": np.float64,
        }
        for name, dt in dtypes.items():
            values = (np.arange(12) - 4).reshape(3, 4).astype(dt)
            grp.create_dataset(name, data=values)
            expected[f"/outer/inner/{name}"] = values.astype(np.float64).ravel().tolist()
        scalar = np.float64(3.25)
        f.create_dataset("scalar", data=scalar)
        expected["/scalar"] = [3.25]
        vec = rng.normal(size=5)
        f.create_dataset("vector", data=vec)
        expected["/vector"] = vec.tolist()
        cube = rng.integers(-100, 100, size=(2, 3, 4)).astype(np.int16)
        f.create_dataset("cube", data=cube)
        expected["/cube"] = cube.astype(np.float64).ravel().tolist()
        f["vector"].attrs.create("units", data=np.bytes_(b"m s-1"))
        f["vector"].attrs["valid"] = np.array([1, 2], dtype=np.int32)

    with h5py.File(OUT / "chunked.h5", "w") as f:
        data = rng.normal(size=(7, 9))
        f.create_dataset("plain", data=data, chunks=(3, 4))
        expected["/plain"] = data.ravel().tolist()
        packed = rng.integers(0, 1000, size=(8, 10)).astype(np.int32)
        f.create_dataset("deflate", data=packed, chunks=(4, 5), compression="gzip", compression_opts=4)
        expected["/deflate"] = packed.astype(np.float64).ravel().tolist()
        shuffled = rng.integers(0, 60000, size=(9, 6)).astype(np.uint16)
        f.create_dataset("shuffled", data=shuffled, chunks=(4, 4), compression="gzip", shuffle=True)
        expected["/shuffled"] = shuffled.astype(np.float64).ravel().tolist()
        # Only some chunks are written; the others read as the fill value.
        sparse = f.create_dataset("sparse", shape=(6, 6), dtype="f8", chunks=(3, 3), fillvalue=-9.0)
        sparse[0:3, 0:3] = np.arange(9.0).reshape(3, 3)
        sparse[3:6, 3:6] = 100.0
        dense_expect = np.full((6, 6), -9.0)
        dense_expect[0:3, 0:3] = np.arange(9.0).reshape(3, 3)
        dense_expect[3:6, 3:6] = 100.0
        expected["/sparse"] = dense_expect.ravel().tolist()

    with h5py.File(OUT / "scales.h5", "w") as f:
        time = f.create_dataset("time", data=np.arange(4, dtype=np.float64))
        lat = f.create_dataset("lat", data=np.array([-30.0, 0.0, 30.0]))
        time.make_scale("time")
        lat.make_scale("lat")
        temp = f.create_dataset("temp", data=rng.normal(size=(4, 3)), chunks=(2, 3), compression="gzip")
        temp.dims[0].attach_scale(time)
        temp.dims[1].attach_scale(lat)
        temp.attrs.create("units", data=np.bytes_(b"K"))
        expected["/temp"] = temp[...].ravel().tolist()
        expected["/time"] = [0.0, 1.0, 2.0, 3.0]

    # libver='latest': superblock v3, version 2 object headers and link messages.
    with h5py.File(OUT / "latest.h5", "w", libver="latest") as f:
        f.attrs.create("format", data=np.bytes_(b"latest"))
        g = f.create_group("results/run1")
        g.attrs["n"] = np.int64(3)
        values = np.arange(6, dtype=np.float32).reshape(2, 3) * 0.5
        g.create_dataset("values", data=values)
        expected["/results/run1/values"] = values.astype(np.float64).ravel().tolist()
        f.create_dataset("flags", data=np.array([1, 0, 1, 1], dtype=np.uint8))
        # Chunked storage under libver=latest uses the version 4 layout, which the reader does not support.
        f.create_dataset("chunked_v4", data=np.arange(6.0), chunks=(3,))
        expected["/flags"] = [1.0, 0.0, 1.0, 1.0]

    with open(OUT / "expected.json", "w", encoding="utf-8") as fh:
        json.dump(expected, fh, indent=1)
    print(f"wrote fixtures to {OUT}: {len(expected)} datasets")


if __name__ == "__main__":
    main()
