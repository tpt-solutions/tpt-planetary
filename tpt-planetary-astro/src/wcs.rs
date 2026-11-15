//! World Coordinate System (WCS) for celestial images: FITS header keywords,
//! linear pixel transform, the TAN, SIN and AIT projections, and the
//! native-to-celestial spherical rotation with LONPOLE and LATPOLE.
//!
//! Conventions follow Calabretta & Greisen (2002), "Representations of
//! celestial coordinates in FITS". Pixel coordinates are FITS 1-based. Angles
//! in the header are in degrees.
//!
//! Scope and limits:
//! - Celestial axes only (`RA`/`DEC`-style CTYPE pairs, or any pair whose
//!   projection code is TAN, SIN or AIT). The output is the lon/lat pair in
//!   whatever system the header names; no system conversion is applied.
//! - Only the primary (blank-suffix) WCS description is read. Alternate
//!   descriptions (`WCSNAMEa` and so on) are not.
//! - SIP distortion, TPV, and other distortion conventions are not supported
//!   and are deliberately deferred.
//! - Validation: the tests check analytic identities (the reference pixel maps
//!   to CRVAL, the TAN and SIN great-circle distance from CRVAL, the AIT edge
//!   position) and round trips. They do not compare against wcslib or Astropy
//!   output, so agreement with those libraries is not claimed.

use core::f64::consts::{PI, TAU};
use core::fmt;

/// Degrees per radian.
const R2D: f64 = 180.0 / PI;

/// Supported zenithal and pseudocylindrical projections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    /// Gnomonic (TAN): projection from the centre of the sphere onto a tangent plane.
    Tan,
    /// Orthographic (SIN, no slant): projection along the axis onto a tangent plane.
    Sin,
    /// Hammer-Aitoff (AIT): equal-area pseudocylindrical projection.
    Ait,
}

impl Projection {
    /// Parses the three-letter projection code, for example `TAN` from `RA---TAN`.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code.trim() {
            "TAN" => Some(Self::Tan),
            "SIN" => Some(Self::Sin),
            "AIT" => Some(Self::Ait),
            _ => None,
        }
    }

    /// Native latitude of the reference point, `theta_0`, in degrees.
    fn native_reference_lat_deg(self) -> f64 {
        match self {
            Self::Tan | Self::Sin => 90.0,
            Self::Ait => 0.0,
        }
    }

    /// Forward projection: native `(phi, theta)` in radians to intermediate
    /// `(x, y)` in degrees. Returns `None` outside the projection's domain.
    fn forward(self, phi: f64, theta: f64) -> Option<(f64, f64)> {
        match self {
            Self::Tan => {
                if theta <= 0.0 {
                    return None;
                }
                let r = 1.0 / theta.tan() * R2D;
                Some((r * phi.sin(), -r * phi.cos()))
            }
            Self::Sin => {
                if theta < 0.0 {
                    return None;
                }
                let r = theta.cos() * R2D;
                Some((r * phi.sin(), -r * phi.cos()))
            }
            Self::Ait => {
                let half = phi / 2.0;
                let s = (1.0 + theta.cos() * half.cos()).sqrt();
                if s <= 0.0 {
                    return None;
                }
                let x = 2.0 * core::f64::consts::SQRT_2 * theta.cos() * half.sin() / s * R2D;
                let y = core::f64::consts::SQRT_2 * theta.sin() / s * R2D;
                Some((x, y))
            }
        }
    }

    /// Inverse projection: intermediate `(x, y)` in degrees to native
    /// `(phi, theta)` in radians. Returns `None` outside the domain.
    fn inverse(self, x: f64, y: f64) -> Option<(f64, f64)> {
        match self {
            Self::Tan => {
                let r = (x * x + y * y).sqrt() / R2D;
                Some((x.atan2(-y), (1.0 / r).atan()))
            }
            Self::Sin => {
                let r = (x * x + y * y).sqrt() / R2D;
                if r > 1.0 {
                    return None;
                }
                // theta = acos(r), written as atan2 to keep precision near r = 1.
                let theta = ((1.0 - r) * (1.0 + r)).max(0.0).sqrt().atan2(r);
                Some((x.atan2(-y), theta))
            }
            Self::Ait => {
                let xr = x / R2D;
                let yr = y / R2D;
                let z2 = 1.0 - (xr / 4.0).powi(2) - (yr / 2.0).powi(2);
                if z2 <= 0.0 {
                    return None;
                }
                let z = z2.sqrt();
                // cos(theta) sin(phi/2) = xr z / 2 and cos(theta) cos(phi/2) = 2 z^2 - 1.
                let (cx, cy) = (xr * z / 2.0, 2.0 * z2 - 1.0);
                let theta = (yr * z).atan2(cx.hypot(cy));
                let phi = 2.0 * cx.atan2(cy);
                Some((phi, theta))
            }
        }
    }
}

