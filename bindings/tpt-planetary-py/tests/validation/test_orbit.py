"""SGP4 near-Earth propagation and TLE parsing (``tpt_planetary_astro::orbit``).

Reference: the Python ``sgp4`` package (Vallado's SGP4, MIT licence), using its
accelerated ``Satrec.twoline2rv`` with the ``WGS72`` constants and the default
``i`` operation mode. The crate uses the same WGS-72 constants and the same
algorithm (Vallado et al., "Revisiting Spacetrack Report #3", AIAA 2006).

Test elements are generated here as TLE text with a seeded generator. The
checksum is computed by this file (not by the crate), so the parser's checksum
handling is checked against an independent implementation. The reference
states are computed by ``sgp4`` from that text.

Tolerances (set before measuring):
- Position: 1e-6 km (1 mm) for every case. This is the target stated for the
  implementation; see the report for the measured maxima.
- Velocity: 1e-9 km/s (1 micrometre per second).

Cases:
- 200 random element sets (mean motion 12 to 16 rev/day, eccentricity 0 to 0.2),
  each at one random time in [-1440, 1440] minutes, and 25 of them at nine fixed
  times from -1440 to +1440 minutes. Element sets that decay at any of the
  fixed times are discarded and redrawn, so that the numerical comparison
  covers only orbits that are propagated without error.
- Decay and eccentricity errors: high-mean-motion, high-eccentricity sets must
  produce the same error (or the same valid states) in both codes.
- Deep space (period of 225 minutes or more) is rejected by the crate.
- A corrupted checksum is rejected by the crate.
"""

from __future__ import annotations

import math
import subprocess

import numpy as np
import pytest
from _reference import REPO_ROOT
from sgp4.api import WGS72, Satrec

SEED = 20261116
N_RANDOM = 200
N_GRID_SETS = 25
GRID_MINUTES = [-1440.0, -720.0, -360.0, -60.0, 0.0, 60.0, 360.0, 720.0, 1440.0]
POS_TOL_KM = 1.0e-6
VEL_TOL_KM_S = 1.0e-9

# Maps sgp4 error codes to the crate's OrbitError variant names.
SGP4_ERROR_NAMES = {
    1: "EccentricityOutOfRange",
    3: "EccentricityOutOfRange",
    4: "SemiLatusRectumNegative",
    6: "Decayed",
}


def tle_checksum(body: str) -> int:
    """Checksum digit of the first 68 characters (minus signs count as 1)."""
    total = 0
    for ch in body[:68]:
        if ch.isdigit():
            total += int(ch)
        elif ch == "-":
            total += 1
    return total % 10


def _implied(value: float) -> str:
    """Eight-character implied-decimal field, e.g. ``' 28098-4'`` for 2.8098e-5."""
    if value == 0.0:
        return " 00000-0"
    sign = "-" if value < 0 else " "
    magnitude = abs(value)
    exponent = math.floor(math.log10(magnitude)) + 1
    mantissa = round(magnitude / 10.0**exponent * 1e5)
    if mantissa >= 100000:
        mantissa //= 10
        exponent += 1
    return f"{sign}{mantissa:05d}{'-' if exponent < 0 else '+'}{abs(exponent)}"


def _signed_fraction(value: float) -> str:
    """Ten-character signed field such as ``' .00000023'`` (leading zero dropped)."""
    text = f"{abs(value):.8f}"[1:]
    return ("-" if value < 0 else " ") + text


def make_tle(
    satnum: int,
    intl: str,
    epoch_day: float,
    ndot: float,
    bstar: float,
    incl_deg: float,
    raan_deg: float,
    ecc: float,
    argp_deg: float,
    mean_anomaly_deg: float,
    mean_motion: float,
    revnum: int,
    elnum: int,
) -> tuple[str, str]:
    """Builds the two TLE lines (year 2026) with checksums in column 69."""
    line1 = (
        f"1 {satnum:05d}U {intl:<8s} 26{epoch_day:012.8f} "
        f"{_signed_fraction(ndot)} {_implied(0.0)} {_implied(bstar)} 0 {elnum:4d}"
    )
    # Eccentricity has seven digits with the leading "0." dropped.
    ecc_digits = f"{ecc:.7f}"[2:]
    line2 = (
        f"2 {satnum:05d} {incl_deg:8.4f} {raan_deg:8.4f} {ecc_digits}"
        + f" {argp_deg:8.4f} {mean_anomaly_deg:8.4f} {mean_motion:11.8f}{revnum:5d}"
    )
    assert len(line1) == 68, f"line 1 body is {len(line1)} characters"
    assert len(line2) == 68, f"line 2 body is {len(line2)} characters"
    return line1 + str(tle_checksum(line1)), line2 + str(tle_checksum(line2))


