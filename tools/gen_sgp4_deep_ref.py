#!/usr/bin/env python3
"""Write tpt-planetary-astro/tests/fixtures/sgp4_deep_ref.tsv.

Uses the sgp4 Python package (MIT): the deep-space element sets of its
verification file (period of 225 minutes or more), propagated at fixed times by
the pure-Python model (sgp4.model.Satrec, which keeps no state between calls).
Each row: line 1, line 2, minutes since epoch, status ("ok" or "errN"), then
TEME position (km) and velocity (km/s) for "ok" rows.

The TLE checksum (column 69) is recomputed when the verification file has a
wrong one; the orbital elements are unchanged.

Usage: python tools/gen_sgp4_deep_ref.py
Requires: pip install sgp4
"""

import math
from pathlib import Path

from sgp4 import api
from sgp4.model import Satrec as PySatrec

TIMES = [0.0, 30.0, 360.0, 720.0, 1440.0, -720.0, 2880.0, 4320.0, 10080.0, -1440.0]
OUT = Path(__file__).resolve().parent.parent / "tpt-planetary-astro" / "tests" / "fixtures" / "sgp4_deep_ref.tsv"


def checksum(line: str) -> int:
    total = 0
    for ch in line[:68]:
        if ch.isdigit():
            total += int(ch)
        elif ch == "-":
            total += 1
    return total % 10


def main() -> None:
    src = Path(api.__file__).with_name("SGP4-VER.TLE").read_text(encoding="utf-8").splitlines()
    pairs = [(src[i], src[i + 1]) for i in range(len(src) - 1)
             if src[i].startswith("1 ") and src[i + 1].startswith("2 ")]
    rows = [
        "# Reference SGP4/SDP4 states from the sgp4 Python package (MIT), pure-Python",
        "# propagation (model.Satrec). Columns: line 1, line 2, minutes since epoch,",
        "# status (ok, or the SGP4 error code), then TEME position (km) and velocity (km/s).",
        "# Column 69 (the TLE checksum) is recomputed where the source line had a wrong one;",
        "# the orbital elements are unchanged.",
    ]
    for l1, l2 in pairs:
        probe = api.Satrec.twoline2rv(l1, l2)
        if 2 * math.pi / probe.no_kozai < 225.0:
            continue
        fixed = []
        for line in (l1, l2):
            c = str(checksum(line))
            fixed.append(line[:68] + c + line[69:])
        l1, l2 = fixed
        jd0, fr0 = probe.jdsatepoch, probe.jdsatepochF
        for t in TIMES:
            sat = PySatrec.twoline2rv(l1, l2)
            err, r, v = sat.sgp4(jd0, fr0 + t / 1440.0)
            if err != 0:
                rows.append("\t".join([l1, l2, repr(t), f"err{err}"]))
            else:
                vals = [repr(float(x)) for x in list(r) + list(v)]
                rows.append("\t".join([l1, l2, repr(t), "ok"] + vals))
    OUT.write_text("\n".join(rows) + "\n", encoding="utf-8", newline="\n")
    print(f"wrote {OUT} ({len(rows) - 5} rows)")


if __name__ == "__main__":
    main()
