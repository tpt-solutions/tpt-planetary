#!/usr/bin/env python3
"""Write the float16 rounding fixture: doubles and their IEEE binary16 bit patterns
as numpy rounds them (round to nearest, ties to even).

tpt-planetary-array/tests/fixtures/float16_rounding.tsv has one line per value:
the double (repr, full precision) and the 16-bit pattern (decimal).

Usage: python tools/gen_float16_fixture.py   (requires numpy)
"""

from pathlib import Path

import numpy as np

OUT = Path(__file__).resolve().parent.parent / "tpt-planetary-array" / "tests" / "fixtures" / "float16_rounding.tsv"


def main() -> None:
    rng = np.random.default_rng(16)
    # Spread over the binary16 range: normals, subnormals, overflow and underflow.
    exps = rng.uniform(-30.0, 17.0, 4000)
    signs = rng.choice([-1.0, 1.0], 4000)
    values = signs * (2.0 ** exps) * rng.uniform(1.0, 2.0, 4000)
    values = np.concatenate([values, [0.0, -0.0, 65504.0, 65519.0, 65520.0, 2.0**-24, 2.0**-25, 0.1]])
    lines = []
    for v in values:
        bits = int(np.array(v, dtype=np.float64).astype(np.float16).view(np.uint16))
        lines.append(f"{float(v)!r}\t{bits}")
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {len(lines)} values to {OUT}")


if __name__ == "__main__":
    main()
