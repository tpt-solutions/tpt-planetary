#!/usr/bin/env python3
"""Generate ``tpt-planetary-astro/src/nutation_data.rs`` from the IERS tables.

Parses the published IERS Conventions (2010) Chapter 5 tables:

- Table 5.3a (``tab5.3a.txt``): nutation in longitude, IAU 2000A_R06.
- Table 5.3b (``tab5.3b.txt``): nutation in obliquity, IAU 2000A_R06.

Each file has a j=0 block (amplitudes of the constant term) and a j=1 block
(amplitudes of the term linear in t). Every row is one term: an index, two
amplitudes (microarcseconds), and 14 integer multipliers of the fundamental
arguments F1..F14 (l, l', F, D, Om, L_Me, L_Ve, L_E, L_Ma, L_J, L_Sa, L_U,
L_Ne, p_A).

Column roles (chapter Eq. 5.35, and the column headers of both files):

- Longitude: the first amplitude column multiplies sin(ARG) and the second
  multiplies cos(ARG). Headers are ``A_i  A"_i`` (j=0) and ``A'_i  A"'_i`` (j=1).
- Obliquity: headers are ``B"_i  B_i`` (j=0) and ``B"'_i  B'_i`` (j=1), so the
  FIRST column is the sin coefficient and the SECOND is the cos coefficient.

Both files therefore store (sin, cos) in that order. Row indices run
continuously from 1 across the j=0 and j=1 blocks.

The generator fails loudly if an input's sha256 differs from the value pinned
below, if a row is malformed, if the row indices are not 1..N in order, or if a
block's row count differs from its ``Number of terms`` header.

Usage (run from the ``tpt-planetary-astro`` directory or the repository root)::

    python tools/gen_nutation_table.py <tab5.3a.txt> <tab5.3b.txt> [--out FILE]

The default output is ``src/nutation_data.rs`` next to this script's crate.
"""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
from pathlib import Path

# Pinned input checksums (sha256) of the IERS Conventions 2010 Chapter 5 files.
EXPECTED_SHA256 = {
    "tab5.3a.txt": "f333edfa5bf8296efbb383cc1425c60b9b5925388e750c588e58dc6ac8a0ece5",
    "tab5.3b.txt": "3ab45772c58922cc54c5a3424aebc4bb0caaf18d103ff75f939010e5b4b751f1",
}

N_ARGS = 14
HEADER_RE = re.compile(r"^j\s*=\s*([01])\s+Number\s+of\s+terms\s*=\s*(\d+)\s*$")
ROW_RE = re.compile(r"^\s*(\d+)\s+(\S+)\s+(\S+)((?:\s+-?\d+){%d})\s*$" % N_ARGS)
HEADING_RE = re.compile(r"^\s*i\s")
INT_RE = re.compile(r"^-?\d+$")
FLOAT_RE = re.compile(r"^-?\d+(\.\d+)?$")

Row = tuple[list[int], str, str]  # (multipliers, sin_text, cos_text)


class GenerationError(Exception):
    """Raised for any parse, count or checksum mismatch."""


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 16), b""):
            h.update(chunk)
    return h.hexdigest()


def parse_table(path: Path) -> dict[int, list[Row]]:
    """Parse both blocks of one table into ``{j: rows}``.

    Verifies the row count of each block against its header and the
    continuity of the row indices across blocks.
    """
    lines = path.read_text(encoding="ascii").splitlines()
    blocks: dict[int, list[Row]] = {}
    declared: dict[int, int] = {}
    current_j: int | None = None
    next_index = 1  # indices run continuously across j=0 and j=1

    for lineno, raw in enumerate(lines, start=1):
        line = raw.strip()
        header = HEADER_RE.match(line)
        if header:
            current_j = int(header.group(1))
            if current_j in blocks:
                raise GenerationError(f"{path.name}: duplicate block j={current_j}")
            declared[current_j] = int(header.group(2))
            blocks[current_j] = []
            continue
        if current_j is None or not line or line.startswith(("-", "Table")):
            continue
        if HEADING_RE.match(raw):
            continue
        m = ROW_RE.match(raw)
        if not m:
            if blocks[current_j]:
                raise GenerationError(f"{path.name}:{lineno}: unparsable row: {raw!r}")
            continue  # text between the header and the first row
        index = int(m.group(1))
        if index != next_index:
            raise GenerationError(f"{path.name}:{lineno}: row index {index}, expected {next_index}")
        next_index += 1
        amp1, amp2 = m.group(2), m.group(3)
        for tok in (amp1, amp2):
            if not FLOAT_RE.match(tok):
                raise GenerationError(f"{path.name}:{lineno}: bad amplitude {tok!r}")
        mults_text = m.group(4).split()
        if len(mults_text) != N_ARGS or not all(INT_RE.match(t) for t in mults_text):
            raise GenerationError(f"{path.name}:{lineno}: bad multipliers {mults_text!r}")
        blocks[current_j].append(([int(t) for t in mults_text], amp1, amp2))

    for j in (0, 1):
        if j not in blocks:
            raise GenerationError(f"{path.name}: block j={j} not found")
        if len(blocks[j]) != declared[j]:
            raise GenerationError(
                f"{path.name}: j={j} parsed {len(blocks[j])} rows, header says {declared[j]}"
            )
    return blocks


