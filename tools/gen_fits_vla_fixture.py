#!/usr/bin/env python3
"""Write the FITS variable-length column fixture and its expected values.

tpt-planetary-astro/tests/fixtures/fits/vla.fits is a BINTABLE with a P column
of int32 arrays (`PJ`), a Q column of float64 arrays (`QD`, 64-bit descriptors)
and a P column of bytes (`PB`). Rows have different lengths, and one row is
empty. The expected values are what astropy.io.fits returns, written to
vla_expected.tsv: one line per column, the name and a tab, then the rows
separated by '|' and the elements of a row by spaces ('|' alone is an empty row).

Usage: python tools/gen_fits_vla_fixture.py   (requires astropy and numpy)
"""

from pathlib import Path

import numpy as np
from astropy.io import fits

OUT = Path(__file__).resolve().parent.parent / "tpt-planetary-astro" / "tests" / "fixtures" / "fits"


def build() -> fits.BinTableHDU:
    ints = np.empty(4, dtype=object)
    ints[:] = [np.array([1, -2, 3], dtype=np.int32), np.array([], dtype=np.int32),
               np.array([40], dtype=np.int32), np.array([-5, 6], dtype=np.int32)]
    dbls = np.empty(4, dtype=object)
    dbls[:] = [np.array([0.5]), np.array([1.5, -2.25, 1.0e-3]), np.array([]),
               np.array([7.0, 8.0])]
    bytes_ = np.empty(4, dtype=object)
    bytes_[:] = [np.array([255, 0], dtype=np.uint8), np.array([9], dtype=np.uint8),
                 np.array([1, 2, 3, 4], dtype=np.uint8), np.array([], dtype=np.uint8)]
    cols = [
        fits.Column(name="ints", format="PJ()", array=ints),
        fits.Column(name="dbls", format="QD()", array=dbls),
        fits.Column(name="bytes", format="PB()", array=bytes_),
    ]
    return fits.BinTableHDU.from_columns(cols)


def line(name: str, rows) -> str:
    text = []
    for row in rows:
        text.append(" ".join(repr(float(v)) if isinstance(v, float) else str(int(v)) for v in row))
    return name + "\t" + "|".join(text)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    fits.HDUList([fits.PrimaryHDU(), build()]).writeto(OUT / "vla.fits", overwrite=True)
    back = fits.open(OUT / "vla.fits")[1]
    lines = [line(name, back.data[name]) for name in ("ints", "dbls", "bytes")]
    (OUT / "vla_expected.tsv").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {OUT / 'vla.fits'} and vla_expected.tsv")


if __name__ == "__main__":
    main()
