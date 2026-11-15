#!/usr/bin/env python3
"""Generate ``tpt-planetary-astro/src/cio_data.rs`` from the IERS tables.

Parses the published IERS Conventions (2010) Chapter 5 tables used for the
CIP coordinates X and Y, the CIO locator s, and the complementary terms of the
equation of the origins:

- Table 5.2a (``tab5.2a.txt``): X of the CIP, IAU 2006/2000A. Eq. (5.16):
  ``X = polynomial + sum_j t^j sum_i [a_s,j sin(ARG) + a_c,j cos(ARG)]``.
- Table 5.2b (``tab5.2b.txt``): Y of the CIP, IAU 2006/2000A. Same form with
  the sine column first and the cosine column second (``b_s``, ``b_c``).
- Table 5.2d (``tab5.2d.txt``): the quantity ``s + XY/2``, IAU 2006/2000A.
  Same form with ``C_s`` (sine) and ``C_c`` (cosine).
- Table 5.2e (``tab5.2e.txt``): the non-polynomial part of the ERA-based GST
  expression, i.e. the complementary terms of the equation of the origins
  (``C'_s`` sine and ``C'_c`` cosine, j = 0 and j = 1 only). The polynomial part
  of that table is the GMST polynomial and is not emitted here.

Each block of the tables is ``j = k  Number of terms = N``. Row indices run
continuously from 1 across all blocks of one table. Each row is an index, two
amplitudes in microarcseconds, and 14 integer multipliers of F1..F14 (l, l',
F, D, Om, L_Me, L_Ve, L_E, L_Ma, L_J, L_Sa, L_U, L_Ne, p_A), the same argument
order as ``nutation.rs``. The generator fails loudly if an input's sha256
differs from the pinned value, if a row is malformed, if the row indices are
not 1..N in order, or if a block's row count differs from its header.

The polynomial parts of X, Y and s + XY/2 are not table rows. They are copied
into the generator below as literal strings, and the generator checks that
each string occurs verbatim in its source file.

Usage (run from the ``tpt-planetary-astro`` directory or the repository root)::

    python tools/gen_cio_table.py <tab5.2a.txt> <tab5.2b.txt> <tab5.2d.txt> <tab5.2e.txt> [--out FILE]

The default output is ``src/cio_data.rs`` next to this script's crate.
"""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
from pathlib import Path

# Pinned input checksums (sha256) of the IERS Conventions 2010 Chapter 5 files.
EXPECTED_SHA256 = {
    "tab5.2a.txt": "19549252df9eb77c8237dbf5b749de82b7fd7713a8b60b35371538cd36b6aa1d",
    "tab5.2b.txt": "1f17a3a6ad0b468705b3323bfe72d0320ce996798ec7cba0cdde4405d88f14d9",
    "tab5.2d.txt": "fe94c83e1ef6f92b15b3f007779ae70c0984c06ee45d5511b4c821ee9a0ecded",
    "tab5.2e.txt": "a5c690b277108e35d88df9cc20fea4120dfbc10a81444bb41b15cf9c6edcd6cb",
}

# Polynomial parts as printed in each table (unit microarcsecond), verbatim.
POLY_TEXT = {
    "tab5.2a.txt": " - 16617. + 2004191898. t - 429782.9 t^2 - 198618.34 t^3 + 7.578 t^4 + 5.9285 t^5",
    "tab5.2b.txt": " - 6951. - 25896. t - 22407274.7 t^2 + 1900.59 t^3 + 1112.526 t^4 + 0.1358 t^5",
    "tab5.2d.txt": "  94.0 + 3808.65 t - 122.68 t^2 - 72574.11 t^3 + 27.98 t^4 + 15.62 t^5",
}

# Number of blocks (j = 0..K-1) expected in each table.
N_BLOCKS = {"tab5.2a.txt": 5, "tab5.2b.txt": 5, "tab5.2d.txt": 5, "tab5.2e.txt": 2}

