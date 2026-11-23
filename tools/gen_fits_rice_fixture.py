#!/usr/bin/env python3
"""Write the FITS tile-compressed image fixtures and their expected values.

tpt-planetary-astro/tests/fixtures/fits/{rice,gzip1,gzip2}{16,32}.fits are compressed
image extensions (BINTABLE with ZIMAGE = T) written by astropy.io.fits
(CompImageHDU, which uses CFITSIO's tile compression): RICE_1, GZIP_1 (deflate of
the big-endian pixel bytes) and GZIP_2 (deflate of byte-shuffled pixels), each
for a 16-bit and a 32-bit signed image. Tiles are 8 rows by 16 columns, so edge
tiles are partial. Each image has a smooth background and a noisy band, so the
Rice coder sees both low- and high-entropy blocks.

The expected pixels are what astropy returns when it decompresses each
extension, written to rice_expected.tsv: one line per image, the name, the
shape, and the pixel values in FITS order (row-major by the first axis, fastest),
separated by spaces.

Usage: python tools/gen_fits_rice_fixture.py   (requires astropy and numpy)
"""

from pathlib import Path

import numpy as np
from astropy.io import fits

OUT = Path(__file__).resolve().parent.parent / "tpt-planetary-astro" / "tests" / "fixtures" / "fits"
SHAPE = (23, 37)  # NAXIS2 = 23 rows, NAXIS1 = 37 columns
TILE = (16, 8)    # ZTILE1 = 16 columns, ZTILE2 = 8 rows: partial tiles at both edges

CASES = {
    "rice16": (np.int16, "RICE_1", 16),
    "rice32": (np.int32, "RICE_1", 32),
    "gzip1_16": (np.int16, "GZIP_1", 16),
    "gzip1_32": (np.int32, "GZIP_1", 32),
    "gzip2_16": (np.int16, "GZIP_2", 16),
    "gzip2_32": (np.int32, "GZIP_2", 32),
}


def image(seed: int, dtype) -> np.ndarray:
    info = np.iinfo(dtype)
    low, high = max(info.min, -2_000_000_000), min(info.max, 2_000_000_000)
    if dtype == np.int16:
        low, high = -30000, 30000
    rng = np.random.default_rng(seed)
    rows, cols = SHAPE
    y, x = np.mgrid[0:rows, 0:cols]
    base = 40.0 * np.sin(x / 7.0) + 25.0 * np.cos(y / 5.0)
    img = np.round(base + rng.normal(0.0, 1.0, SHAPE)).astype(np.int64)
    band = slice(8, 15)
    img[band, :] = rng.integers(low, high, size=(band.stop - band.start, cols))
    return img.astype(dtype)


def write(path: Path, data: np.ndarray, compression: str) -> None:
    hdu = fits.CompImageHDU(
        data=data, compression_type=compression, tile_shape=(TILE[1], TILE[0])
    )
    fits.HDUList([fits.PrimaryHDU(), hdu]).writeto(path, overwrite=True)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    lines = []
    for index, (name, (dtype, compression, bits)) in enumerate(CASES.items()):
        data = image(100 + index, dtype)
        path = OUT / f"{name}.fits"
        write(path, data, compression)
        back = np.asarray(fits.open(path)[1].data)
        assert np.array_equal(back, data), f"{name}: astropy round trip"
        values = " ".join(str(int(v)) for v in back.ravel())
        lines.append(f"{name}\t{back.shape[0]}x{back.shape[1]}\t{values}")
    (OUT / "rice_expected.tsv").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {len(CASES)} compressed fixtures and rice_expected.tsv")


if __name__ == "__main__":
    main()