def random_elements(rng: np.random.Generator, satnum: int, n_lo: float, n_hi: float,
                    e_lo: float, e_hi: float) -> tuple[str, str]:
    """A random element set with mean motion in [n_lo, n_hi] rev/day."""
    intl = f"{rng.integers(0, 27):02d}{rng.integers(1, 999):03d}A"
    return make_tle(
        satnum=satnum,
        intl=intl,
        epoch_day=float(rng.uniform(1.0, 364.0)),
        ndot=float(rng.uniform(-1e-5, 1e-5)),
        bstar=float(rng.choice([-1.0, 1.0]) * 10.0 ** rng.uniform(-5.0, -3.0)),
        incl_deg=float(rng.uniform(0.0, 180.0)),
        raan_deg=float(rng.uniform(0.0, 360.0)),
        ecc=float(rng.uniform(e_lo, e_hi)),
        argp_deg=float(rng.uniform(0.0, 360.0)),
        mean_anomaly_deg=float(rng.uniform(0.0, 360.0)),
        mean_motion=float(rng.uniform(n_lo, n_hi)),
        revnum=int(rng.integers(0, 99999)),
        elnum=int(rng.integers(1, 999)),
    )


def reference_state(sat: Satrec, minutes: float) -> tuple[int, np.ndarray, np.ndarray]:
    """sgp4 state at ``minutes`` after epoch: (error code, position km, velocity km/s)."""
    code, r, v = sat.sgp4(sat.jdsatepoch, sat.jdsatepochF + minutes / 1440.0)
    return code, np.asarray(r, dtype=float), np.asarray(v, dtype=float)


def run_orbit_dump(records: list[tuple[str, str, float]]) -> list[str]:
    """Runs ``orbit_dump`` on the records and returns one output line per record.

    Mirrors ``_reference.run_validation_dump``: ``cargo run`` of the example,
    records on stdin, one result line per record on stdout.
    """
    stdin = "\n".join(f"{l1}|{l2}|{float(t)!r}" for l1, l2, t in records) + "\n"
    proc = subprocess.run(
        [
            "cargo",
            "run",
            "--quiet",
            "-p",
            "tpt-planetary-astro",
            "--example",
            "orbit_dump",
        ],
        input=stdin,
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
        check=False,
    )
    assert proc.returncode == 0, f"orbit_dump failed: {proc.stderr}"
    lines = proc.stdout.strip().splitlines()
    assert len(lines) == len(records), "orbit_dump returned the wrong number of lines"
    return lines


def parse_ok(line: str) -> tuple[np.ndarray, np.ndarray]:
    """Parses an ``ok x y z vx vy vz`` line."""
    fields = line.split()
    assert fields[0] == "ok", f"expected a state, got {line!r}"
    values = np.array([float(x) for x in fields[1:]])
    return values[:3], values[3:]


@pytest.fixture(scope="module")
def element_sets() -> list[tuple[str, str]]:
    """Random near-Earth element sets whose propagation is error-free at the test times."""
    rng = np.random.default_rng(SEED)
    accepted: list[tuple[str, str]] = []
    rejected = 0
    satnum = 10000
    while len(accepted) < N_RANDOM:
        satnum += 1
        l1, l2 = random_elements(rng, satnum, 12.0, 16.0, 0.0, 0.2)
        sat = Satrec.twoline2rv(l1, l2, WGS72)
        times = GRID_MINUTES + [float(rng.uniform(-1440.0, 1440.0))]
        if sat.error != 0 or any(reference_state(sat, t)[0] != 0 for t in times):
            rejected += 1
            continue
        accepted.append((l1, l2))
    print(f"\nelement sets accepted: {len(accepted)}, rejected (decayed or init error): {rejected}")
    return accepted


