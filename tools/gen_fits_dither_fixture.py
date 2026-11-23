#!/usr/bin/env python3
"""Write the FITS dithered-quantization fixtures and their expected values.

tpt-planetary-astro/tests/fixtures/fits/dither1.fits and dither2.fits are
tile-compressed float images (RICE_1, 8 by 16 tiles) quantized with CFITSIO's
SUBTRACTIVE_DITHER_1 and SUBTRACTIVE_DITHER_2, with a fixed dither seed. The
image has a smooth background, a few exact zeros (preserved by dither 2) and a
NaN. The expected pixels are astropy's decompression, written to
dither_expected.tsv: one line per image, the name, the shape, and the values in
FITS order, with 'nan' for undefined pixels.

Usage: python tools/gen_fits_dither_fixture.py   (requires astropy and numpy)
"""

from pathlib import Path

import numpy as np
from astropy.io import fits

OUT = Path(__file__).resolve().parent.parent / "tpt-planetary-astro" / "tests" / "fixtures" / "fits"
SHAPE = (23, 37)
TILE = (8, 16)  # numpy order: rows, columns
SEED = 7        # ZDITHER0


def image(seed: int) -> np.ndarray:
    rng = np.random.default_rng(seed)
    rows, cols = SHAPE
    y, x = np.mgrid[0:rows, 0:cols]
    img = (0.3 * np.sin(x / 6.0) + 0.2 * np.cos(y / 4.0) + rng.normal(0.0, 0.01, SHAPE))
    img = img.astype(np.float32)
    img[img > 0.45] = 0.0  # exact zeros, which dither 2 keeps exact
    img[3, 4] = np.nan
    return img


def write(path: Path, data: np.ndarray, method: int) -> None:
    hdu = fits.CompImageHDU(
        data=data,
        compression_type="RICE_1",
        tile_shape=TILE,
        quantize_method=method,
        quantize_level=16.0,
        dither_seed=SEED,
    )
    fits.HDUList([fits.PrimaryHDU(), hdu]).writeto(path, overwrite=True)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    data = image(300)
    lines = []
    for name, method in [("dither1", 1), ("dither2", 2)]:
        path = OUT / f"{name}.fits"
        write(path, data, method)
        back = np.asarray(fits.open(path)[1].data)
        values = " ".join("nan" if np.isnan(v) else repr(float(v)) for v in back.ravel())
        lines.append(f"{name}\t{back.shape[0]}x{back.shape[1]}\t{values}")
    (OUT / "dither_expected.tsv").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("wrote dither1.fits, dither2.fits and dither_expected.tsv")


if __name__ == "__main__":
    main()
