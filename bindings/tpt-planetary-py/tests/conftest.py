"""Shared helpers for the tpt_planetary test suite.

The Zarr helper writes Zarr v2 directories by hand, following the v2 spec:
a ``.zarray`` JSON file, and one raw C-order file per chunk named by its chunk
indices with ``.`` as separator. Edge chunks are stored padded to the full
chunk shape, as the spec requires.
"""

from __future__ import annotations

import itertools
import json
from pathlib import Path

import numpy as np
import pytest


def write_zarr_v2(
    root: Path,
    data: np.ndarray,
    chunks: tuple[int, ...],
    *,
    fill_value=None,
) -> Path:
    """Write ``data`` as a Zarr v2 array (uncompressed, C order) at ``root``."""
    root.mkdir(parents=True, exist_ok=True)
    data = np.ascontiguousarray(data)
    shape = data.shape
    meta = {
        "zarr_format": 2,
        "shape": list(shape),
        "chunks": list(chunks),
        "dtype": data.dtype.str,
        "compressor": None,
        "fill_value": fill_value,
        "order": "C",
        "filters": None,
    }
    (root / ".zarray").write_text(json.dumps(meta), encoding="utf-8")

    grid = [-(-s // c) for s, c in zip(shape, chunks)]  # ceil division
    for idx in itertools.product(*(range(n) for n in grid)):
        block = np.zeros(chunks, dtype=data.dtype)
        if fill_value is not None:
            block[...] = fill_value
        src = tuple(slice(i * c, min((i + 1) * c, s)) for i, c, s in zip(idx, chunks, shape))
        dst = tuple(slice(0, sl.stop - sl.start) for sl in src)
        block[dst] = data[src]
        key = ".".join(str(i) for i in idx)
        (root / key).write_bytes(block.tobytes(order="C"))
    return root


@pytest.fixture
def zarr_v2(tmp_path):
    """Factory fixture: ``zarr_v2(data, chunks, name=...) -> Path``."""

    def make(data, chunks, name="arr", **kw):
        return write_zarr_v2(tmp_path / name, data, chunks, **kw)

    return make
