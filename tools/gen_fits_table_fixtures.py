#!/usr/bin/env python3
"""Write the FITS binary-table fixture and its expected values.

tpt-planetary-astro/tests/fixtures/fits/table.fits is a BINTABLE with columns
of each fixed-width type, repeat counts, TSCAL/TZERO scaling (including the
unsigned-16-bit convention), a TNULL integer column, and a logical column. The
expected physical values are what astropy.io.fits returns (with scaling
applied; undefined values as 'nan'), written to expected.tsv beside it: one
line per column, the name, a tab, and the values separated by spaces. Labels
are written as they are, with an empty label as '_'.

Usage: python tools/gen_fits_table_fixtures.py   (requires astropy and numpy)
"""

from pathlib import Path

import numpy as np
from astropy.io import fits

OUT = Path(__file__).resolve().parent.parent / "tpt-planetary-astro" / "tests" / "fixtures" / "fits"

NUMERIC = ["id", "flux", "gain", "small", "byte", "big", "ok", "count", "scaled", "vec"]


def build() -> fits.BinTableHDU:
    rows = 5
    cols = [
        fits.Column(name="id", format="J", array=np.arange(rows, dtype=np.int32) * 7 - 10),
        fits.Column(name="flux", format="D", array=np.linspace(-1.5, 2.5, rows)),
        fits.Column(name="gain", format="E", array=np.arange(rows, dtype=np.float32) * 0.25),
        fits.Column(name="small", format="I", array=np.array([-3, 0, 5, 100, -200], dtype=np.int16)),
        fits.Column(name="byte", format="B", array=np.array([0, 9, 255, 17, 42], dtype=np.uint8)),
        fits.Column(name="big", format="K", array=np.array([2**40, -(2**40), 0, 12, 99], dtype=np.int64)),
        fits.Column(name="vec", format="3E", array=np.arange(rows * 3, dtype=np.float32).reshape(rows, 3)),
        fits.Column(name="ok", format="L", array=np.array([True, False, True, True, False])),
        fits.Column(name="label", format="8A", array=np.array(["alpha", "b", "", "delta-x", "e"])),
        # Stored as signed 16-bit; TZERO10 (set below) makes them unsigned.
        fits.Column(name="count", format="I", array=np.array([-32768, -1, 0, 1, 32767], dtype=np.int16)),
        # Stored integers; TSCAL11 and TZERO11 (set below) give physical = 0.5 * raw + 10.
        fits.Column(name="scaled", format="J", array=np.array([2, 4, 6, 8, 10], dtype=np.int32)),
    ]
    hdu = fits.BinTableHDU.from_columns(cols, name="SAMPLES")
    # TNULL on 'small': the stored value -200 is undefined.
    hdu.header["TNULL4"] = -200
    hdu.header["TZERO10"] = 32768.0
    hdu.header["TSCAL11"] = 0.5
    hdu.header["TZERO11"] = 10.0
    return hdu


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    fits.HDUList([fits.PrimaryHDU(), build()]).writeto(OUT / "table.fits", overwrite=True)

    # Expected physical values, as astropy reads them back (uint=False keeps
    # the TZERO scaling of the unsigned column without astropy's uint cast).
    expected: dict[str, list] = {}
    with fits.open(OUT / "table.fits", uint=False) as f:
        t = f["SAMPLES"].data
        for name in NUMERIC:
            expected[name] = np.asarray(t[name], dtype=np.float64).ravel()
        expected["label"] = [str(s).rstrip() for s in t["label"]]
        # TNULL: astropy returns the stored -200 unchanged; it is undefined here.
        expected["small"] = np.where(expected["small"] == -200, np.nan, expected["small"])

    lines = []
    for name in NUMERIC:
        items = ["nan" if np.isnan(v) else repr(float(v)) for v in expected[name]]
        lines.append(name + "\t" + " ".join(items))
    labels = [v if v else "_" for v in expected["label"]]
    lines.append("label\t" + " ".join(labels))
    (OUT / "expected.tsv").write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"wrote {OUT / 'table.fits'} and expected.tsv")


if __name__ == "__main__":
    main()