/// Errors from header parsing and WCS construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WcsError {
    /// A required keyword is missing from the header.
    MissingKeyword(&'static str),
    /// The projection code is not one of TAN, SIN, AIT.
    UnsupportedProjection(String),
    /// The two axes name different projections.
    MismatchedProjection,
    /// The linear transform matrix is singular and cannot be inverted.
    SingularMatrix,
    /// No valid LATPOLE could be derived from the reference point.
    NoLatPole,
}

impl fmt::Display for WcsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingKeyword(k) => write!(f, "missing WCS keyword {k}"),
            Self::UnsupportedProjection(p) => write!(f, "unsupported projection {p}"),
            Self::MismatchedProjection => write!(f, "CTYPE1 and CTYPE2 name different projections"),
            Self::SingularMatrix => write!(f, "WCS linear transform is singular"),
            Self::NoLatPole => write!(f, "could not derive LATPOLE for the reference point"),
        }
    }
}

impl std::error::Error for WcsError {}

/// Result alias for WCS operations.
pub type Result<T> = core::result::Result<T, WcsError>;

/// Parameters of a celestial WCS, in the form the rest of the crate uses.
#[derive(Debug, Clone, PartialEq)]
pub struct WcsParams {
    /// Reference pixel `(CRPIX1, CRPIX2)`, FITS 1-based.
    pub crpix: [f64; 2],
    /// Reference world coordinate `(CRVAL1, CRVAL2)` in degrees.
    pub crval: [f64; 2],
    /// Linear matrix `CD` in degrees per pixel (row = intermediate axis).
    pub cd: [[f64; 2]; 2],
    /// Projection.
    pub projection: Projection,
    /// LONPOLE in degrees, or `None` to use the Calabretta & Greisen default.
    pub lonpole: Option<f64>,
    /// LATPOLE in degrees, or `None` to derive it from the reference point.
    pub latpole: Option<f64>,
}

/// A celestial World Coordinate System.
#[derive(Debug, Clone, PartialEq)]
pub struct Wcs {
    crpix: [f64; 2],
    crval_rad: [f64; 2],
    cd_inv: [[f64; 2]; 2],
    cd: [[f64; 2]; 2],
    projection: Projection,
    /// Native longitude of the celestial pole, `phi_p`, in radians.
    phi_p: f64,
    /// Celestial right ascension of the native pole, `alpha_p`, in radians.
    alpha_p: f64,
    /// Celestial declination of the native pole, `delta_p`, in radians.
    delta_p: f64,
}

impl Wcs {
    /// Builds a WCS from explicit parameters.
    ///
    /// # Errors
    ///
    /// Returns an error if the linear matrix is singular or LATPOLE cannot be
    /// derived.
    pub fn new(params: WcsParams) -> Result<Self> {
        let cd = params.cd;
        let det = cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0];
        if det == 0.0 || !det.is_finite() {
            return Err(WcsError::SingularMatrix);
        }
        let cd_inv = [
            [cd[1][1] / det, -cd[0][1] / det],
            [-cd[1][0] / det, cd[0][0] / det],
        ];

