#!/usr/bin/env python3
"""Write the FITS bit and complex column fixture and its expected values.

tpt-planetary-astro/tests/fixtures/fits/bitcomplex.fits is a BINTABLE with an
X column (bits, 12 per row, packed as the standard does), a C column
(single-precision complex, 2 per row) and an M column (double-precision complex,
1 per row). The expected values are what astropy.io.fits returns, written to
bitcomplex_expected.tsv beside it: one line per column. Bits are written as 0 or
1, and complex values as 're,im' pairs, separated by spaces, row by row.

Usage: python tools/gen_fits_bit_complex_fixture.py   (requires astropy and numpy)
"""

from pathlib import Path

import numpy as np
from astropy.io import fits

OUT = Path(__file__).resolve().parent.parent / "tpt-planetary-astro" / "tests" / "fixtures" / "fits"


def build() -> fits.BinTableHDU:
    rows = 3
    bits = np.zeros((rows, 12), dtype=bool)
    for r in range(rows):
        for k in range(12):
            bits[r, k] = (r * 5 + k * 3) % 4 == 1
    cplx = (np.arange(rows * 2, dtype=np.float32).reshape(rows, 2) * 0.5
            + 1j * (np.arange(rows * 2, dtype=np.float32).reshape(rows, 2) - 2.0))
    mcplx = np.array([1.0e-7 - 2.5j, -3.0 + 1.0e6j, 0.0 + 0.0j])
    cols = [
        fits.Column(name="flags", format="12X", array=bits),
        fits.Column(name="pair", format="2C", array=cplx.astype(np.complex64)),
        fits.Column(name="z", format="M", array=mcplx.astype(np.complex128)),
    ]
    return fits.BinTableHDU.from_columns(cols)


def fmt_complex(z: complex) -> str:
    return f"{float(z.real)!r},{float(z.imag)!r}"


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    hdu = build()
    fits.HDUList([fits.PrimaryHDU(), hdu]).writeto(OUT / "bitcomplex.fits", overwrite=True)
    back = fits.open(OUT / "bitcomplex.fits")[1]
    lines = []
    flags = np.asarray(back.data["flags"]).astype(int).ravel()
    lines.append("flags\t" + " ".join(str(int(v)) for v in flags))
    pair = np.asarray(back.data["pair"]).ravel()
    lines.append("pair\t" + " ".join(fmt_complex(z) for z in pair))
    z = np.asarray(back.data["z"]).ravel()
    lines.append("z\t" + " ".join(fmt_complex(v) for v in z))
    (OUT / "bitcomplex_expected.tsv").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {OUT / 'bitcomplex.fits'} and bitcomplex_expected.tsv")


if __name__ == "__main__":
    main()