N_ARGS = 14
HEADER_RE = re.compile(r"^j\s*=\s*(\d+)\s+Number\s+of\s+terms\s*=\s*(\d+)\s*$")
ROW_RE = re.compile(r"^\s*(\d+)\s+(\S+)\s+(\S+)((?:\s+-?\d+){%d})\s*$" % N_ARGS)
HEADING_RE = re.compile(r"^\s*i\s")
INT_RE = re.compile(r"^-?\d+$")
FLOAT_RE = re.compile(r"^-?\d+(\.\d+)?$")

Row = tuple[list[int], str, str]  # (multipliers, first amplitude, second amplitude)


class GenerationError(Exception):
    """Raised for any parse, count or checksum mismatch."""


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 16), b""):
            h.update(chunk)
    return h.hexdigest()


def parse_table(path: Path) -> dict[int, list[Row]]:
    """Parse all blocks of one table into ``{j: rows}``."""
    lines = path.read_text(encoding="ascii").splitlines()
    blocks: dict[int, list[Row]] = {}
    declared: dict[int, int] = {}
    current_j: int | None = None
    next_index = 1  # indices run continuously across the blocks

    for lineno, raw in enumerate(lines, start=1):
        line = raw.strip()
        header = HEADER_RE.match(line)
        if header:
            current_j = int(header.group(1))
            if current_j in blocks:
                raise GenerationError(f"{path.name}: duplicate block j={current_j}")
            if current_j != len(blocks):
                raise GenerationError(f"{path.name}: block j={current_j} out of order")
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

    expected = N_BLOCKS[path.name]
    if sorted(blocks) != list(range(expected)):
        raise GenerationError(f"{path.name}: blocks {sorted(blocks)} found, expected 0..{expected - 1}")
    for j in range(expected):
        if len(blocks[j]) != declared[j]:
            raise GenerationError(
                f"{path.name}: j={j} parsed {len(blocks[j])} rows, header says {declared[j]}"
            )
    return blocks


def emit_block(name: str, doc: str, rows: list[Row]) -> str:
    body = "\n".join(
        f"    Term {{ args: [{', '.join(str(m) for m in mults)}], sin: {s}, cos: {c} }},"
        for mults, s, c in rows
    )
    return f"/// {doc}\n#[rustfmt::skip]\npub static {name}: [Term; {len(rows)}] = [\n{body}\n];\n"


def emit_table(prefix: str, blocks: dict[int, list[Row]], label: str) -> str:
    out = []
    names = []
    for j in sorted(blocks):
        name = f"{prefix}_J{j}"
        names.append(name)
        out.append(emit_block(name, f"{label}, j={j}.", blocks[j]))
    refs = ", ".join(f"&{n}" for n in names)
    out.append(
        f"/// {label}: element `j` multiplies `t^j`.\n"
        f"pub static {prefix}_BLOCKS: [&[Term]; {len(names)}] = [{refs}];\n"
    )
    return "\n".join(out)


def check_polynomials(paths: dict[str, Path]) -> None:
    for name, text in POLY_TEXT.items():
        src = paths[name].read_text(encoding="ascii")
        if text not in src:
            raise GenerationError(f"{name}: polynomial line not found verbatim: {text!r}")


