#!/usr/bin/env python3
"""Write a CK attitude kernel of data type 2 and the values SPICE reads from it.

tpt-planetary-astro/tests/fixtures/ck/attitude.bc is a DAF/CK file written here
byte by byte, following the type 2 layout of the NAIF CK required reading
(ck.req, "Data Type 2"): pointing records of eight doubles (the quaternion q0..q3
at the interval start, the angular velocity a1..a3 in rad/s, and the rate that
converts SCLK ticks to seconds), then the interval START times, the STOP times,
and no directory (there are fewer than 100 intervals).

The segment is for instrument -999 in the J2000 frame. It has two intervals of
500 SCLK ticks, with a tick of 1/120 s. Each interval starts from a given quaternion and
turns at a constant rate about a fixed base-frame axis, as type 2 intervals do.

The expected values are what spiceypy (CSPICE) returns from ckgpav at SCLK
ticks inside the segment: the C-matrix (row-major, 9 values), the angular
velocity (3 values, rad/s) and the clock time, one line per epoch in
attitude_expected.tsv.

Usage: python tools/gen_ck_fixture.py   (requires numpy and spiceypy)
"""

import struct
from pathlib import Path

import numpy as np
import spiceypy as sp

OUT = Path(__file__).resolve().parent.parent / "tpt-planetary-astro" / "tests" / "fixtures" / "ck"
INST = -999
FRAME = 1  # J2000
DATA_TYPE = 2
RECORD_BYTES = 1024
DOUBLE = 8
BEGIN = 1000.0
INTERVAL = 500.0
STARTS = [BEGIN, BEGIN + INTERVAL]
STOPS = [BEGIN + INTERVAL, BEGIN + 2 * INTERVAL]
TICK_SECONDS = 1.0 / 120.0
ANGLE_PER_TICK = 4.0e-4
AXIS = np.array([0.3, -0.5, 0.81]) / np.linalg.norm([0.3, -0.5, 0.81])
AV_RAD_PER_S = ANGLE_PER_TICK / TICK_SECONDS * AXIS


# The quaternion at each interval start is a rotation about a different axis from
# the angular velocity, so a test can tell whether the spin is applied on the left
# (in the base frame) or on the right of the start matrix.
START_AXIS = np.array([1.0, 0.2, -0.4]) / np.linalg.norm([1.0, 0.2, -0.4])


def start_quaternion(index: int) -> np.ndarray:
    """Scalar-first quaternion at the start of interval `index`."""
    half = (0.7 + 0.1 * index) / 2.0
    return np.array([np.cos(half), *(np.sin(half) * START_AXIS)])


def build_segment() -> list[float]:
    words: list[float] = []
    for index in range(len(STARTS)):
        q = start_quaternion(index)
        words.extend([*q, *AV_RAD_PER_S, TICK_SECONDS])
    words.extend(STARTS)
    words.extend(STOPS)
    return words


def file_record(free_word: int) -> bytes:
    rec = bytearray(RECORD_BYTES)
    rec[0:8] = b"DAF/CK  "
    struct.pack_into("<ii", rec, 8, 2, 6)
    rec[16:76] = b"TPT CK test attitude".ljust(60)
    struct.pack_into("<iii", rec, 76, 2, 2, free_word)
    rec[88:96] = b"LTL-IEEE"
    # FTP transfer check string at byte 700 (1-based); SPICE writes the same bytes.
    ftp = b"FTPSTR:\r:\n:\r\n:\r\x00:\x81:\x10\xce:ENDFTP"
    rec[699:699 + len(ftp)] = ftp
    return bytes(rec)


def summary_record(begin_word: int, end_word: int) -> bytes:
    rec = bytearray(RECORD_BYTES)
    struct.pack_into("<ddd", rec, 0, 0.0, 0.0, 1.0)  # next, prev, count
    struct.pack_into("<dd", rec, 24, BEGIN, BEGIN + 2 * INTERVAL)
    # Integers: instrument, frame, data type, AV flag, begin and end word.
    struct.pack_into("<iiiiii", rec, 40, INST, FRAME, DATA_TYPE, 1, begin_word, end_word)
    return bytes(rec)


def name_record() -> bytes:
    rec = bytearray(RECORD_BYTES)
    rec[0:40] = b"attitude segment, instrument -999".ljust(40)
    return bytes(rec)


def write(path: Path) -> None:
    words = build_segment()
    data_start_word = 3 * RECORD_BYTES // DOUBLE + 1  # after file, summary and name records
    begin_word = data_start_word
    end_word = data_start_word + len(words) - 1
    free_word = end_word + 1
    body = struct.pack(f"<{len(words)}d", *words)
    padded = body + b"\x00" * ((-len(body)) % RECORD_BYTES)
    blob = (
        file_record(free_word)
        + summary_record(begin_word, end_word)
        + name_record()
        + padded
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(blob)


def expected(path: Path) -> None:
    sp.furnsh(str(path))
    header = ["sclk"] + [f"c{i}" for i in range(9)] + [f"w{i}" for i in range(3)]
    lines = ["\t".join(header)]
    for t in np.linspace(BEGIN + 3.0, BEGIN + 2 * INTERVAL - 3.0, 11):
        result = sp.ckgpav(INST, float(t), 0.0, "J2000")
        cmat, av = result[0], result[1]
        values = [float(t)] + [float(v) for v in np.asarray(cmat).ravel()] + [float(v) for v in av]
        lines.append("\t".join(repr(v) for v in values))
    sp.kclear()
    (OUT / "attitude_expected.tsv").write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> None:
    bc = OUT / "attitude.bc"
    write(bc)
    expected(bc)
    print(f"wrote {bc} and attitude_expected.tsv")


if __name__ == "__main__":
    main()