def emit_array(name: str, doc: str, rows: list[Row]) -> str:
    body = "\n".join(
        f"    Term {{ args: [{', '.join(str(m) for m in mults)}], sin: {s}, cos: {c} }},"
        for mults, s, c in rows
    )
    return f"/// {doc}\n#[rustfmt::skip]\npub static {name}: [Term; {len(rows)}] = [\n{body}\n];\n"


def build(a_path: Path, b_path: Path, out_path: Path, command: str) -> dict[str, int]:
    for p in (a_path, b_path):
        digest = sha256_of(p)
        if digest != EXPECTED_SHA256.get(p.name):
            raise GenerationError(
                f"{p.name}: sha256 {digest} does not match the pinned value "
                f"{EXPECTED_SHA256.get(p.name)}"
            )
    lon = parse_table(a_path)
    obl = parse_table(b_path)
    counts = {
        "LONGITUDE_J0": len(lon[0]),
        "LONGITUDE_J1": len(lon[1]),
        "OBLIQUITY_J0": len(obl[0]),
        "OBLIQUITY_J1": len(obl[1]),
    }

    header = f"""//! Nutation series from IERS Conventions (2010), Chapter 5, Tables 5.3a and
//! 5.3b (IAU 2000A_R06 expression). GENERATED FILE: do not edit by hand.
//! Regenerate with the command below.
//!
//! Sources (sha256):
//! - tab5.3a.txt (Table 5.3a, nutation in longitude): {sha256_of(a_path)}
//! - tab5.3b.txt (Table 5.3b, nutation in obliquity): {sha256_of(b_path)}
//!
//! Generator: tpt-planetary-astro/tools/gen_nutation_table.py
//! Command:   {command}
//!
//! Row counts, verified against the "Number of terms" headers:
//! - longitude: j=0 {counts["LONGITUDE_J0"]}, j=1 {counts["LONGITUDE_J1"]}
//! - obliquity: j=0 {counts["OBLIQUITY_J0"]}, j=1 {counts["OBLIQUITY_J1"]}
//!
//! Units: microarcseconds. In both arrays `sin` and `cos` are the coefficients
//! of sin(ARG) and cos(ARG) (for j=0) or of t sin(ARG) and t cos(ARG) (for
//! j=1), with t in Julian centuries of TT from J2000. `args` multiplies
//! F1..F14 in this order: l, l', F, D, Om, L_Me, L_Ve, L_E, L_Ma, L_J, L_Sa,
//! L_U, L_Ne, p_A.

// Published amplitudes such as 6.28 microarcseconds are data, not 2*pi.
#![allow(clippy::approx_constant)]

use crate::nutation::Term;

"""
    body = "\n".join(
        [
            emit_array("LONGITUDE_J0", "Table 5.3a, j=0: amplitudes of Delta psi.", lon[0]),
            emit_array("LONGITUDE_J1", "Table 5.3a, j=1: amplitudes of t in Delta psi.", lon[1]),
            emit_array("OBLIQUITY_J0", "Table 5.3b, j=0: amplitudes of Delta epsilon.", obl[0]),
            emit_array("OBLIQUITY_J1", "Table 5.3b, j=1: amplitudes of t in Delta epsilon.", obl[1]),
        ]
    )
    out_path.write_text(header + body, encoding="utf-8", newline="\n")
    return counts


def main(argv: list[str]) -> int:
    default_out = Path(__file__).resolve().parents[1] / "src" / "nutation_data.rs"
    ap = argparse.ArgumentParser(description="Generate the IAU 2000A_R06 nutation tables.")
    ap.add_argument("tab5_3a", type=Path, help="path to tab5.3a.txt")
    ap.add_argument("tab5_3b", type=Path, help="path to tab5.3b.txt")
    ap.add_argument("--out", type=Path, default=default_out, help="output .rs file")
    args = ap.parse_args(argv)

    command = (
        "python tpt-planetary-astro/tools/gen_nutation_table.py "
        f"{args.tab5_3a.name} {args.tab5_3b.name}"
    )
    try:
        counts = build(args.tab5_3a, args.tab5_3b, args.out, command)
    except GenerationError as err:
        print(f"gen_nutation_table: ERROR: {err}", file=sys.stderr)
        return 1
    print(f"wrote {args.out}")
    for name, n in counts.items():
        print(f"  {name}: {n} terms")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