        let theta0 = params.projection.native_reference_lat_deg().to_radians();
        let alpha0 = params.crval[0].to_radians();
        let delta0 = params.crval[1].to_radians();

        // LONPOLE default (C&G Table 2): 0 if delta0 >= theta0, otherwise 180.
        let phi_p = match params.lonpole {
            Some(v) => v.to_radians(),
            None => {
                if delta0 >= theta0 {
                    0.0
                } else {
                    PI
                }
            }
        };

        // Solve sin(delta0) = sin(theta0) sin(delta_p) + cos(theta0) cos(phi0 - phi_p) cos(delta_p)
        // with phi0 = 0 for the reference point.
        let delta_p = match params.latpole {
            Some(v) => v.to_radians(),
            None => {
                let a = theta0.sin();
                let b = theta0.cos() * (-phi_p).cos();
                let r = a.hypot(b);
                if r == 0.0 {
                    return Err(WcsError::NoLatPole);
                }
                let gamma = b.atan2(a);
                let s = (delta0.sin() / r).clamp(-1.0, 1.0);
                let base = s.asin();
                // Two candidate roots; pick the one in [-pi/2, pi/2] closest to +pi/2.
                let candidates = [base - gamma, PI - base - gamma];
                candidates
                    .into_iter()
                    .filter(|c| (-PI / 2.0 - 1e-12..=PI / 2.0 + 1e-12).contains(c))
                    .fold(None::<f64>, |best, c| match best {
                        Some(b) if b >= c => Some(b),
                        _ => Some(c),
                    })
                    .ok_or(WcsError::NoLatPole)?
            }
        };

        // alpha_p from the reference point: alpha0 = alpha_p + atan2(...).
        let phi0_minus_phi_p = -phi_p;
        let (st0, ct0) = (theta0.sin(), theta0.cos());
        let (sd_p, cd_p) = (delta_p.sin(), delta_p.cos());
        let alpha_p = alpha0
            - (-ct0 * phi0_minus_phi_p.sin())
                .atan2(st0 * cd_p - ct0 * sd_p * phi0_minus_phi_p.cos());

