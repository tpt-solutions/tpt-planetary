#!/usr/bin/env python3
"""Write a blosclz frame that needs a far back-reference (distance above 8191).

The data is a 600-byte random block, then 12000 bytes of a repeating filler
(which compresses), then the same 600-byte block again. The second copy is
matched at distance 12600, past the 8191 limit of the short back-reference
forms, so the frame uses the 16-bit far form. The frame is written by
python-blosc (c-blosc 1.x) with the blosclz back end, no shuffle and typesize 1,
into tpt-planetary-array/src/codec/testdata/, with the raw data beside it.

Usage: python tools/gen_blosc_far_fixture.py   (requires the blosc package)
"""

from pathlib import Path

import blosc
import numpy as np

OUT = Path(__file__).resolve().parent.parent / "tpt-planetary-array" / "src" / "codec" / "testdata"


def main() -> None:
    rng = np.random.default_rng(8191)
    block = rng.integers(0, 256, size=600, dtype=np.uint8).tobytes()
    filler = (b"abcdefgh" * 2000)[:12000]
    raw = block + filler + block
    frame = blosc.compress(raw, typesize=1, clevel=9, shuffle=blosc.NOSHUFFLE, cname="blosclz")
    assert blosc.decompress(frame) == raw
    (OUT / "blosclz_far.blosc").write_bytes(frame)
    (OUT / "blosclz_far.raw").write_bytes(raw)
    print(f"wrote {len(frame)} byte frame for {len(raw)} bytes of data")


if __name__ == "__main__":
    main()
