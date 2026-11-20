//! World Coordinate System (WCS) for celestial images: FITS header keywords,
//! linear pixel transform, SIP distortion, the zenithal projections ARC, ZEA,
//! TAN, SIN and STG, the cylindrical and pseudocylindrical projections CAR,
//! MOL, AIT and HPX, and the native-to-celestial spherical rotation with
//! LONPOLE and LATPOLE.
//!
//! Conventions follow Calabretta & Greisen (2002), "Representations of
//! celestial coordinates in FITS" (Paper II), for the projections and the
//! spherical rotation. HPX follows Calabretta & Roukema (2007), "Mapping on the
//! HEALPix grid". SIP follows Shupe et al. (2005). Pixel coordinates are FITS
//! 1-based. Angles in the header are in degrees.
//!
//! Scope and limits:
//! - Celestial axes only (`RA`/`DEC`-style CTYPE pairs, or any pair whose
//!   projection code is one of the supported codes). The output is the lon/lat
//!   pair in whatever system the header names; no system conversion is applied.
//! - Only the primary (blank-suffix) WCS description is read. Alternate
//!   descriptions (`WCSNAMEa` and so on) are not.
//! - SIP distortion is applied when `A_ORDER` and `B_ORDER` are present. When
//!   `AP_ORDER` and `BP_ORDER` are also present, the world-to-pixel direction
//!   uses the AP/BP polynomials; otherwise it is solved by Newton iteration on
//!   the forward polynomials.
//! - The HPX projection is implemented for the standard HEALPix parameters
//!   `PV2_1 = 4` and `PV2_2 = 3`; other values are rejected.
//! - TPV and other distortion conventions are not supported.
//! - Validation: the tests check analytic identities (the reference pixel maps
//!   to CRVAL, boundary points, the SIP composition identity) and round trips.
//!   Agreement with wcslib (via Astropy) is checked in the Python validation
//!   suite; see `docs/validation.md`.

use core::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, SQRT_2, TAU};
use core::fmt;

/// Degrees per radian.
const R2D: f64 = 180.0 / PI;

/// Tolerance in radians used when testing whether a native or intermediate
/// coordinate lies on the boundary of a projection's domain.
const DOMAIN_TOL: f64 = 1e-12;

/// Maximum Newton iterations for the SIP world-to-pixel inverse.
const SIP_MAX_ITER: usize = 50;

/// HPX equatorial-zone slope: `y = (3 pi / 8) sin(theta)` in radians.
const HPX_EQ_SLOPE: f64 = 3.0 * PI / 8.0;

/// Supported projections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    /// Zenithal equidistant (ARC): `R = 90 deg - theta`.
    Arc,
    /// Zenithal equal-area (ZEA): `R = 2 (180/pi) sin((90 deg - theta) / 2)`.
    Zea,
    /// Gnomonic (TAN): projection from the centre of the sphere onto a tangent plane.
    Tan,
    /// Orthographic (SIN, no slant): projection along the axis onto a tangent plane.
    Sin,
    /// Stereographic (STG): `R = 2 (180/pi) tan((90 deg - theta) / 2)`.
    Stg,
    /// Hammer-Aitoff (AIT): equal-area pseudocylindrical projection.
    Ait,
    /// Plate carree (CAR): `x = phi`, `y = theta`.
    Car,
    /// Mollweide (MOL): equal-area pseudocylindrical projection.
    Mol,
    /// HEALPix (HPX) with the standard parameters `H = 4`, `K = 3`.
    Hpx,
}