def test_tle_checksum_rejects_corruption_in_both_codes(element_sets):
    """The reference accepts each generated line; the crate rejects a corrupted one."""
    l1, l2 = element_sets[0]
    from sgp4.io import verify_checksum

    verify_checksum(l1, l2)
    corrupted = l1[:20] + str((int(l1[20]) + 1) % 10) + l1[21:]
    records = [(corrupted, l2, 0.0)]
    assert run_orbit_dump(records) == ["err TleChecksum { line: 1 }"]


def test_random_element_sets_match_sgp4(element_sets):
    """Position and velocity agree with sgp4 at 200 random times and on a time grid."""
    records: list[tuple[str, str, float]] = []
    refs: list[tuple[np.ndarray, np.ndarray]] = []
    skipped = 0
    rng = np.random.default_rng(SEED + 1)
    for index, (l1, l2) in enumerate(element_sets):
        sat = Satrec.twoline2rv(l1, l2, WGS72)
        times = [float(rng.uniform(-1440.0, 1440.0))]
        if index < N_GRID_SETS:
            times = GRID_MINUTES + times
        for t in times:
            code, r, v = reference_state(sat, t)
            if code != 0:
                # The reference reports an error (decay) at this time; the
                # comparison only covers error-free states.
                skipped += 1
                continue
            records.append((l1, l2, t))
            refs.append((r, v))
    assert skipped <= 0.05 * (len(records) + skipped), f"too many reference errors: {skipped}"

    lines = run_orbit_dump(records)
    max_pos_km = 0.0
    max_vel_km_s = 0.0
    failures = []
    for (l1, l2, t), line, (r_ref, v_ref) in zip(records, lines, refs):
        if not line.startswith("ok"):
            failures.append(f"{t} min: {line}")
            continue
        r, v = parse_ok(line)
        max_pos_km = max(max_pos_km, float(np.linalg.norm(r - r_ref)))
        max_vel_km_s = max(max_vel_km_s, float(np.linalg.norm(v - v_ref)))

    print(
        f"\ncases: {len(records)}; max position difference {max_pos_km * 1e3:.6g} m; "
        f"max velocity difference {max_vel_km_s * 1e3:.6g} m/s"
    )
    assert not failures, failures[:5]
    assert max_pos_km <= POS_TOL_KM, f"position {max_pos_km} km exceeds {POS_TOL_KM} km"
    assert max_vel_km_s <= VEL_TOL_KM_S, f"velocity {max_vel_km_s} km/s exceeds {VEL_TOL_KM_S}"


def test_decay_and_eccentricity_errors_agree():
    """High mean motion and eccentricity: the same error, or the same states, in both codes."""
    rng = np.random.default_rng(SEED + 2)
    records: list[tuple[str, str, float]] = []
    expected: list[str] = []
    for index in range(100):
        l1, l2 = random_elements(rng, 50000 + index, 15.0, 16.0, 0.1, 0.2)
        sat = Satrec.twoline2rv(l1, l2, WGS72)
        for t in GRID_MINUTES:
            code, r, v = reference_state(sat, t)
            records.append((l1, l2, t))
            if code == 0:
                expected.append("ok")
            else:
                expected.append(SGP4_ERROR_NAMES[code])

    lines = run_orbit_dump(records)
    decayed = sum(1 for e in expected if e == "Decayed")
    mismatches = []
    for want, got in zip(expected, lines):
        if want == "ok":
            if not got.startswith("ok"):
                mismatches.append((want, got))
        elif got != f"err {want}":
            mismatches.append((want, got))
    print(f"\ndecay and eccentricity cases: {len(records)}, decayed in sgp4: {decayed}")
    assert not mismatches, mismatches[:5]


def test_deep_space_is_rejected():
    """Period of about 720 minutes: the crate returns DeepSpace; sgp4 would use SDP4."""
    rng = np.random.default_rng(SEED + 3)
    l1, l2 = random_elements(rng, 60001, 2.0, 2.0, 0.01, 0.02)
    assert run_orbit_dump([(l1, l2, 0.0)]) == ["err DeepSpace"]


def test_bad_line_length_is_rejected():
    """A truncated line is a format error, not a checksum error."""
    rng = np.random.default_rng(SEED + 4)
    l1, l2 = random_elements(rng, 60002, 14.0, 14.0, 0.01, 0.01)
    assert run_orbit_dump([(l1[:60], l2, 0.0)]) == ["err InvalidTle { line: 1 }"]
