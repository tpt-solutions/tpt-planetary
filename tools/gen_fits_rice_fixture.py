#!/usr/bin/env python3
"""Write the FITS tile-compressed (RICE_1) image fixtures and their expected values.

tpt-planetary-astro/tests/fixtures/fits/rice16.fits and rice32.fits are compressed
image extensions (BINTABLE with ZIMAGE = T) written by astropy.io.fits
(CompImageHDU, which uses CFITSIO's RICE_1 algorithm, block size 32). rice16
is a 16-bit signed image whose rows are smooth with a few noisy tiles; rice32
is a 32-bit signed image, also with a noisy band, so both low-entropy and
high-entropy blocks occur. The expected pixels are what astropy returns when it
decompresses the extension, written to rice_expected.tsv: one line per image,
the name, the shape, and the pixel values in FITS order (row-major by the
first axis, fastest), separated by spaces.

Usage: python tools/gen_fits_rice_fixture.py   (requires astropy and numpy)
"""

from pathlib import Path

import numpy as np
from astropy.io import fits

OUT = Path(__file__).resolve().parent.parent / "tpt-planetary-astro" / "tests" / "fixtures" / "fits"
SHAPE = (23, 37)  # NAXIS2 = 23 rows, NAXIS1 = 37 columns
TILE = (16, 8)    # ZTILE1 = 16 columns, ZTILE2 = 8 rows: partial tiles at both edges


def smooth_with_noise(seed: int, dtype, low: int, high: int) -> np.ndarray:
    rng = np.random.default_rng(seed)
    rows, cols = SHAPE
    y, x = np.mgrid[0:rows, 0:cols]
    base = 40.0 * np.sin(x / 7.0) + 25.0 * np.cos(y / 5.0)
    img = np.round(base + rng.normal(0.0, 1.0, SHAPE)).astype(np.int64)
    # A noisy band of tiles: wide random values, so the Rice blocks are high-entropy.
    band = slice(8, 15)
    img[band, :] = rng.integers(low, high, size=(band.stop - band.start, cols))
    return np.clip(img, np.iinfo(dtype).min, np.iinfo(dtype).max).astype(dtype)


def write(path: Path, data: np.ndarray) -> None:
    hdu = fits.CompImageHDU(data=data, compression_type="RICE_1", tile_shape=(TILE[1], TILE[0]))
    fits.HDUList([fits.PrimaryHDU(), hdu]).writeto(path, overwrite=True)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    images = {
        "rice16": smooth_with_noise(16, np.int16, -30000, 30000),
        "rice32": smooth_with_noise(32, np.int32, -2_000_000_000, 2_000_000_000),
    }
    lines = []
    for name, data in images.items():
        write(OUT / f"{name}.fits", data)
        back = fits.open(OUT / f"{name}.fits")[1].data
        assert np.array_equal(np.asarray(back), data), f"{name}: astropy round trip"
        values = " ".join(str(int(v)) for v in np.asarray(back).ravel())
        shape = f"{back.shape[0]}x{back.shape[1]}"
        lines.append(f"{name}\t{shape}\t{values}")
    (OUT / "rice_expected.tsv").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("wrote rice16.fits, rice32.fits and rice_expected.tsv")


if __name__ == "__main__":
    main()
