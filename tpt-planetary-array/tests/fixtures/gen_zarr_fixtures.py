"""Generates Zarr v2 and v3 conformance fixtures with zarr-python and numcodecs.

Run from the repository root with a Python that has `zarr` (3.x), `numcodecs`
and `numpy` installed:

    python tpt-planetary-array/tests/fixtures/gen_zarr_fixtures.py

Each fixture is a Zarr store directory under `zarr/<name>/` holding the array,
and `zarr/<name>.expected.raw` with the expected values as little-endian bytes
in C order (the values are what zarr-python reads back, so the Rust reader is
compared against the reference implementation). Missing chunks are left out
of the store on purpose, so the fill value is exercised.

The fixtures are regenerated only by hand; the Rust tests read the checked-in
files and do not need Python.
"""

import json
import os
import shutil

import numcodecs
import numpy as np
import zarr
from zarr.codecs import BytesCodec, GzipCodec, ShardingCodec, TransposeCodec, ZstdCodec

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "zarr")


def expected_path(name):
    return os.path.join(OUT, name + ".expected.raw")


def write_expected(name, values):
    values = np.ascontiguousarray(values)
    with open(expected_path(name), "wb") as f:
        f.write(values.astype(values.dtype.newbyteorder("<")).tobytes())


def fresh(name):
    path = os.path.join(OUT, name)
    if os.path.exists(path):
        shutil.rmtree(path)
    return path


def v2_array(name, data, chunks, compressor, fill, dtype, partial=None):
    """Zarr v2 array. `partial` is a slice written instead of the whole array,
    which leaves the other chunks missing."""
    path = fresh(name)
    arr = zarr.create_array(
        store=path,
        shape=data.shape,
        chunks=chunks,
        dtype=dtype,
        zarr_format=2,
        compressors=compressor,
        fill_value=fill,
        order="C",
    )
    if partial is None:
        arr[...] = data
        expected = data
    else:
        arr[partial] = data[partial]
        expected = np.full(data.shape, fill, dtype=dtype)
        expected[partial] = data[partial]
    write_expected(name, expected)
    return path


def v3_array(name, data, chunks, codecs, fill, dtype, partial=None, shards=None):
    path = fresh(name)
    kwargs = {}
    if shards is not None:
        kwargs["shards"] = shards
    arr = zarr.create_array(
        store=path,
        shape=data.shape,
        chunks=chunks,
        dtype=dtype,
        zarr_format=3,
        filters=[c for c in codecs if isinstance(c, TransposeCodec)],
        serializer=next(c for c in codecs if isinstance(c, (BytesCodec, ShardingCodec))),
        compressors=[c for c in codecs if isinstance(c, (ZstdCodec, GzipCodec))],
        fill_value=fill,
        **kwargs,
    )
    if partial is None:
        arr[...] = data
        expected = data
    else:
        arr[partial] = data[partial]
        expected = np.full(data.shape, fill, dtype=dtype)
        expected[partial] = data[partial]
    write_expected(name, expected)
    return path