        Ok(Self {
            crpix: params.crpix,
            crval_rad: [alpha0, delta0],
            cd_inv,
            cd,
            projection: params.projection,
            phi_p,
            alpha_p,
            delta_p,
        })
    }

    /// Reads the primary WCS from FITS header text.
    ///
    /// The text may be newline-separated lines or a block of 80-character
    /// cards. Recognised keywords: `CRPIXn`, `CRVALn`, `CDELTn`, `CTYPEn`,
    /// `PCi_j`, `CDi_j`, `LONPOLE`, `LATPOLE`. Missing `CRPIX` and `CRVAL`
    /// default to 0, `CDELT` to 1, `PC` to the identity. If any `CD` keyword is
    /// present, `CD` is used and `PC`/`CDELT` are ignored, as in FITS WCS.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing `CTYPE1`/`CTYPE2`, an unsupported
    /// projection, mismatched projection codes, or a singular matrix.
    pub fn from_header(text: &str) -> Result<Self> {
        let params = parse_header(text)?;
        Self::new(params)
    }

    /// Converts a FITS 1-based pixel position to celestial `(lon, lat)` in degrees.
    ///
    /// Returns `None` if the position lies outside the projection's domain.
    #[must_use]
    pub fn pixel_to_world(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let dx = x - self.crpix[0];
        let dy = y - self.crpix[1];
        let ix = self.cd[0][0] * dx + self.cd[0][1] * dy;
        let iy = self.cd[1][0] * dx + self.cd[1][1] * dy;
        let (phi, theta) = self.projection.inverse(ix, iy)?;
        let (alpha, delta) = self.native_to_celestial(phi, theta);
        // `rem_euclid` can return exactly 360.0 for a tiny negative input
        // (rounding), so fold that back to 0.0 to keep the range [0, 360).
        let ra = alpha.to_degrees().rem_euclid(360.0);
        let ra = if ra >= 360.0 { 0.0 } else { ra };
        Some((ra, delta.to_degrees()))
    }

    /// Converts celestial `(lon, lat)` in degrees to a FITS 1-based pixel position.
    ///
    /// Returns `None` if the sky position is outside the projection's domain
    /// (for example, the far hemisphere of TAN).
    #[must_use]
    pub fn world_to_pixel(&self, lon_deg: f64, lat_deg: f64) -> Option<(f64, f64)> {
        let (phi, theta) = self.celestial_to_native(lon_deg.to_radians(), lat_deg.to_radians());
        let (ix, iy) = self.projection.forward(phi, theta)?;
        let x = self.cd_inv[0][0] * ix + self.cd_inv[0][1] * iy;
        let y = self.cd_inv[1][0] * ix + self.cd_inv[1][1] * iy;
        Some((x + self.crpix[0], y + self.crpix[1]))
    }

    /// Native `(phi, theta)` to celestial `(alpha, delta)`, all in radians.
    ///
    /// Uses the Calabretta & Greisen Eq. (2) rotation written as a unit
    /// vector and `atan2`, which stays accurate near the celestial poles.
    fn native_to_celestial(&self, phi: f64, theta: f64) -> (f64, f64) {
        let (sth, cth) = theta.sin_cos();
        let (sdp, cdp) = self.delta_p.sin_cos();
        let (sdph, cdph) = (phi - self.phi_p).sin_cos();
        // cos(delta) cos(alpha - alpha_p), cos(delta) sin(alpha - alpha_p), sin(delta).
        let x = sth * cdp - cth * sdp * cdph;
        let y = -cth * sdph;
        let z = sth * sdp + cth * cdp * cdph;
        let delta = z.atan2(x.hypot(y));
        let alpha = self.alpha_p + y.atan2(x);
        (alpha, delta)
    }

    /// Celestial `(alpha, delta)` to native `(phi, theta)`, all in radians.
    fn celestial_to_native(&self, alpha: f64, delta: f64) -> (f64, f64) {
        let (sd, cd) = delta.sin_cos();
        let (sdp, cdp) = self.delta_p.sin_cos();
        let (sda, cda) = (alpha - self.alpha_p).sin_cos();
        // cos(theta) cos(phi - phi_p), -cos(theta) sin(phi - phi_p), sin(theta).
        let x = sd * cdp - cd * sdp * cda;
        let y = cd * sda;
        let z = sd * sdp + cd * cdp * cda;
        // Wrap to (-pi, pi]: the AIT forward formula is only valid on that range.
        let phi = (self.phi_p + (-y).atan2(x) + PI).rem_euclid(TAU) - PI;
        let theta = z.atan2(x.hypot(y));
        (phi, theta)
    }

    /// Celestial reference point `CRVAL` in degrees, `(lon, lat)`.
    #[must_use]
    pub fn crval_deg(&self) -> (f64, f64) {
        (
            self.crval_rad[0].to_degrees(),
            self.crval_rad[1].to_degrees(),
        )
    }
}

