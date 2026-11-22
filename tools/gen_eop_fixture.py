#!/usr/bin/env python3
"""Write an excerpt of the IERS finals2000A file and the values to check against.

The excerpt is the rows of a finals2000A.all file whose MJD lies in a few short
windows (so the Rust reader sees real rows, with the flags and blanks that IERS
uses). The expected values are UT1-UTC and polar motion from astropy's bundled
IERS-B table, at epochs midway between two daily rows; they are compared with
the linear interpolation of the excerpt rows.

Usage: python tools/gen_eop_fixture.py path/to/finals2000A.all
(Get the full file from https://datacenter.iers.org/data/9/finals2000A.all.)
Requires astropy.
"""

import sys
from pathlib import Path

from astropy.time import Time
from astropy.utils import iers

OUT = Path(__file__).resolve().parent.parent / "tpt-planetary-astro" / "tests" / "fixtures" / "eop"
WINDOWS = [(58000, 58003), (58500, 58502), (59000, 59002), (59499, 59502)]
EPOCHS = [58000.25, 58000.5, 58500.5, 59000.75, 59500.1]


def main() -> None:
    source = Path(sys.argv[1])
    lines = source.read_text(encoding="ascii").splitlines()
    keep = []
    for line in lines:
        if len(line) < 68 or line[57] not in "IP" or not line[58:68].strip():
            continue
        mjd = float(line[7:15])
        if any(lo <= mjd <= hi for lo, hi in WINDOWS):
            keep.append(line)
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "finals_excerpt.all").write_text("\n".join(keep) + "\n", encoding="ascii", newline="\n")

    iers.conf.auto_download = False
    table = iers.IERS_B.open()
    rows = ["mjd\tdut1_s\txp_arcsec\typ_arcsec"]
    for mjd in EPOCHS:
        t = Time(mjd, format="mjd", scale="utc")
        dut1 = float(table.ut1_utc(t.jd1, t.jd2).to("s").value)
        xp, yp = (float(q.to("arcsec").value) for q in table.pm_xy(t.jd1, t.jd2))
        rows.append(f"{mjd!r}\t{dut1!r}\t{xp!r}\t{yp!r}")
    (OUT / "expected.tsv").write_text("\n".join(rows) + "\n", encoding="utf-8", newline="\n")
    print(f"wrote {len(keep)} finals rows and {len(EPOCHS)} expected epochs to {OUT}")


if __name__ == "__main__":
    main()