def main():
    if os.path.exists(OUT):
        shutil.rmtree(OUT)
    os.makedirs(OUT)
    rng = np.random.default_rng(12345)

    f64 = rng.normal(size=(12, 10)).astype("<f8")
    i16 = np.arange(60, dtype="<i2").reshape(6, 10) - 30
    f32 = (rng.normal(size=(9, 7)) * 100).astype("<f4")
    nan_f64 = f64.copy()
    nan_f64[0, 0] = np.nan

    # Zarr v2: each compressor the Rust side claims to support.
    v2_array("v2_none", f64, (5, 4), None, 0.0, "<f8")
    v2_array("v2_zlib", f64, (5, 4), numcodecs.Zlib(level=5), 0.0, "<f8")
    v2_array("v2_gzip", f64, (4, 4), numcodecs.GZip(level=1), 0.0, "<f8")
    v2_array("v2_lz4", f64, (5, 4), numcodecs.LZ4(acceleration=1), 0.0, "<f8")
    v2_array("v2_zstd", f64, (5, 5), numcodecs.Zstd(level=3), 0.0, "<f8")
    v2_array("v2_blosc_lz4_shuffle", f64, (5, 4), numcodecs.Blosc("lz4", clevel=5, shuffle=numcodecs.Blosc.SHUFFLE), 0.0, "<f8")
    v2_array("v2_blosc_zstd_shuffle", f64, (5, 4), numcodecs.Blosc("zstd", clevel=3, shuffle=numcodecs.Blosc.SHUFFLE), 0.0, "<f8")
    v2_array("v2_blosc_zlib_bitshuffle", i16, (3, 5), numcodecs.Blosc("zlib", clevel=5, shuffle=numcodecs.Blosc.BITSHUFFLE), 0, "<i2")
    v2_array("v2_blosc_blosclz_shuffle", f64, (5, 4), numcodecs.Blosc("blosclz", clevel=5, shuffle=numcodecs.Blosc.SHUFFLE), 0.0, "<f8")
    v2_array("v2_float32_nan_fill_missing", f32, (4, 3), numcodecs.Zlib(level=1), float("nan"), "<f4", partial=(slice(0, 4), slice(0, 4)))
    v2_array("v2_int16_partial", i16, (2, 4), None, -7, "<i2", partial=(slice(2, 6), slice(0, 10)))
    v2_array("v2_f64_nan_data", nan_f64, (4, 4), numcodecs.Zlib(level=1), 0.0, "<f8")

    # Large chunks: Blosc splits each block into one stream per element byte
    # (typesize 8, 4096 elements), so the multi-stream path is exercised. The
    # array is not a multiple of the chunk shape, so edge chunks are padded.
    big = rng.normal(size=(100, 90)).astype("<f8")
    for cname, shuffle in [("lz4", numcodecs.Blosc.SHUFFLE), ("zstd", numcodecs.Blosc.SHUFFLE), ("zlib", numcodecs.Blosc.NOSHUFFLE), ("blosclz", numcodecs.Blosc.BITSHUFFLE)]:
        v2_array(
            "v2_blosc_" + cname + "_split_big",
            big,
            (64, 64),
            numcodecs.Blosc(cname, clevel=5, shuffle=shuffle),
            0.0,
            "<f8",
        )

    # Zarr v3: bytes with compression, transpose, and sharding (index at the end,
    # with and without a checksum, as zarr-python writes it).
    v3_array("v3_bytes_zstd", f64, (5, 4), [BytesCodec(), ZstdCodec(level=3)], 0.0, "float64")
    v3_array("v3_bytes_gzip", f64, (4, 4), [BytesCodec(), GzipCodec(level=2)], 0.0, "float64")
    v3_array("v3_transpose_bytes", f64, (4, 5), [TransposeCodec(order=(1, 0)), BytesCodec()], 0.0, "float64")
    v3_array("v3_partial_missing", i16, (2, 5), [BytesCodec()], -3, "int16", partial=(slice(0, 4), slice(5, 10)))
    v3_array(
        "v3_sharded_f64",
        rng.normal(size=(8, 8)).astype("<f8"),
        (8, 8),
        [ShardingCodec(chunk_shape=(2, 4), codecs=[BytesCodec(), GzipCodec(level=1)])],
        0.0,
        "float64",
    )

    # A small index of what was generated, so the Rust test can check coverage.
    manifest = sorted(
        d for d in os.listdir(OUT) if os.path.isdir(os.path.join(OUT, d))
    )
    with open(os.path.join(OUT, "manifest.json"), "w") as f:
        json.dump({"zarr_python": zarr.__version__, "numcodecs": numcodecs.__version__, "arrays": manifest}, f, indent=1)
    print("generated", len(manifest), "arrays")


if __name__ == "__main__":
    main()