/// Splits header text into 80-character cards or lines.
fn cards(text: &str) -> Vec<&str> {
    if text.contains('\n') {
        text.lines().collect()
    } else {
        text.as_bytes()
            .chunks(80)
            .filter_map(|c| core::str::from_utf8(c).ok())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Value<'a> {
    Number(f64),
    Text(&'a str),
    Logical(bool),
}

/// Parses one FITS card of the form `KEYWORD = value / comment`.
fn parse_card(card: &str) -> Option<(&str, Value<'_>)> {
    let key = card.get(..8)?.trim_end();
    if card.get(8..10) != Some("= ") {
        return None;
    }
    let rest = card.get(10..)?.trim_start();
    if let Some(stripped) = rest.strip_prefix('\'') {
        let end = stripped.find('\'')?;
        return Some((key, Value::Text(stripped[..end].trim_end())));
    }
    let raw = rest.split('/').next().unwrap_or("").trim();
    if raw == "T" {
        return Some((key, Value::Logical(true)));
    }
    if raw == "F" {
        return Some((key, Value::Logical(false)));
    }
    let number: f64 = raw.replace(['D', 'd'], "E").parse().ok()?;
    Some((key, Value::Number(number)))
}

/// Parses an `i_j` index pair, for example `1_2` from `PC1_2`.
fn parse_pair(s: &str) -> Option<(usize, usize)> {
    let (a, b) = s.split_once('_')?;
    let i: usize = a.parse().ok()?;
    let j: usize = b.parse().ok()?;
    if (1..=2).contains(&i) && (1..=2).contains(&j) {
        Some((i - 1, j - 1))
    } else {
        None
    }
}

fn parse_header(text: &str) -> Result<WcsParams> {
    let mut crpix = [0.0; 2];
    let mut crval = [0.0; 2];
    let mut cdelt = [1.0; 2];
    let mut pc = [[1.0, 0.0], [0.0, 1.0]];
    let mut cd = [[0.0; 2]; 2];
    let mut have_cd = false;
    let mut ctype: [Option<String>; 2] = [None, None];
    let mut lonpole = None;
    let mut latpole = None;

    for card in cards(text) {
        let Some((key, value)) = parse_card(card) else {
            continue;
        };
        let number = match value {
            Value::Number(n) => Some(n),
            _ => None,
        };
        match key {
            "LONPOLE" => lonpole = number,
            "LATPOLE" => latpole = number,
            _ => {
                if let Some(idx) = key
                    .strip_prefix("CRPIX")
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    if let (Some(n), 1..=2) = (number, idx) {
                        crpix[idx - 1] = n;
                    }
                } else if let Some(idx) = key
                    .strip_prefix("CRVAL")
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    if let (Some(n), 1..=2) = (number, idx) {
                        crval[idx - 1] = n;
                    }
                } else if let Some(idx) = key
                    .strip_prefix("CDELT")
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    if let (Some(n), 1..=2) = (number, idx) {
                        cdelt[idx - 1] = n;
                    }
                } else if let Some(idx) = key
                    .strip_prefix("CTYPE")
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    if let (Value::Text(t), 1..=2) = (&value, idx) {
                        ctype[idx - 1] = Some(t.to_string());
                    }
                } else if let Some(pair) = key.strip_prefix("PC").and_then(parse_pair) {
                    if let Some(n) = number {
                        pc[pair.0][pair.1] = n;
                    }
                } else if let Some(pair) = key.strip_prefix("CD").and_then(parse_pair) {
                    if let Some(n) = number {
                        cd[pair.0][pair.1] = n;
                        have_cd = true;
                    }
                }
            }
        }
    }

    let code1 = projection_code(ctype[0].as_deref(), "CTYPE1")?;
    let code2 = projection_code(ctype[1].as_deref(), "CTYPE2")?;
    if code1 != code2 {
        return Err(WcsError::MismatchedProjection);
    }
    let projection = Projection::from_code(&code1).ok_or(WcsError::UnsupportedProjection(code1))?;

    let linear = if have_cd {
        cd
    } else {
        [
            [cdelt[0] * pc[0][0], cdelt[0] * pc[0][1]],
            [cdelt[1] * pc[1][0], cdelt[1] * pc[1][1]],
        ]
    };

    Ok(WcsParams {
        crpix,
        crval,
        cd: linear,
        projection,
        lonpole,
        latpole,
    })
}