impl Projection {
    /// Parses the three-letter projection code, for example `TAN` from `RA---TAN`.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code.trim() {
            "ARC" => Some(Self::Arc),
            "ZEA" => Some(Self::Zea),
            "TAN" => Some(Self::Tan),
            "SIN" => Some(Self::Sin),
            "STG" => Some(Self::Stg),
            "AIT" => Some(Self::Ait),
            "CAR" => Some(Self::Car),
            "MOL" => Some(Self::Mol),
            "HPX" => Some(Self::Hpx),
            _ => None,
        }
    }

    /// Native latitude of the reference point, `theta_0`, in degrees.
    ///
    /// Zenithal projections use the pole (90 degrees). Cylindrical,
    /// pseudocylindrical and HEALPix use the equator (0 degrees), as in
    /// Paper II Table 1 and Calabretta & Roukema (2007).
    fn native_reference_lat_deg(self) -> f64 {
        match self {
            Self::Arc | Self::Zea | Self::Tan | Self::Sin | Self::Stg => 90.0,
            Self::Ait | Self::Car | Self::Mol | Self::Hpx => 0.0,
        }
    }

    /// Forward projection: native `(phi, theta)` in radians to intermediate
    /// `(x, y)` in degrees. Returns `None` outside the projection's domain.
    fn forward(self, phi: f64, theta: f64) -> Option<(f64, f64)> {
        match self {
            Self::Arc => {
                // Paper II zenithal: R_theta = 90 deg - theta (equidistant).
                if theta < -FRAC_PI_2 {
                    return None;
                }
                let r = (FRAC_PI_2 - theta) * R2D;
                Some((r * phi.sin(), -r * phi.cos()))
            }
            Self::Zea => {
                // Paper II zenithal: R_theta = 2 sin((90 deg - theta) / 2), in radians.
                let r = 2.0 * R2D * ((FRAC_PI_2 - theta) / 2.0).sin();
                Some((r * phi.sin(), -r * phi.cos()))
            }
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
            Self::Stg => {
                // Paper II zenithal: R_theta = 2 tan((90 deg - theta) / 2); the
                // antipode of the pole has infinite radius and is excluded.
                if theta <= -FRAC_PI_2 {
                    return None;
                }
                let r = 2.0 * R2D * ((FRAC_PI_2 - theta) / 2.0).tan();
                Some((r * phi.sin(), -r * phi.cos()))
            }
            Self::Ait => {
                let half = phi / 2.0;
                let s = (1.0 + theta.cos() * half.cos()).sqrt();
                if s <= 0.0 {
                    return None;
                }
                let x = 2.0 * SQRT_2 * theta.cos() * half.sin() / s * R2D;
                let y = SQRT_2 * theta.sin() / s * R2D;
                Some((x, y))
            }
            Self::Car => Some((phi * R2D, theta * R2D)),
            Self::Mol => {
                // Paper II pseudocylindrical: x = (2 sqrt(2) / pi) phi cos(gamma),
                // y = sqrt(2) sin(gamma), where 2 gamma + sin(2 gamma) = pi sin(theta).
                let gamma = mollweide_aux_angle(theta);
                let x = 2.0 * SQRT_2 / PI * phi * gamma.cos() * R2D;
                let y = SQRT_2 * gamma.sin() * R2D;
                Some((x, y))
            }
            Self::Hpx => hpx_forward(phi, theta),
        }
    }

    /// Inverse projection: intermediate `(x, y)` in degrees to native
    /// `(phi, theta)` in radians. Returns `None` outside the domain.
    fn inverse(self, x: f64, y: f64) -> Option<(f64, f64)> {
        match self {
            Self::Arc => {
                let r = (x * x + y * y).sqrt() / R2D;
                if r > PI + DOMAIN_TOL {
                    return None;
                }
                Some((x.atan2(-y), FRAC_PI_2 - r))
            }
            Self::Zea => {
                let r = (x * x + y * y).sqrt() / R2D;
                if r > 2.0 + DOMAIN_TOL {
                    return None;
                }
                let zeta = 2.0 * (r / 2.0).min(1.0).asin();
                Some((x.atan2(-y), FRAC_PI_2 - zeta))
            }
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
            Self::Stg => {
                let r = (x * x + y * y).sqrt() / R2D;
                Some((x.atan2(-y), FRAC_PI_2 - 2.0 * (r / 2.0).atan()))
            }
            Self::Ait => {
                let xr = x / R2D;
                let yr = y / R2D;
                let z2 = 1.0 - (xr / 4.0).powi(2) - (yr / 2.0).powi(2);
                // The Hammer domain is xr^2/8 + yr^2/2 <= 1, which is z2 >= 1/2.
                // Native phi is in (-pi, pi], so cos(theta) cos(phi/2) = 2 z2 - 1
                // must be non-negative; a smaller value lies outside the domain.
                // A small tolerance keeps points exactly on the edge in the domain.
                let cy = 2.0 * z2 - 1.0;
                if cy < -1e-12 {
                    return None;
                }
                let z = z2.sqrt();
                // cos(theta) sin(phi/2) = xr z / 2 and cos(theta) cos(phi/2) = 2 z^2 - 1.
                let cx = xr * z / 2.0;
                let theta = (yr * z).atan2(cx.hypot(cy));
                let phi = 2.0 * cx.atan2(cy);
                Some((phi, theta))
            }
            Self::Car => {
                let yr = y / R2D;
                if yr.abs() > FRAC_PI_2 + DOMAIN_TOL {
                    return None;
                }
                Some((x / R2D, yr))
            }
            Self::Mol => {
                let xr = x / R2D;
                let yr = y / R2D;
                let s = yr / SQRT_2;
                if s.abs() > 1.0 + DOMAIN_TOL {
                    return None;
                }
                let gamma = s.clamp(-1.0, 1.0).asin();
                let sin_theta = ((2.0 * gamma + (2.0 * gamma).sin()) / PI).clamp(-1.0, 1.0);
                let theta = sin_theta.asin();
                let cos_gamma = gamma.cos();
                // At the poles cos(gamma) vanishes and only x = 0 is in the domain.
                let phi = if cos_gamma < 1e-12 {
                    if xr.abs() > 1e-9 {
                        return None;
                    }
                    0.0
                } else {
                    PI * xr / (2.0 * SQRT_2 * cos_gamma)
                };
                if phi.abs() > PI + DOMAIN_TOL {
                    return None;
                }
                Some((phi, theta))
            }
            Self::Hpx => hpx_inverse(x / R2D, y / R2D),
        }
    }
}

/// Solves `2 gamma + sin(2 gamma) = pi sin(theta)` for the Mollweide auxiliary
/// angle `gamma` (Paper II pseudocylindrical section), by Newton iteration.
fn mollweide_aux_angle(theta: f64) -> f64 {
    if theta.abs() >= FRAC_PI_2 {
        return theta;
    }
    let target = PI * theta.sin();
    let mut gamma = theta;
    for _ in 0..100 {
        let f = 2.0 * gamma + (2.0 * gamma).sin() - target;
        let df = 2.0 + 2.0 * (2.0 * gamma).cos();
        if df.abs() < 1e-300 {
            break;
        }
        let step = f / df;
        gamma -= step;
        if step.abs() < 1e-15 {
            break;
        }
    }
    gamma
}

/// HEALPix facet centre in native longitude for `x` (radians): the centre of
/// the polar facet `[k pi/2, (k+1) pi/2)`, with `k` clamped to `[-2, 1]`.
fn hpx_facet_centre(x: f64) -> f64 {
    let k = (x / FRAC_PI_2).floor().clamp(-2.0, 1.0);
    (k + 0.5) * FRAC_PI_2
}

