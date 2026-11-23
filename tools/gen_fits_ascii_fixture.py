#!/usr/bin/env python3
"""Write the FITS ASCII-table fixture and its expected values.

tpt-planetary-astro/tests/fixtures/fits/ascii.fits is an ASCII table
(XTENSION = 'TABLE') with a text column, an integer column with a TNULL value,
a fixed-point F column with a TSCAL/TZERO scaling, an E column and a D column
(Fortran D exponents). The expected physical values are what astropy.io.fits
returns, with undefined values as 'nan', written to expected.tsv beside it: one
line per column, the name, a tab, and the values separated by spaces. Text
values are written as they are, with an empty string as '_'.

Usage: python tools/gen_fits_ascii_fixture.py   (requires astropy and numpy)
"""

from pathlib import Path

import numpy as np
from astropy.io import fits

OUT = Path(__file__).resolve().parent.parent / "tpt-planetary-astro" / "tests" / "fixtures" / "fits"


def build() -> fits.TableHDU:
    cols = [
        fits.Column(name="label", format="A6", array=np.array(["alpha", "b", "", "delta"])),
        fits.Column(name="count", format="I6", array=np.array([-3, 0, -999, 12]), null=-999),
        fits.Column(name="fixed", format="F9.3", array=np.array([0.5, -1.25, 3.0, 1000.125])),
        fits.Column(name="expo", format="E12.4", array=np.array([1.5e-3, -2.0, 3.25e5, 0.0])),
        fits.Column(name="dbl", format="D15.7", array=np.array([1.0e-10, 2.5, -7.0, 123.456])),
    ]
    hdu = fits.TableHDU.from_columns(cols)
    # Scaling on the F column: physical = 10 + 0.5 * stored.
    hdu.columns["fixed"].bscale = 0.5
    hdu.columns["fixed"].bzero = 10.0
    return hdu


def values(hdu: fits.TableHDU) -> dict[str, list]:
    out = {}
    for name in hdu.columns.names:
        col = hdu.data[name]
        if col.dtype.kind in "SU":
            out[name] = [s.strip() if s.strip() else "_" for s in col]
        else:
            # Undefined (TNULL) values are masked by astropy; they become NaN.
            values = np.ma.filled(np.ma.asarray(col, dtype=np.float64), np.nan)
            out[name] = [float(v) for v in values]
    return out


def fmt(v) -> str:
    if isinstance(v, str):
        return v
    return "nan" if np.isnan(v) else repr(float(v))


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    primary = fits.PrimaryHDU()
    hdu = build()
    fits.HDUList([primary, hdu]).writeto(OUT / "ascii.fits", overwrite=True)
    # Read back through astropy, so the expected values are what a reader sees.
    back = fits.open(OUT / "ascii.fits")[1]
    lines = []
    for name, vals in values(back).items():
        if name == "count":
            # astropy reads the TNULL field of this ASCII integer column as 0
            # and does not mask it, so the undefined row comes from the input.
            # The stored text is -999, and TNULL2 is -999.
            undefined = np.array([-3, 0, -999, 12]) == -999
            vals = [float("nan") if u else v for v, u in zip(vals, undefined)]
        lines.append(name + "\t" + " ".join(fmt(v) for v in vals))
    (OUT / "ascii_expected.tsv").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {OUT / 'ascii.fits'} and ascii_expected.tsv")


if __name__ == "__main__":
    main()