fn projection_code(ctype: Option<&str>, keyword: &'static str) -> Result<String> {
    let ctype = ctype.ok_or(WcsError::MissingKeyword(keyword))?;
    // CTYPE has the form "RA---TAN" or "DEC--SIN": the projection code follows the last '-'.
    let code = ctype.rsplit('-').next().unwrap_or("").trim();
    Ok(code.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(projection: Projection, crval: [f64; 2], cd: [[f64; 2]; 2]) -> WcsParams {
        WcsParams {
            crpix: [100.5, 50.5],
            crval,
            cd,
            projection,
            lonpole: None,
            latpole: None,
        }
    }

    fn identity_cd(scale_deg: f64) -> [[f64; 2]; 2] {
        [[scale_deg, 0.0], [0.0, scale_deg]]
    }

    /// Absolute difference between two longitudes in degrees, wrapped to [0, 180].
    fn angle_diff_deg(a: f64, b: f64) -> f64 {
        ((a - b + 180.0).rem_euclid(360.0) - 180.0).abs()
    }

    /// Great-circle separation between two sky positions, in degrees (haversine).
    fn separation_deg(a: (f64, f64), b: (f64, f64)) -> f64 {
        let (ra1, de1) = (a.0.to_radians(), a.1.to_radians());
        let (ra2, de2) = (b.0.to_radians(), b.1.to_radians());
        let h = ((de2 - de1) / 2.0).sin().powi(2)
            + de1.cos() * de2.cos() * ((ra2 - ra1) / 2.0).sin().powi(2);
        (2.0 * h.sqrt().asin()).to_degrees()
    }

    #[test]
    fn tan_reference_pixel_returns_crval_exactly() {
        for (crval, lonpole) in [
            ([83.633_08, 22.014_46], None),
            ([0.0, 0.0], None),
            ([10.0, -45.0], Some(180.0)),
        ] {
            let mut p = params(Projection::Tan, crval, identity_cd(-0.0002));
            p.lonpole = lonpole;
            let w = Wcs::new(p).expect("wcs");
            let (ra, dec) = w.pixel_to_world(100.5, 50.5).expect("domain");
            assert!(angle_diff_deg(ra, crval[0]) < 1e-11, "ra={ra}");
            assert!((dec - crval[1]).abs() < 1e-11, "dec={dec}");
        }
    }

    #[test]
    fn sin_and_ait_reference_pixel_returns_crval() {
        for proj in [Projection::Sin, Projection::Ait] {
            let w = Wcs::new(params(proj, [120.0, 35.0], identity_cd(0.01))).expect("wcs");
            let (ra, dec) = w.pixel_to_world(100.5, 50.5).expect("domain");
            assert!((ra - 120.0).abs() < 1e-10, "{proj:?} ra={ra}");
            assert!((dec - 35.0).abs() < 1e-10, "{proj:?} dec={dec}");
        }
    }

    #[test]
    fn tan_distance_from_crval_satisfies_tan_d_equals_r() {
        // Intermediate radius R (radians) satisfies tan(d) = R, where d is the
        // angular distance from CRVAL. Choose R = 1 rad, so d = 45 degrees.
        let w = Wcs::new(params(Projection::Tan, [200.0, 30.0], identity_cd(1.0))).expect("wcs");
        let r_deg = 1.0_f64.to_degrees();
        // Intermediate offset along x; CD = identity with 1 deg per pixel.
        let (ra, dec) = w.pixel_to_world(100.5 + r_deg, 50.5).expect("domain");
        let d = separation_deg((ra, dec), (200.0, 30.0));
        assert!((d - 45.0).abs() < 1e-9, "d={d}");
    }

    #[test]
    fn sin_distance_from_crval_satisfies_sin_d_equals_r() {
        // For SIN, R = sin(d) (radians). R = 0.5 gives d = 30 degrees.
        let w = Wcs::new(params(Projection::Sin, [200.0, 30.0], identity_cd(1.0))).expect("wcs");
        let r_deg = 0.5_f64.to_degrees();
        let (ra, dec) = w.pixel_to_world(100.5, 50.5 + r_deg).expect("domain");
        let d = separation_deg((ra, dec), (200.0, 30.0));
        assert!((d - 30.0).abs() < 1e-9, "d={d}");
    }

    #[test]
    fn ait_edge_at_equator_is_two_root_two_radians() {
        // CRVAL (0,0), default LONPOLE 0 and LATPOLE 90: the native equator
        // point at phi = 180 deg maps to the celestial point (180, 0), which
        // sits at the Hammer edge x = 2*sqrt(2) * (180/pi) degrees.
        let w = Wcs::new(params(Projection::Ait, [0.0, 0.0], identity_cd(1.0))).expect("wcs");
        let edge = 2.0 * core::f64::consts::SQRT_2 * R2D;
        let (ra, dec) = w.pixel_to_world(100.5 + edge, 50.5).expect("domain");
        assert!((ra - 180.0).abs() < 1e-8, "ra={ra}");
        assert!(dec.abs() < 1e-8, "dec={dec}");
    }

    #[test]
    fn round_trip_pixel_sky_pixel() {
        let cases = [
            (Projection::Tan, [83.633_08, 22.014_46], None, None),
            (Projection::Tan, [250.0, -60.0], None, None),
            (Projection::Sin, [10.0, 5.0], None, None),
            (Projection::Ait, [0.0, 0.0], None, None),
            (Projection::Ait, [180.0, -40.0], None, None),
            (Projection::Tan, [45.0, 80.0], Some(0.0), None),
        ];
        for (proj, crval, lonpole, latpole) in cases {
            let mut p = params(proj, crval, [[-0.0001, 0.00002], [0.00003, 0.0002]]);
            p.lonpole = lonpole;
            p.latpole = latpole;
            let w = Wcs::new(p).expect("wcs");
            for &(px, py) in &[(100.5, 50.5), (120.0, 60.0), (80.0, 30.0), (150.0, 90.0)] {
                let Some((ra, dec)) = w.pixel_to_world(px, py) else {
                    continue;
                };
                let (bx, by) = w.world_to_pixel(ra, dec).expect("inverse domain");
                assert!((bx - px).abs() < 1e-8, "{proj:?} x {bx} vs {px}");
                assert!((by - py).abs() < 1e-8, "{proj:?} y {by} vs {py}");
            }
        }
    }

    #[test]
    fn round_trip_over_sky_grid_for_each_projection() {
        // Sweep world coordinates and check that world -> pixel -> world returns the input.
        for proj in [Projection::Tan, Projection::Sin, Projection::Ait] {
            let w = Wcs::new(params(proj, [150.0, 2.0], identity_cd(0.05))).expect("wcs");
            for ra_i in 0..12 {
                for dec_i in 0..7 {
                    let ra = 130.0 + 4.0 * f64::from(ra_i);
                    let dec = -20.0 + 4.0 * f64::from(dec_i);
                    let Some((px, py)) = w.world_to_pixel(ra, dec) else {
                        continue;
                    };
                    let (ra2, dec2) = w.pixel_to_world(px, py).expect("domain");
                    let sep = separation_deg((ra, dec), (ra2, dec2));
                    assert!(sep < 1e-9, "{proj:?} sep={sep} at ({ra},{dec})");
                }
            }
        }
    }

    #[test]
    fn tan_rejects_far_hemisphere() {
        let w = Wcs::new(params(Projection::Tan, [0.0, 0.0], identity_cd(0.01))).expect("wcs");
        // A point 120 degrees from CRVAL is on the far side of a tangent plane.
        assert!(w.world_to_pixel(120.0, 0.0).is_none());
    }

    #[test]
    fn header_parsing_reads_keywords_and_uses_pc_with_cdelt() {
        let header = "\
CRPIX1  =                 100.5 / reference pixel
CRPIX2  =                  50.5
CRVAL1  =              83.63308 / deg
CRVAL2  =              22.01446
CDELT1  =              -0.00020
CDELT2  =               0.00020
CTYPE1  = 'RA---TAN'
CTYPE2  = 'DEC--TAN'
PC1_1   =                   1.0
PC1_2   =                   0.0
PC2_1   =                   0.0
PC2_2   =                   1.0
";
        let w = Wcs::from_header(header).expect("header");
        let (ra, dec) = w.crval_deg();
        assert!((ra - 83.63308).abs() < 1e-12);
        assert!((dec - 22.01446).abs() < 1e-12);
        let (px, py) = w.world_to_pixel(83.63308, 22.01446).expect("ref");
        assert!((px - 100.5).abs() < 1e-9 && (py - 50.5).abs() < 1e-9);
    }

    #[test]
    fn cd_takes_precedence_over_pc_and_cdelt() {
        let header = "CRPIX1  = 1.0\nCRPIX2  = 1.0\nCDELT1  = 99.0\nCDELT2  = 99.0\n\
CTYPE1  = 'RA---SIN'\nCTYPE2  = 'DEC--SIN'\nCD1_1   = 0.5\nCD1_2   = 0.0\nCD2_1   = 0.0\nCD2_2   = 0.5\n";
        let w = Wcs::from_header(header).expect("header");
        // Two pixels in x at 0.5 deg/pixel is a 1 deg intermediate offset. For SIN
        // at CRVAL (0,0) that is a great-circle separation of exactly 1 deg. If PC
        // and CDELT (99 deg/pixel) were wrongly used, the separation would be far larger.
        // For SIN, sin(d) equals the intermediate radius in radians: R = 1 deg in radians.
        let (ra_b, dec_b) = w.pixel_to_world(3.0, 1.0).expect("b");
        let sep = separation_deg((ra_b, dec_b), (0.0, 0.0));
        let expected = (1.0_f64.to_radians()).asin().to_degrees();
        assert!(
            (sep - expected).abs() < 1e-9,
            "sep={sep} expected={expected}"
        );
    }

    #[test]
    fn header_rejects_unknown_projection_and_missing_ctype() {
        let bad = "CTYPE1  = 'RA---ZEA'\nCTYPE2  = 'DEC--ZEA'\n";
        assert!(matches!(
            Wcs::from_header(bad),
            Err(WcsError::UnsupportedProjection(_))
        ));
        let missing = "CRVAL1 = 1.0\n";
        assert!(matches!(
            Wcs::from_header(missing),
            Err(WcsError::MissingKeyword("CTYPE1"))
        ));
        let mismatch = "CTYPE1  = 'RA---TAN'\nCTYPE2  = 'DEC--SIN'\n";
        assert!(matches!(
            Wcs::from_header(mismatch),
            Err(WcsError::MismatchedProjection)
        ));
    }

    #[test]
    fn singular_matrix_is_rejected() {
        let p = params(Projection::Tan, [0.0, 0.0], [[1.0, 2.0], [2.0, 4.0]]);
        assert_eq!(Wcs::new(p).err(), Some(WcsError::SingularMatrix));
    }

    #[test]
    fn fixed_cards_parse_as_80_column_block() {
        let card1 = format!("{:<80}", "CRPIX1  =                 10.0");
        let card2 = format!("{:<80}", "CTYPE1  = 'RA---AIT'");
        let card3 = format!("{:<80}", "CTYPE2  = 'DEC--AIT'");
        let block = format!("{card1}{card2}{card3}");
        let w = Wcs::from_header(&block).expect("block");
        let (px, _) = w.world_to_pixel(0.0, 0.0).expect("ref");
        assert!((px - 10.0).abs() < 1e-9);
    }
}
