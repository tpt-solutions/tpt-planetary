"""Reads Zarr arrays written by this crate's Blosc encoder with zarr-python.

Produce the arrays first (from the repository root):

    cargo run -p tpt-planetary-array --example write_blosc_zarr -- <out_dir>

then run, with a Python that has `zarr`, `numcodecs` and `numpy`:

    python tpt-planetary-array/tests/fixtures/check_blosc_written.py <out_dir>

Each array must hold 0.25 * row + 1.5 * col over a 50 x 40 grid. A pass shows
that zarr-python's Blosc decoder (c-blosc) accepts the frames this crate writes,
including the split and shuffled layouts.
"""

import os
import sys

import numpy as np
import zarr

NAMES = [
    "lz4_noshuffle",
    "lz4_shuffle",
    "lz4_bitshuffle",
    "zlib_shuffle",
    "zstd_shuffle",
    "zstd_bitshuffle",
]


def main(out_dir):
    rows, cols = 50, 40
    expected = 0.25 * np.arange(rows)[:, None] + 1.5 * np.arange(cols)[None, :]
    failures = []
    for name in NAMES:
        path = os.path.join(out_dir, name)
        try:
            arr = zarr.open_array(path, mode="r")
            got = np.asarray(arr[:])
            if got.shape != expected.shape or not np.array_equal(got, expected):
                failures.append(f"{name}: values differ")
            else:
                print(f"OK {name} ({arr.compressors})")
        except Exception as e:  # noqa: BLE001 - report every array
            failures.append(f"{name}: {type(e).__name__}: {e}")
    for f in failures:
        print("FAIL", f)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