def build(paths: dict[str, Path], out_path: Path, command: str) -> dict[str, int]:
    for p in paths.values():
        digest = sha256_of(p)
        if digest != EXPECTED_SHA256.get(p.name):
            raise GenerationError(
                f"{p.name}: sha256 {digest} does not match the pinned value "
                f"{EXPECTED_SHA256.get(p.name)}"
            )
    check_polynomials(paths)
    x = parse_table(paths["tab5.2a.txt"])
    y = parse_table(paths["tab5.2b.txt"])
    sxy = parse_table(paths["tab5.2d.txt"])
    eq = parse_table(paths["tab5.2e.txt"])
    counts = {
        "X": {j: len(v) for j, v in x.items()},
        "Y": {j: len(v) for j, v in y.items()},
        "SXY": {j: len(v) for j, v in sxy.items()},
        "EQ": {j: len(v) for j, v in eq.items()},
    }
    sha = {name: sha256_of(p) for name, p in paths.items()}

    def fmt(c: dict[int, int]) -> str:
        return ", ".join(f"j={j} {n}" for j, n in sorted(c.items()))

    header = f"""//! CIP coordinates X, Y and the CIO locator quantity s + XY/2, from IERS
//! Conventions (2010), Chapter 5, Tables 5.2a, 5.2b, 5.2d and 5.2e (IAU
//! 2006/2000A). GENERATED FILE: do not edit by hand.
//! Regenerate with the command below.
//!
//! Sources (sha256):
//! - tab5.2a.txt (Table 5.2a, X of the CIP): {sha["tab5.2a.txt"]}
//! - tab5.2b.txt (Table 5.2b, Y of the CIP): {sha["tab5.2b.txt"]}
//! - tab5.2d.txt (Table 5.2d, s + XY/2): {sha["tab5.2d.txt"]}
//! - tab5.2e.txt (Table 5.2e, GST complementary terms, non-polynomial part): {sha["tab5.2e.txt"]}
//!
//! Generator: tpt-planetary-astro/tools/gen_cio_table.py
//! Command:   {command}
//!
//! Row counts per block, verified against the "Number of terms" headers:
//! - X: {fmt(counts["X"])}
//! - Y: {fmt(counts["Y"])}
//! - s + XY/2: {fmt(counts["SXY"])}
//! - EO complementary terms: {fmt(counts["EQ"])}
//!
//! Units: microarcseconds. `sin` and `cos` are the coefficients of sin(ARG)
//! and cos(ARG), multiplied by `t^j` for block `j`, with `t` in Julian
//! centuries of TT from J2000. `args` multiplies F1..F14 in the order of
//! [`crate::nutation::fundamental_arguments`]. For Y, the table's first
//! amplitude column is the sine coefficient and the second is the cosine
//! coefficient (`b_s`, `b_c`), as printed in the table headers.
//!
//! Polynomial parts are not in this file; see `cio.rs`.

// Published amplitudes such as 6.28 microarcseconds are data, not 2*pi.
#![allow(clippy::approx_constant)]

use crate::nutation::Term;

"""
    body = "\n".join(
        [
            emit_table("X", x, "Table 5.2a: X of the CIP, non-polynomial part"),
            emit_table("Y", y, "Table 5.2b: Y of the CIP, non-polynomial part"),
            emit_table("SXY", sxy, "Table 5.2d: s + XY/2, non-polynomial part"),
            emit_table("EQ", eq, "Table 5.2e: EO complementary terms (non-polynomial part)"),
        ]
    )
    out_path.write_text(header + body, encoding="utf-8", newline="\n")
    return {"X": len(x), "Y": len(y), "SXY": len(sxy), "EQ": len(eq)}


def main(argv: list[str]) -> int:
    default_out = Path(__file__).resolve().parents[1] / "src" / "cio_data.rs"
    ap = argparse.ArgumentParser(description="Generate the CIP X, Y and s tables.")
    ap.add_argument("tab5_2a", type=Path, help="path to tab5.2a.txt")
    ap.add_argument("tab5_2b", type=Path, help="path to tab5.2b.txt")
    ap.add_argument("tab5_2d", type=Path, help="path to tab5.2d.txt")
    ap.add_argument("tab5_2e", type=Path, help="path to tab5.2e.txt")
    ap.add_argument("--out", type=Path, default=default_out, help="output .rs file")
    args = ap.parse_args(argv)

    paths = {
        "tab5.2a.txt": args.tab5_2a,
        "tab5.2b.txt": args.tab5_2b,
        "tab5.2d.txt": args.tab5_2d,
        "tab5.2e.txt": args.tab5_2e,
    }
    command = "python tpt-planetary-astro/tools/gen_cio_table.py " + " ".join(
        p.name for p in paths.values()
    )
    try:
        counts = build(paths, args.out, command)
    except GenerationError as err:
        print(f"gen_cio_table: ERROR: {err}", file=sys.stderr)
        return 1
    print(f"wrote {args.out}")
    for name, n in counts.items():
        print(f"  {name}: {n} blocks")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
