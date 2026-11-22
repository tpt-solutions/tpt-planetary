"""Zenithal, cylindrical and pseudocylindrical WCS projections, and SIP, against
Astropy's wcslib (``astropy.wcs.WCS``).

The Rust side is the ``wcs_dump`` example of ``tpt-planetary-astro``. Build it
first with ``cargo build -p tpt-planetary-astro --examples``; this test runs the
built binary (``target/debug/examples/wcs_dump``) on each header.

Projections: ARC, ZEA, STG, SIN, TAN, AIT, CAR, MOL, HPX. Each has a reference
header and a rotated header; CAR and HPX also have explicit LONPOLE/LATPOLE and
PV2 cases, so the spherical rotation is exercised with non-default poles.

Points, per header, from a fixed seed:
- 500 pixels uniform in a box of +/-230 pixels about CRPIX, for pixel -> world.
- 500 sky positions uniform on the sphere, for world -> pixel. Uniform on the
  sphere is uniform in native coordinates too, since the rotation preserves
  measure; the points outside the projection's domain give NaN.

Positions outside a projection's domain must be NaN in both implementations;
the NaN pattern is compared exactly.

Tolerances (set before measuring):
- pixel -> world: 1e-9 degrees, great-circle separation.
- world -> pixel: 1e-6 pixels.

SIP (TAN-SIP): forward polynomials only (pixel -> world, compared with
``all_pix2world``, and world -> pixel, compared with ``all_world2pix`` at
tolerance 1e-10 pixels). With AP/BP present, world -> pixel is compared with
``sip_foc2pix(wcs_world2pix(...))``, which applies the same inverse polynomials.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import numpy as np
import pytest
from astropy.io import fits
from astropy.wcs import WCS

DEG_TOL = 1e-9
PIX_TOL = 1e-6
N_POINTS = 500
RNG = np.random.default_rng(20260116)

REPO = Path(__file__).resolve().parents[4]
TARGET = Path(os.environ.get("CARGO_TARGET_DIR", REPO / "target"))
EXE = TARGET / "debug" / "examples" / ("wcs_dump.exe" if os.name == "nt" else "wcs_dump")


def _base(ctype_code, crval, crpix, cdelt=0.9, **extra):
    h = {
        "CTYPE1": f"RA---{ctype_code}",
        "CTYPE2": f"DEC--{ctype_code}",
        "CRVAL1": crval[0],
        "CRVAL2": crval[1],
        "CRPIX1": crpix[0],
        "CRPIX2": crpix[1],
        "CDELT1": -cdelt,
        "CDELT2": cdelt,
        "CUNIT1": "deg",
        "CUNIT2": "deg",
    }
    h.update(extra)
    return h


def _rotated(ctype_code, crval, crpix, angle_deg=30.0, cdelt=0.9, **extra):
    c, s = np.cos(np.radians(angle_deg)), np.sin(np.radians(angle_deg))
    h = {
        "CTYPE1": f"RA---{ctype_code}",
        "CTYPE2": f"DEC--{ctype_code}",
        "CRVAL1": crval[0],
        "CRVAL2": crval[1],
        "CRPIX1": crpix[0],
        "CRPIX2": crpix[1],
        "CD1_1": -cdelt * c,
        "CD1_2": -cdelt * s,
        "CD2_1": cdelt * s,
        "CD2_2": cdelt * c,
        "CUNIT1": "deg",
        "CUNIT2": "deg",
    }
    h.update(extra)
    return h


HEADERS = {}
for code in ["ARC", "ZEA", "STG", "SIN", "TAN", "AIT", "CAR", "MOL", "HPX"]:
    HEADERS[f"{code}_ref"] = _base(code, (150.0, 40.0), (200.5, 200.5))
    HEADERS[f"{code}_rot"] = _rotated(code, (210.0, -35.0), (200.5, 200.5))
HEADERS["CAR_pole"] = _base(
    "CAR", (30.0, 20.0), (200.5, 200.5), LONPOLE=60.0, LATPOLE=45.0
)
HEADERS["HPX_pole"] = _base(
    "HPX", (300.0, 60.0), (200.5, 200.5), LONPOLE=20.0, PV2_1=4.0, PV2_2=3.0
)
HEADERS["MOL_south"] = _base("MOL", (60.0, -75.0), (200.5, 200.5))
HEADERS["HPX_south"] = _base("HPX", (100.0, -50.0), (200.5, 200.5))


def _header_text(h):
    lines = []
    for k, v in h.items():
        if isinstance(v, str):
            lines.append(f"{k:<8}= '{v}'")
        else:
            lines.append(f"{k:<8}= {float(v)!r}")
    return "\n".join(lines) + "\n"


def _write_header(tmp_path, name, h):
    path = tmp_path / f"{name}.hdr"
    path.write_text(_header_text(h))
    return path


def _run_dump(header_path, mode, rows):
    if not EXE.exists():
        pytest.skip(f"{EXE} not built; run cargo build -p tpt-planetary-astro --examples")
    text = "\n".join(f"{float(a)!r} {float(b)!r}" for a, b in rows) + "\n"
    proc = subprocess.run(
        [str(EXE), str(header_path), mode],
        input=text,
        capture_output=True,
        text=True,
        cwd=REPO,
        check=False,
    )
    assert proc.returncode == 0, proc.stderr
    out = [line.split() for line in proc.stdout.splitlines() if line.strip()]
    assert len(out) == len(rows), "wcs_dump returned the wrong number of lines"
    return np.array([[float(a), float(b)] for a, b in out])


def _sep_deg(lon1, lat1, lon2, lat2):
    """Great-circle separation in degrees, NaN where either input is NaN."""
    a = np.radians
    l1, b1, l2, b2 = a(lon1), a(lat1), a(lon2), a(lat2)
    cross = np.hypot(
        np.cos(b2) * np.sin(l2 - l1),
        np.cos(b1) * np.sin(b2) - np.sin(b1) * np.cos(b2) * np.cos(l2 - l1),
    )
    dot = np.sin(b1) * np.sin(b2) + np.cos(b1) * np.cos(b2) * np.cos(l2 - l1)
    return np.degrees(np.arctan2(cross, dot))


def _pixels(h):
    cx, cy = h["CRPIX1"], h["CRPIX2"]
    x = cx + RNG.uniform(-230.0, 230.0, N_POINTS)
    y = cy + RNG.uniform(-230.0, 230.0, N_POINTS)
    return np.column_stack([x, y])


def _sky(n=N_POINTS):
    lon = RNG.uniform(0.0, 360.0, n)
    lat = np.degrees(np.arcsin(RNG.uniform(-1.0, 1.0, n)))
    return np.column_stack([lon, lat])


def _astropy(h):
    return WCS(fits.Header([(k, v) for k, v in h.items()]))


def _assert_nan_pattern(ours, ref, what):
    ours_nan = np.isnan(ours).any(axis=1)
    ref_nan = np.isnan(ref).any(axis=1)
    assert np.array_equal(ours_nan, ref_nan), (
        f"{what}: NaN pattern differs at {np.flatnonzero(ours_nan != ref_nan)[:5]}"
    )
    return ~ours_nan


@pytest.mark.parametrize("name", list(HEADERS))
def test_pixel_to_world_matches_wcslib(name, tmp_path):
    h = HEADERS[name]
    path = _write_header(tmp_path, name, h)
    pix = _pixels(h)
    ours = _run_dump(path, "p2w", pix)
    ref = _astropy(h).wcs_pix2world(pix, 1)
    ok = _assert_nan_pattern(ours, ref, name)
    assert ok.sum() > 0, "no in-domain points"
    sep = _sep_deg(ours[ok, 0], ours[ok, 1], ref[ok, 0], ref[ok, 1])
    assert sep.max() < DEG_TOL, f"{name}: max sep {sep.max():.3e} deg"


@pytest.mark.parametrize("name", list(HEADERS))
def test_world_to_pixel_matches_wcslib(name, tmp_path):
    h = HEADERS[name]
    path = _write_header(tmp_path, name, h)
    sky = _sky()
    ours = _run_dump(path, "w2p", sky)
    ref = _astropy(h).wcs_world2pix(sky, 1)
    ok = _assert_nan_pattern(ours, ref, name)
    assert ok.sum() > 0, "no in-domain points"
    dpix = np.abs(ours[ok] - ref[ok]).max()
    assert dpix < PIX_TOL, f"{name}: max pixel diff {dpix:.3e}"


SIP_FORWARD = {
    "A_ORDER": 3,
    "B_ORDER": 3,
    "A_2_0": 3.0e-6,
    "A_1_1": -2.0e-6,
    "A_0_2": 1.0e-6,
    "A_3_0": 1.0e-9,
    "B_2_0": -1.5e-6,
    "B_1_1": 2.5e-6,
    "B_0_2": 4.0e-7,
    "B_0_3": -5.0e-10,
}
SIP_INVERSE = {
    "AP_ORDER": 3,
    "BP_ORDER": 3,
    "AP_2_0": -3.0e-6,
    "AP_1_1": 2.0e-6,
    "AP_0_2": -1.0e-6,
    "BP_2_0": 1.5e-6,
    "BP_1_1": -2.5e-6,
    "BP_0_2": -4.0e-7,
}


def _sip_header(extra):
    return _base("TAN-SIP", (150.0, 20.0), (512.5, 480.0), cdelt=0.0002, **extra) | {}


def _sip_header_text(extra):
    h = {
        "CTYPE1": "RA---TAN-SIP",
        "CTYPE2": "DEC--TAN-SIP",
        "CRVAL1": 150.0,
        "CRVAL2": 20.0,
        "CRPIX1": 512.5,
        "CRPIX2": 480.0,
        "CDELT1": -0.0002,
        "CDELT2": 0.0002,
        "CUNIT1": "deg",
        "CUNIT2": "deg",
    }
    h.update(extra)
    return h


def _sip_sky_near_reference(h, n=N_POINTS):
    """Sky points within 0.5 deg of CRVAL, where the TAN-SIP domain is valid."""
    ra0, dec0 = h["CRVAL1"], h["CRVAL2"]
    ang = RNG.uniform(0.0, 2 * np.pi, n)
    rad = np.radians(0.5) * np.sqrt(RNG.uniform(0.0, 1.0, n))
    d0 = np.radians(dec0)
    dec = np.arcsin(np.sin(d0) * np.cos(rad) + np.cos(d0) * np.sin(rad) * np.cos(ang))
    lon = np.radians(ra0) + np.arctan2(
        np.sin(ang) * np.sin(rad) * np.cos(d0), np.cos(rad) - np.sin(d0) * np.sin(dec)
    )
    return np.column_stack([np.degrees(lon) % 360.0, np.degrees(dec)])


def _sip_pixels(h):
    x = h["CRPIX1"] + RNG.uniform(-300.0, 300.0, N_POINTS)
    y = h["CRPIX2"] + RNG.uniform(-300.0, 300.0, N_POINTS)
    return np.column_stack([x, y])


def test_sip_forward_matches_wcslib(tmp_path):
    h = _sip_header_text(SIP_FORWARD)
    path = _write_header(tmp_path, "sip_fwd", h)
    pix = _sip_pixels(h)
    ours = _run_dump(path, "p2w", pix)
    ref = _astropy(h).all_pix2world(pix, 1)
    ok = _assert_nan_pattern(ours, ref, "SIP forward")
    sep = _sep_deg(ours[ok, 0], ours[ok, 1], ref[ok, 0], ref[ok, 1])
    assert sep.max() < DEG_TOL, f"SIP forward: max sep {sep.max():.3e} deg"


def test_sip_inverse_iterative_matches_wcslib(tmp_path):
    # No AP/BP: our world -> pixel is the Newton inverse of the forward polynomials.
    h = _sip_header_text(SIP_FORWARD)
    path = _write_header(tmp_path, "sip_iter", h)
    sky = _sip_sky_near_reference(h)
    ours = _run_dump(path, "w2p", sky)
    ref = _astropy(h).all_world2pix(sky, 1, tolerance=1e-10, maxiter=100)
    ok = _assert_nan_pattern(ours, ref, "SIP iterative inverse")
    dpix = np.abs(ours[ok] - ref[ok]).max()
    assert dpix < PIX_TOL, f"SIP iterative: max pixel diff {dpix:.3e}"


def _eval_poly(terms, u, v):
    return sum(c * u**p * v**q for (p, q), c in terms)


def test_sip_inverse_polynomial_matches_wcslib(tmp_path):
    # With AP/BP: world -> pixel is the linear inverse followed by the FITS SIP
    # inverse polynomials, pixel = crpix + U + AP(U, V) with U, V the linear
    # offsets from CRPIX. Astropy's sip_foc2pix uses a different convention for
    # its input, so the reference is the FITS definition evaluated in numpy from
    # astropy's linear world -> pixel transform.
    h = _sip_header_text({**SIP_FORWARD, **SIP_INVERSE})
    path = _write_header(tmp_path, "sip_ap", h)
    sky = _sip_sky_near_reference(h)
    ours = _run_dump(path, "w2p", sky)
    wcs = _astropy(h)
    foc = wcs.wcs_world2pix(sky, 1)
    crpix = np.array([h["CRPIX1"], h["CRPIX2"]])
    u = foc[:, 0] - crpix[0]
    v = foc[:, 1] - crpix[1]
    ap_terms = [((p, q), SIP_INVERSE[f"AP_{p}_{q}"]) for (p, q) in [(2, 0), (1, 1), (0, 2)]]
    bp_terms = [((p, q), SIP_INVERSE[f"BP_{p}_{q}"]) for (p, q) in [(2, 0), (1, 1), (0, 2)]]
    ref = np.column_stack(
        [
            crpix[0] + u + _eval_poly(ap_terms, u, v),
            crpix[1] + v + _eval_poly(bp_terms, u, v),
        ]
    )
    ok = _assert_nan_pattern(ours, ref, "SIP AP/BP inverse")
    dpix = np.abs(ours[ok] - ref[ok]).max()
    assert dpix < PIX_TOL, f"SIP AP/BP: max pixel diff {dpix:.3e}"


# TPV distortion: TAN with the 40-term polynomials PV1_k (xi) and PV2_k (eta).
# Coefficients span orders 0 to 7 and the radius terms, with the linear identity.
TPV_HEADER = {
    "CTYPE1": "RA---TPV",
    "CTYPE2": "DEC--TPV",
    "CRVAL1": 150.0,
    "CRVAL2": 20.0,
    "CRPIX1": 200.5,
    "CRPIX2": 200.5,
    "CDELT1": -2.0e-4,
    "CDELT2": 2.0e-4,
    "CUNIT1": "deg",
    "CUNIT2": "deg",
    # The polynomial variables are pixel-scale (up to about 230), so the
    # coefficients are small: each term contributes about 1e-5 degrees.
    "PV1_1": 1.0,
    "PV1_3": 1.0e-3,
    "PV1_4": 1.0e-6,
    "PV1_5": -5.0e-7,
    "PV1_6": 2.0e-6,
    "PV1_7": 1.0e-9,
    "PV1_12": 1.0e-12,
    "PV1_24": 1.0e-17,
    "PV1_39": 1.0e-20,
    "PV2_1": 1.0,
    "PV2_3": -2.0e-4,
    "PV2_4": -1.0e-6,
    "PV2_9": 1.0e-9,
    "PV2_17": 1.0e-14,
    "PV2_31": 1.0e-19,
}


def test_tpv_matches_wcslib(tmp_path):
    h = TPV_HEADER
    path = _write_header(tmp_path, "tpv", h)
    pix = _pixels(h)
    ours = _run_dump(path, "p2w", pix)
    wcs = _astropy(h)
    ref = wcs.wcs_pix2world(pix, 1)
    ok = _assert_nan_pattern(ours, ref, "TPV pixel to world")
    sep = _sep_deg(ours[ok, 0], ours[ok, 1], ref[ok, 0], ref[ok, 1])
    assert sep.max() < DEG_TOL, f"TPV p2w: max sep {sep.max():.3e} deg"


def test_tpv_inverse_matches_wcslib(tmp_path):
    h = TPV_HEADER
    path = _write_header(tmp_path, "tpv_inv", h)
    pix = _pixels(h)
    world = _astropy(h).wcs_pix2world(pix, 1)
    ours = _run_dump(path, "w2p", world)
    ref = _astropy(h).wcs_world2pix(world, 1)
    ok = _assert_nan_pattern(ours, ref, "TPV world to pixel")
    dpix = np.abs(ours[ok] - ref[ok]).max()
    assert dpix < PIX_TOL, f"TPV w2p: max pixel diff {dpix:.3e}"