/// HEALPix forward projection (H = 4, K = 3), native to intermediate, in degrees.
///
/// Equatorial zone `|sin theta| <= 2/3`: `x = phi`, `y = (3 pi / 8) sin theta`.
/// Polar caps: `sigma = sqrt(3 (1 - |sin theta|))`, `y = sign(theta) (pi/2 - (pi/4) sigma)`,
/// `x = c + (phi - c) sigma`, where `c` is the facet centre.
fn hpx_forward(phi: f64, theta: f64) -> Option<(f64, f64)> {
    let eta = theta.sin();
    let abs_eta = eta.abs();
    if abs_eta <= 2.0 / 3.0 {
        Some((phi * R2D, HPX_EQ_SLOPE * eta * R2D))
    } else {
        let sigma = (3.0 * (1.0 - abs_eta)).sqrt();
        let c = hpx_facet_centre(phi);
        let x = c + (phi - c) * sigma;
        let y = eta.signum() * (FRAC_PI_2 - FRAC_PI_4 * sigma);
        Some((x * R2D, y * R2D))
    }
}

/// HEALPix inverse projection (H = 4, K = 3), intermediate to native, in radians.
fn hpx_inverse(x: f64, y: f64) -> Option<(f64, f64)> {
    let ay = y.abs();
    if ay <= FRAC_PI_4 {
        if x.abs() > PI + DOMAIN_TOL {
            return None;
        }
        let eta = (y / HPX_EQ_SLOPE).clamp(-2.0 / 3.0, 2.0 / 3.0);
        Some((x, eta.asin()))
    } else {
        if ay > FRAC_PI_2 + DOMAIN_TOL {
            return None;
        }
        let sigma = (2.0 - 4.0 * ay / PI).clamp(0.0, 1.0);
        let eta_abs = 1.0 - sigma * sigma / 3.0;
        let theta = y.signum() * eta_abs.asin();
        let c = hpx_facet_centre(x);
        let d = x - c;
        if d.abs() > FRAC_PI_4 * sigma + DOMAIN_TOL {
            return None;
        }
        // At the pole (sigma = 0) the facet centre is the only valid x.
        let phi = if sigma > 1e-12 { c + d / sigma } else { c };
        Some((phi, theta))
    }
}

