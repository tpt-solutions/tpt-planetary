"""Helpers for the reference-validation suite.

The reference packages (``pyerfa`` and ``astropy``) are imported directly by
the tests; they are not vendored. Quantities that the Python bindings do not
expose are computed by the ``validation_dump`` example of
``tpt-planetary-astro``, which reads records from stdin and prints one result
per line.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import numpy as np

REPO_ROOT = Path(__file__).resolve().parents[4]


def run_validation_dump(mode: str, rows: np.ndarray) -> np.ndarray:
    """Run ``validation_dump`` in ``mode`` on ``rows`` (shape (n, k)); return (n, m)."""
    rows = np.atleast_2d(np.asarray(rows, dtype=np.float64))
    stdin = "\n".join(" ".join(repr(float(x)) for x in r) for r in rows) + "\n"
    proc = subprocess.run(
        [
            "cargo",
            "run",
            "--quiet",
            "-p",
            "tpt-planetary-astro",
            "--example",
            "validation_dump",
            "--",
            mode,
        ],
        input=stdin,
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
        check=False,
    )
    assert proc.returncode == 0, f"validation_dump {mode} failed: {proc.stderr}"
    lines = proc.stdout.strip().splitlines()
    assert len(lines) == len(rows), "validation_dump returned the wrong number of lines"
    return np.array([[float(x) for x in line.split()] for line in lines])