/// Errors from header parsing and WCS construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WcsError {
    /// A required keyword is missing from the header.
    MissingKeyword(&'static str),
    /// The projection code is not one of the supported codes.
    UnsupportedProjection(String),
    /// A projection parameter has a value this crate does not implement.
    UnsupportedParameter(&'static str),
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
            Self::UnsupportedParameter(p) => write!(f, "unsupported projection parameter {p}"),
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
    /// Projection parameters `PV2_1` and `PV2_2` (used by HPX as H and K),
    /// or `None` for the default.
    pub pv2: [Option<f64>; 2],
}

/// One SIP polynomial `sum c_pq u^p v^q`, stored as `(p, q, c)` terms.
#[derive(Debug, Clone, PartialEq, Default)]
struct SipPoly {
    terms: Vec<(i32, i32, f64)>,
}

impl SipPoly {
    fn eval(&self, u: f64, v: f64) -> f64 {
        self.terms
            .iter()
            .map(|&(p, q, c)| c * u.powi(p) * v.powi(q))
            .sum()
    }

    /// Partial derivatives `(d/du, d/dv)` of the polynomial.
    fn grad(&self, u: f64, v: f64) -> (f64, f64) {
        let mut du = 0.0;
        let mut dv = 0.0;
        for &(p, q, c) in &self.terms {
            if p > 0 {
                du += c * f64::from(p) * u.powi(p - 1) * v.powi(q);
            }
            if q > 0 {
                dv += c * f64::from(q) * u.powi(p) * v.powi(q - 1);
            }
        }
        (du, dv)
    }
}

/// SIP distortion: forward polynomials `A`, `B` (pixel to intermediate pixel)
/// and optional inverse polynomials `AP`, `BP` (intermediate to pixel).
#[derive(Debug, Clone, PartialEq)]
struct Sip {
    a: SipPoly,
    b: SipPoly,
    inverse: Option<(SipPoly, SipPoly)>,
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
    sip: Option<Sip>,
}

impl Wcs {
    /// Builds a WCS from explicit parameters.
    ///
    /// # Errors
    ///
    /// Returns an error if the linear matrix is singular, LATPOLE cannot be
    /// derived, or an HPX parameter other than the standard `H = 4`, `K = 3`
    /// is requested.
    pub fn new(params: WcsParams) -> Result<Self> {
        if params.projection == Projection::Hpx {
            let h = params.pv2[0].unwrap_or(4.0);
            let k = params.pv2[1].unwrap_or(3.0);
            if h != 4.0 || k != 3.0 {
                return Err(WcsError::UnsupportedParameter("PV2_1/PV2_2 (HPX H and K)"));
            }
        }
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
            sip: None,
        })
    }

    /// Reads the primary WCS from FITS header text.
    ///
    /// The text may be newline-separated lines or a block of 80-character
    /// cards. Recognised keywords: `CRPIXn`, `CRVALn`, `CDELTn`, `CTYPEn`,
    /// `PCi_j`, `CDi_j`, `PV2_1`, `PV2_2`, `LONPOLE`, `LATPOLE`, and the SIP
    /// keywords `A_ORDER`, `B_ORDER`, `A_p_q`, `B_p_q`, `AP_ORDER`, `BP_ORDER`,
    /// `AP_p_q`, `BP_p_q`. Missing `CRPIX` and `CRVAL` default to 0, `CDELT` to
    /// 1, `PC` to the identity. If any `CD` keyword is present, `CD` is used and
    /// `PC`/`CDELT` are ignored, as in FITS WCS. A `-SIP` suffix on `CTYPE` is
    /// accepted and removed.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing `CTYPE1`/`CTYPE2`, an unsupported
    /// projection, mismatched projection codes, an unsupported projection
    /// parameter, or a singular matrix.
    ///
    /// # Examples
    ///
    /// ```
    /// use tpt_planetary_astro::wcs::Wcs;
    ///
    /// let header = "CRPIX1  = 0.5\nCRPIX2  = 0.5\nCRVAL1  = 0.0\nCRVAL2  = 90.0\n\
    /// CDELT1  = -0.1\nCDELT2  = 0.1\nCTYPE1  = 'RA---ZEA'\nCTYPE2  = 'DEC--ZEA'\n";
    /// let wcs = Wcs::from_header(header).unwrap();
    /// // The reference pixel maps to the reference point, the north pole here.
    /// let (_, dec) = wcs.pixel_to_world(0.5, 0.5).unwrap();
    /// assert!((dec - 90.0).abs() < 1e-9);
    /// ```
    pub fn from_header(text: &str) -> Result<Self> {
        let (params, sip) = parse_header(text)?;
        let mut wcs = Self::new(params)?;
        wcs.sip = sip;
        Ok(wcs)
    }

    /// Converts a FITS 1-based pixel position to celestial `(lon, lat)` in degrees.
    ///
    /// Applies the SIP forward polynomials first when present.
    /// Returns `None` if the position lies outside the projection's domain.
    #[must_use]
    pub fn pixel_to_world(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let dx = x - self.crpix[0];
        let dy = y - self.crpix[1];
        let (u, v) = match &self.sip {
            Some(sip) => (dx + sip.a.eval(dx, dy), dy + sip.b.eval(dx, dy)),
            None => (dx, dy),
        };
        let ix = self.cd[0][0] * u + self.cd[0][1] * v;
        let iy = self.cd[1][0] * u + self.cd[1][1] * v;
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
        let (u, v) = self.intermediate_to_offset(ix, iy)?;
        Some((u + self.crpix[0], v + self.crpix[1]))
    }

    /// Intermediate `(x, y)` in degrees to pixel offsets from CRPIX, undoing
    /// SIP distortion.
    ///
    /// With AP/BP present, the inverse polynomials are applied to the linear
    /// offsets. Without them, the forward SIP map `(u, v) -> CD (u + A, v + B)`
    /// is inverted by Newton iteration started from the linear solution.
    fn intermediate_to_offset(&self, ix: f64, iy: f64) -> Option<(f64, f64)> {
        let u0 = self.cd_inv[0][0] * ix + self.cd_inv[0][1] * iy;
        let v0 = self.cd_inv[1][0] * ix + self.cd_inv[1][1] * iy;
        let Some(sip) = &self.sip else {
            return Some((u0, v0));
        };
        if let Some((ap, bp)) = &sip.inverse {
            return Some((u0 + ap.eval(u0, v0), v0 + bp.eval(u0, v0)));
        }
        let (mut u, mut v) = (u0, v0);
        for _ in 0..SIP_MAX_ITER {
            let fu = u + sip.a.eval(u, v);
            let fv = v + sip.b.eval(u, v);
            let ex = self.cd[0][0] * fu + self.cd[0][1] * fv - ix;
            let ey = self.cd[1][0] * fu + self.cd[1][1] * fv - iy;
            let (au, av) = sip.a.grad(u, v);
            let (bu, bv) = sip.b.grad(u, v);
            // Jacobian J = CD * [[1 + au, av], [bu, 1 + bv]].
            let m = [[1.0 + au, av], [bu, 1.0 + bv]];
            let j00 = self.cd[0][0] * m[0][0] + self.cd[0][1] * m[1][0];
            let j01 = self.cd[0][0] * m[0][1] + self.cd[0][1] * m[1][1];
            let j10 = self.cd[1][0] * m[0][0] + self.cd[1][1] * m[1][0];
            let j11 = self.cd[1][0] * m[0][1] + self.cd[1][1] * m[1][1];
            let det = j00 * j11 - j01 * j10;
            if det == 0.0 || !det.is_finite() {
                return None;
            }
            let du = (j11 * ex - j01 * ey) / det;
            let dv = (-j10 * ex + j00 * ey) / det;
            u -= du;
            v -= dv;
            if du.abs() < 1e-12 && dv.abs() < 1e-12 {
                break;
            }
        }
        Some((u, v))
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
    // The keyword ends at the first '='. FITS pads it to 8 columns, but hand-written
    // headers often do not, so the padding is not required.
    let eq = card.find('=')?;
    let key = card.get(..eq)?.trim();
    if key.is_empty() || key.len() > 8 {
        return None;
    }
    let rest = card.get(eq + 1..)?.trim_start();
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

/// Parses a SIP `p_q` exponent pair, for example `2_0` from `A_2_0`.
fn parse_sip_pq(s: &str) -> Option<(i32, i32)> {
    let (a, b) = s.split_once('_')?;
    let p: i32 = a.parse().ok()?;
    let q: i32 = b.parse().ok()?;
    if p >= 0 && q >= 0 { Some((p, q)) } else { None }
}

/// Splits a SIP keyword into its polynomial name (`A`, `B`, `AP`, `BP`) and the
/// remainder (`ORDER` or `p_q`).
fn parse_sip_key(key: &str) -> Option<(&'static str, &str)> {
    for (prefix, name) in [("AP_", "AP"), ("BP_", "BP"), ("A_", "A"), ("B_", "B")] {
        if let Some(rest) = key.strip_prefix(prefix) {
            return Some((name, rest));
        }
    }
    None
}

/// Accumulates SIP keywords while the header is read.
#[derive(Default)]
struct SipBuild {
    a_order: bool,
    b_order: bool,
    ap_order: bool,
    bp_order: bool,
    a: SipPoly,
    b: SipPoly,
    ap: SipPoly,
    bp: SipPoly,
}

impl SipBuild {
    fn apply(&mut self, name: &str, rest: &str, number: Option<f64>) {
        if rest == "ORDER" {
            if number.is_some() {
                match name {
                    "A" => self.a_order = true,
                    "B" => self.b_order = true,
                    "AP" => self.ap_order = true,
                    _ => self.bp_order = true,
                }
            }
            return;
        }
        if let (Some(c), Some((p, q))) = (number, parse_sip_pq(rest)) {
            let poly = match name {
                "A" => &mut self.a,
                "B" => &mut self.b,
                "AP" => &mut self.ap,
                _ => &mut self.bp,
            };
            poly.terms.push((p, q, c));
        }
    }

    /// SIP is active when `A_ORDER` and `B_ORDER` are present; the inverse
    /// polynomials are used when `AP_ORDER` and `BP_ORDER` are also present.
    fn finish(self) -> Option<Sip> {
        if !(self.a_order && self.b_order) {
            return None;
        }
        let inverse = (self.ap_order && self.bp_order).then_some((self.ap, self.bp));
        Some(Sip {
            a: self.a,
            b: self.b,
            inverse,
        })
    }
}

fn parse_header(text: &str) -> Result<(WcsParams, Option<Sip>)> {
    let mut crpix = [0.0; 2];
    let mut crval = [0.0; 2];
    let mut cdelt = [1.0; 2];
    let mut pc = [[1.0, 0.0], [0.0, 1.0]];
    let mut cd = [[0.0; 2]; 2];
    let mut have_cd = false;
    let mut ctype: [Option<String>; 2] = [None, None];
    let mut lonpole = None;
    let mut latpole = None;
    let mut pv2 = [None, None];
    let mut sip = SipBuild::default();

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
                } else if let Some(idx) = key
                    .strip_prefix("PV2_")
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    if let (Some(n), 1..=2) = (number, idx) {
                        pv2[idx - 1] = Some(n);
                    }
                } else if let Some((name, rest)) = parse_sip_key(key) {
                    sip.apply(name, rest, number);
                } else if let Some(pair) = key.strip_prefix("PC").and_then(parse_pair) {
                    if let Some(n) = number {
                        pc[pair.0][pair.1] = n;
                    }
                } else if let Some(pair) = key.strip_prefix("CD").and_then(parse_pair)
                    && let Some(n) = number
                {
                    cd[pair.0][pair.1] = n;
                    have_cd = true;
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

    let params = WcsParams {
        crpix,
        crval,
        cd: linear,
        projection,
        lonpole,
        latpole,
        pv2,
    };
    Ok((params, sip.finish()))
}

fn projection_code(ctype: Option<&str>, keyword: &'static str) -> Result<String> {
    let ctype = ctype.ok_or(WcsError::MissingKeyword(keyword))?.trim();
    // A SIP header names the axes "RA---TAN-SIP"; the suffix is not a projection code.
    let base = ctype.strip_suffix("-SIP").unwrap_or(ctype);
    // CTYPE has the form "RA---TAN" or "DEC--SIN": the projection code follows the last '-'.
    let code = base.rsplit('-').next().unwrap_or("").trim();
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
            pv2: [None, None],
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

    const ALL: [Projection; 9] = [
        Projection::Arc,
        Projection::Zea,
        Projection::Tan,
        Projection::Sin,
        Projection::Stg,
        Projection::Ait,
        Projection::Car,
        Projection::Mol,
        Projection::Hpx,
    ];

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
    fn every_projection_reference_pixel_returns_crval() {
        for proj in ALL {
            for crval in [[120.0, 35.0], [0.0, 0.0], [200.0, -60.0]] {
                let w = Wcs::new(params(proj, crval, identity_cd(0.01))).expect("wcs");
                let (ra, dec) = w.pixel_to_world(100.5, 50.5).expect("domain");
                assert!(angle_diff_deg(ra, crval[0]) < 1e-9, "{proj:?} ra={ra}");
                assert!((dec - crval[1]).abs() < 1e-9, "{proj:?} dec={dec}");
                let (px, py) = w.world_to_pixel(crval[0], crval[1]).expect("ref");
                assert!(
                    (px - 100.5).abs() < 1e-8 && (py - 50.5).abs() < 1e-8,
                    "{proj:?}"
                );
            }
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
    fn arc_radius_equals_zenith_distance() {
        // ARC: R = 90 deg - d. A radius of 60 deg from CRVAL is d = 60 deg.
        let w = Wcs::new(params(Projection::Arc, [200.0, 30.0], identity_cd(1.0))).expect("wcs");
        let (ra, dec) = w.pixel_to_world(100.5, 50.5 + 60.0).expect("domain");
        let d = separation_deg((ra, dec), (200.0, 30.0));
        assert!((d - 60.0).abs() < 1e-9, "d={d}");
    }

    #[test]
    fn arc_boundary_at_antipode_and_rejects_beyond() {
        // ARC covers the whole sphere: R = 180 deg is the antipode of CRVAL.
        let w = Wcs::new(params(Projection::Arc, [0.0, 0.0], identity_cd(1.0))).expect("wcs");
        let (ra, dec) = w.pixel_to_world(100.5 + 180.0, 50.5).expect("edge");
        assert!(
            (ra - 180.0).abs() < 1e-8 && dec.abs() < 1e-8,
            "({ra},{dec})"
        );
        assert!(w.pixel_to_world(100.5 + 181.0, 50.5).is_none());
    }

    #[test]
    fn zea_radius_equals_two_sin_half_zenith_distance() {
        // ZEA: R = 2 sin(z/2) in radians. z = 90 deg gives R = sqrt(2) rad.
        let w = Wcs::new(params(Projection::Zea, [0.0, 0.0], identity_cd(1.0))).expect("wcs");
        let r_deg = (2.0 * (std::f64::consts::FRAC_PI_4).sin()).to_degrees();
        let (ra, dec) = w.pixel_to_world(100.5, 50.5 + r_deg).expect("domain");
        let d = separation_deg((ra, dec), (0.0, 0.0));
        assert!((d - 90.0).abs() < 1e-8, "d={d}");
    }

    #[test]
    fn zea_boundary_at_antipode_and_rejects_beyond() {
        // ZEA radius reaches 2 rad (360/pi deg) at the antipode of CRVAL.
        let w = Wcs::new(params(Projection::Zea, [0.0, 0.0], identity_cd(1.0))).expect("wcs");
        let edge = 2.0 * R2D;
        let (ra, dec) = w.pixel_to_world(100.5, 50.5 + edge).expect("edge");
        let d = separation_deg((ra, dec), (0.0, 0.0));
        assert!((d - 180.0).abs() < 1e-7, "d={d}");
        assert!(w.pixel_to_world(100.5, 50.5 + edge + 1.0).is_none());
    }

    #[test]
    fn stg_radius_equals_two_tan_half_zenith_distance() {
        // STG at 90 deg from CRVAL: R = 2 tan(45 deg) = 2 rad.
        let w = Wcs::new(params(Projection::Stg, [0.0, 0.0], identity_cd(1.0))).expect("wcs");
        let (ra, dec) = w.pixel_to_world(100.5, 50.5 + 2.0 * R2D).expect("domain");
        let d = separation_deg((ra, dec), (0.0, 0.0));
        assert!((d - 90.0).abs() < 1e-8, "d={d}");
    }

    #[test]
    fn stg_rejects_antipode_of_crval() {
        let w = Wcs::new(params(Projection::Stg, [0.0, 0.0], identity_cd(1.0))).expect("wcs");
        // The antipode of CRVAL has infinite radius. Rounding may place it either
        // just outside the domain (None) or at a huge finite radius; never near the
        // reference pixel.
        if let Some((px, py)) = w.world_to_pixel(180.0, 0.0) {
            assert!(((px - 100.5).powi(2) + (py - 50.5).powi(2)).sqrt() > 1e6);
        }
        assert!(w.world_to_pixel(179.0, 0.0).is_some());
    }

    #[test]
    fn car_maps_native_angles_linearly() {
        // CAR: x = phi and y = theta in degrees, so CRVAL (0,0) gives a pixel
        // offset equal to the sky offset.
        let w = Wcs::new(params(Projection::Car, [0.0, 0.0], identity_cd(1.0))).expect("wcs");
        let (ra, dec) = w.pixel_to_world(100.5 + 30.0, 50.5 + 20.0).expect("domain");
        assert!(
            (ra - 30.0).abs() < 1e-10 && (dec - 20.0).abs() < 1e-10,
            "({ra},{dec})"
        );
        assert!(w.pixel_to_world(100.5, 50.5 + 90.5).is_none());
    }

    #[test]
    fn mol_equator_edge_is_two_root_two_radians() {
        // MOL at the equator: x = (2 sqrt(2)/pi) phi, so phi = pi gives x = 2 sqrt(2) rad.
        let w = Wcs::new(params(Projection::Mol, [0.0, 0.0], identity_cd(1.0))).expect("wcs");
        let edge = 2.0 * SQRT_2 * R2D;
        let (ra, dec) = w.pixel_to_world(100.5 + edge, 50.5).expect("edge");
        assert!(
            (ra - 180.0).abs() < 1e-8 && dec.abs() < 1e-8,
            "({ra},{dec})"
        );
        assert!(w.pixel_to_world(100.5 + edge + 0.5, 50.5).is_none());
    }

    #[test]
    fn mol_pole_edge_is_root_two_radians() {
        // MOL at the pole: y = sqrt(2) rad at theta = 90 deg.
        let w = Wcs::new(params(Projection::Mol, [0.0, 0.0], identity_cd(1.0))).expect("wcs");
        let edge = SQRT_2 * R2D;
        let (_, dec) = w.pixel_to_world(100.5, 50.5 + edge).expect("pole");
        assert!((dec - 90.0).abs() < 1e-8, "dec={dec}");
        assert!(w.pixel_to_world(100.5, 50.5 + edge + 0.5).is_none());
    }

    #[test]
    fn hpx_equatorial_boundary_is_sin_theta_two_thirds() {
        // HPX: the equatorial zone ends at sin(theta) = 2/3, where y = pi/4.
        let w = Wcs::new(params(Projection::Hpx, [0.0, 0.0], identity_cd(1.0))).expect("wcs");
        let (ra, dec) = w
            .pixel_to_world(100.5 + 0.0, 50.5 + FRAC_PI_4 * R2D)
            .expect("edge");
        assert!(ra.abs() < 1e-9 || (ra - 360.0).abs() < 1e-9, "ra={ra}");
        assert!(
            (dec - (2.0_f64 / 3.0).asin().to_degrees()).abs() < 1e-9,
            "dec={dec}"
        );
    }

    #[test]
    fn right_ascension_folds_rounding_at_360_to_zero() {
        // A pixel a hair west of the reference point gives a right ascension of
        // -1e-15 degrees. rem_euclid rounds 360 - 1e-15 to exactly 360.0, which
        // must come back as 0.0 so the range stays [0, 360).
        let w = Wcs::new(WcsParams {
            crpix: [0.0, 0.0],
            ..params(Projection::Tan, [0.0, 0.0], identity_cd(1.0))
        })
        .expect("wcs");
        let (ra, dec) = w.pixel_to_world(-1e-15, 0.0).expect("in range");
        assert!((0.0..360.0).contains(&ra), "ra={ra}");
        assert!(dec.abs() < 1e-12, "dec={dec}");
    }

    #[test]
    fn hpx_pole_is_polar_facet_centre() {
        // The north pole maps to y = pi/2 at each facet centre x = (k + 1/2) pi/2,
        // so x = 45 deg (the centre of the facet [0, 90) deg) reaches the pole.
        let w = Wcs::new(params(Projection::Hpx, [0.0, 0.0], identity_cd(1.0))).expect("wcs");
        let (_, dec) = w.pixel_to_world(100.5 + 45.0, 50.5 + 90.0).expect("pole");
        assert!((dec - 90.0).abs() < 1e-9, "dec={dec}");
        // Off the facet centre at the pole is outside the domain.
        assert!(w.pixel_to_world(100.5 + 10.0, 50.5 + 90.0).is_none());
    }

    #[test]
    fn hpx_rejects_non_default_hk() {
        let mut p = params(Projection::Hpx, [0.0, 0.0], identity_cd(1.0));
        p.pv2 = [Some(4.0), Some(3.0)];
        assert!(Wcs::new(p.clone()).is_ok());
        p.pv2 = [Some(3.0), Some(3.0)];
        assert!(matches!(
            Wcs::new(p),
            Err(WcsError::UnsupportedParameter(_))
        ));
    }

    #[test]
    fn ait_edge_at_equator_is_two_root_two_radians() {
        // CRVAL (0,0), default LONPOLE 0 and LATPOLE 90: the native equator
        // point at phi = 180 deg maps to the celestial point (180, 0), which
        // sits at the Hammer edge x = 2*sqrt(2) * (180/pi) degrees.
        let w = Wcs::new(params(Projection::Ait, [0.0, 0.0], identity_cd(1.0))).expect("wcs");
        let edge = 2.0 * SQRT_2 * R2D;
        let (ra, dec) = w.pixel_to_world(100.5 + edge, 50.5).expect("domain");
        assert!((ra - 180.0).abs() < 1e-8, "ra={ra}");
        assert!(dec.abs() < 1e-8, "dec={dec}");
    }

    #[test]
    fn ait_inverse_rejects_pixels_outside_hammer_ellipse() {
        // Regression: the inverse used the ellipse x^2/16 + y^2/4 <= 1 (semi-axes
        // 4 and 2 rad), so pixels between the true edge (2*sqrt(2) rad) and 4 rad
        // were mapped to sky positions that project back to other pixels.
        // Reference: astropy.wcs (wcslib) returns NaN for these pixels.
        let w = Wcs::new(params(Projection::Ait, [0.0, 0.0], identity_cd(1.0))).expect("wcs");
        let x_out_deg = 170.0; // 2.967 rad > 2*sqrt(2) rad
        assert!(w.pixel_to_world(100.5 + x_out_deg, 50.5).is_none());
        let y_out_deg = 100.0; // 1.745 rad: y^2/2 > 1
        assert!(w.pixel_to_world(100.5, 50.5 + y_out_deg).is_none());
        // Points just inside the edge still map and round-trip.
        let (lon, lat) = w.pixel_to_world(100.5 + 160.0, 50.5).expect("inside");
        let (px, py) = w.world_to_pixel(lon, lat).expect("forward");
        assert!((px - (100.5 + 160.0)).abs() < 1e-8 && (py - 50.5).abs() < 1e-8);
        let (lon, lat) = w.pixel_to_world(100.5, 50.5 + 80.0).expect("inside");
        let (px, py) = w.world_to_pixel(lon, lat).expect("forward");
        assert!((px - 100.5).abs() < 1e-8 && (py - (50.5 + 80.0)).abs() < 1e-8);
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
    fn round_trip_pixel_sky_pixel_for_every_projection() {
        for proj in ALL {
            for crval in [[0.0, 0.0], [150.0, 40.0], [300.0, -55.0]] {
                let w = Wcs::new(params(proj, crval, identity_cd(0.5))).expect("wcs");
                for i in -6..=6 {
                    for j in -6..=6 {
                        let (px, py) = (100.5 + 17.0 * f64::from(i), 50.5 + 13.0 * f64::from(j));
                        let Some((ra, dec)) = w.pixel_to_world(px, py) else {
                            continue;
                        };
                        let (bx, by) = w.world_to_pixel(ra, dec).expect("inverse domain");
                        assert!(
                            (bx - px).abs() < 1e-7 && (by - py).abs() < 1e-7,
                            "{proj:?} {crval:?} ({px},{py}) -> ({bx},{by})"
                        );
                    }
                }
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
        let bad = "CTYPE1  = 'RA---XYZ'\nCTYPE2  = 'DEC--XYZ'\n";
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
    fn header_accepts_all_new_projection_codes() {
        for code in ["ARC", "ZEA", "STG", "CAR", "MOL", "HPX"] {
            let text = format!("CTYPE1  = 'RA---{code}'\nCTYPE2  = 'DEC--{code}'\n");
            assert!(Wcs::from_header(&text).is_ok(), "{code}");
        }
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

    /// Header with a TAN projection and the given SIP cards appended.
    fn sip_header(extra: &str) -> String {
        format!(
            "CRPIX1  = 512.5\nCRPIX2  = 480.0\nCRVAL1  = 150.0\nCRVAL2  = 20.0\n\
CDELT1  = -0.0002\nCDELT2  = 0.0002\nCTYPE1  = 'RA---TAN-SIP'\nCTYPE2  = 'DEC--TAN-SIP'\n{extra}"
        )
    }

    fn linear_tan_wcs() -> Wcs {
        let text = sip_header("");
        Wcs::from_header(&text).expect("linear")
    }

    #[test]
    fn sip_forward_is_the_linear_map_of_the_distorted_offsets() {
        // SIP forward: the pixel offset (u, v) becomes (u + A(u,v), v + B(u,v))
        // before the linear CD map. So the distorted WCS at p must equal the
        // linear WCS at p + (A, B).
        let extra = "A_ORDER = 3\nB_ORDER = 3\nA_2_0 = 3.0E-6\nA_1_1 = -2.0E-6\nA_0_2 = 1.0E-6\n\
B_2_0 = -1.5E-6\nB_1_1 = 2.5E-6\nB_0_2 = 4.0E-7\nA_3_0 = 1.0E-8\nB_0_3 = -5.0E-9\n";
        let sip = Wcs::from_header(&sip_header(extra)).expect("sip");
        let lin = linear_tan_wcs();
        let (x, y) = (300.25, 210.75);
        let (du, dv): (f64, f64) = (x - 512.5, y - 480.0);
        let a = 3.0e-6 * du * du - 2.0e-6 * du * dv + 1.0e-6 * dv * dv + 1.0e-8 * du.powi(3);
        let b = -1.5e-6 * du * du + 2.5e-6 * du * dv + 4.0e-7 * dv * dv - 5.0e-9 * dv.powi(3);
        let got = sip.pixel_to_world(x, y).expect("sip domain");
        let want = lin.pixel_to_world(x + a, y + b).expect("linear domain");
        assert!(angle_diff_deg(got.0, want.0) < 1e-12, "{got:?} {want:?}");
        assert!((got.1 - want.1).abs() < 1e-12, "{got:?} {want:?}");
    }

    #[test]
    fn sip_world_to_pixel_newton_inverts_forward() {
        // Without AP/BP the world-to-pixel direction is solved by Newton iteration.
        let extra = "A_ORDER = 2\nB_ORDER = 2\nA_2_0 = 4.0E-6\nA_1_1 = 2.0E-6\nB_0_2 = -3.0E-6\n\
B_1_1 = 1.0E-6\n";
        let w = Wcs::from_header(&sip_header(extra)).expect("sip");
        for (x, y) in [
            (512.5, 480.0),
            (100.0, 50.0),
            (900.0, 800.0),
            (512.5 + 40.0, 480.0 - 25.0),
        ] {
            let (ra, dec) = w.pixel_to_world(x, y).expect("domain");
            let (bx, by) = w.world_to_pixel(ra, dec).expect("inverse");
            assert!(
                (bx - x).abs() < 1e-8 && (by - y).abs() < 1e-8,
                "({x},{y}) -> ({bx},{by})"
            );
        }
    }

    #[test]
    fn sip_inverse_polynomials_are_applied_when_present() {
        // Small second-order distortion with matching first-order inverse terms.
        // The truncation error is O(a^2 r^3), far below the tolerance for r < 50 px.
        let a = 1.0e-5;
        let extra = format!(
            "A_ORDER = 2\nB_ORDER = 2\nA_2_0 = {a}\nB_0_2 = {a}\nAP_ORDER = 2\nBP_ORDER = 2\n\
AP_2_0 = {}\nBP_0_2 = {}\n",
            -a, -a
        );
        let w = Wcs::from_header(&sip_header(&extra)).expect("sip");
        for (dx, dy) in [(30.0, -20.0), (-45.0, 10.0), (5.0, 40.0)] {
            let (x, y) = (512.5 + dx, 480.0 + dy);
            let (ra, dec) = w.pixel_to_world(x, y).expect("domain");
            let (bx, by) = w.world_to_pixel(ra, dec).expect("inverse");
            assert!(
                (bx - x).abs() < 1e-3 && (by - y).abs() < 1e-3,
                "({bx},{by}) vs ({x},{y})"
            );
        }
    }

    #[test]
    fn sip_requires_both_orders() {
        // A_ORDER alone does not activate SIP, so the mapping stays linear.
        let with_one = Wcs::from_header(&sip_header("A_ORDER = 2\nA_2_0 = 1.0E-3\n")).expect("ok");
        let lin = linear_tan_wcs();
        let a = with_one.pixel_to_world(600.0, 500.0).expect("d");
        let b = lin.pixel_to_world(600.0, 500.0).expect("d");
        assert!(angle_diff_deg(a.0, b.0) < 1e-14 && (a.1 - b.1).abs() < 1e-14);
    }
}
