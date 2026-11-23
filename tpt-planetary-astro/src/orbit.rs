//! Keplerian elements, Kepler's equation, two-line element sets (TLE) and the
//! SGP4 near-Earth propagator.
//!
//! Contents:
//! - [`Keplerian`]: classical orbital elements with conversion to and from a
//!   Cartesian [`StateVector`] (inertial frame, km and km/s). The anomaly is a
//!   [`Anomaly`] value, either mean or true; a mean anomaly is converted to a
//!   true anomaly by solving Kepler's equation.
//! - [`solve_kepler`], [`mean_to_true_anomaly`], [`true_to_mean_anomaly`]:
//!   Kepler's equation and anomaly conversions for elliptic orbits.
//! - [`Tle`]: parser for the NORAD two-line element format, with checksum
//!   validation.
//! - [`Sgp4`]: the SGP4 near-Earth model (Hoots and Roehrich, Spacetrack Report
//!   No. 3, as revised by Vallado, Crawford, Hujsak and Kelso, 2006), with the
//!   WGS-72 constants. Output is in the TEME frame, in km and km/s.
//!
//! Limits:
//! - Deep-space orbits (period of 225 minutes or more) use the SDP4 lunar-solar
//!   and resonance terms (see `sdp4`), as in the reference `sgp4` package. The
//!   resonance integrator starts from the epoch state at each propagation, so
//!   results do not depend on the order of calls.
//! - Orbit determination (fitting elements to observations) is not
//!   implemented.
//! - The SGP4 `ndot` and `nddot` fields of a TLE are parsed but not used, as in
//!   the reference implementation.
//! - The time scale of an epoch is not tagged. TLE epochs are UTC by
//!   convention; the caller must use the same scale for all inputs.
//!
//! Feature flags: the whole module needs `std`, because the trigonometric and
//! square-root functions are not in `core`. Only the module is gated; the
//! parts that would work without `std` (the error type's data and the
//! checksum) are not separately exposed.
//!
//! Gravitational constants: [`WGS72_MU_KM3_S2`] and [`WGS72_RADIUS_EARTH_KM`]
//! are the WGS-72 values used by SGP4 (Vallado et al., "Revisiting Spacetrack
//! Report #3", AIAA 2006-6753, Table 1; the same values as the `wgs72` set of
//! the Python `sgp4` package). The Keplerian conversions use the same
//! `mu` so that they are consistent with [`Sgp4`].
//!
//! # Example
//!
//! ```
//! use tpt_planetary_astro::orbit::Sgp4;
//!
//! // The first element set of the SGP4 verification file (Vanguard 1, 1958).
//! let line1 = "1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753";
//! let line2 = "2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667";
//! let sat = Sgp4::from_tle_lines(line1, line2)?;
//! let state = sat.propagate(360.0)?;
//! // Position in km (TEME), close to the published reference -7154.031202 km.
//! assert!((state.position_km[0] + 7154.031202).abs() < 1e-5);
//! # Ok::<(), tpt_planetary_astro::orbit::OrbitError>(())
//! ```

use core::f64::consts::{PI, TAU};
use std::fmt;

use tpt_planetary_core::jd::JulianDate;

/// WGS-72 Earth gravitational parameter in km^3/s^2, the value used by SGP4.
///
/// Source: Vallado et al. (2006), Table 1; Python `sgp4`, `getgravconst('wgs72')`.
pub const WGS72_MU_KM3_S2: f64 = 398_600.8;

/// WGS-72 equatorial radius of the Earth in km, the value used by SGP4.
///
/// Source: Vallado et al. (2006), Table 1; Python `sgp4`, `getgravconst('wgs72')`.
pub const WGS72_RADIUS_EARTH_KM: f64 = 6_378.135;

/// WGS-72 second zonal harmonic (un-normalised), used by SGP4.
const WGS72_J2: f64 = 0.001_082_616;
/// WGS-72 third zonal harmonic (un-normalised), used by SGP4.
const WGS72_J3: f64 = -0.000_002_538_81;
/// WGS-72 fourth zonal harmonic (un-normalised), used by SGP4.
const WGS72_J4: f64 = -0.000_001_655_97;

/// Convergence tolerance for Newton's method on Kepler's equation, in radians.
///
/// The iteration stops when the Newton correction is at most this size.
pub const KEPLER_TOLERANCE_RAD: f64 = 1.0e-14;

/// Iteration limit for Newton's method on Kepler's equation.
///
/// Exceeding it returns [`OrbitError::NonConvergence`].
pub const KEPLER_MAX_ITERATIONS: u32 = 50;

/// Eccentricity below which an orbit is treated as circular, and the angle
/// below which a node vector is treated as zero (radians, dimensionless).
const DEGENERACY_EPS: f64 = 1.0e-11;

/// Radians per degree.
const DEG2RAD: f64 = PI / 180.0;

/// Errors returned by the orbit module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OrbitError {
    /// Newton's method on Kepler's equation did not reach
    /// [`KEPLER_TOLERANCE_RAD`] within [`KEPLER_MAX_ITERATIONS`] steps.
    NonConvergence,
    /// Orbital elements are outside the range required by the requested
    /// conversion (for example eccentricity not in `[0, 1)` for an elliptic
    /// conversion, or a non-positive semi-major axis).
    InvalidElements,
    /// A state vector has zero position or zero angular momentum, so the
    /// orbit plane or the anomaly is undefined.
    DegenerateState,
    /// A TLE line is too short, has the wrong line number, or has a field
    /// that cannot be read. The payload is the line number (1 or 2).
    InvalidTle {
        /// Line number of the TLE (1 or 2) in which the error was found.
        line: u8,
    },
    /// The checksum digit in column 69 of a TLE line does not match the
    /// computed checksum. The payload is the line number (1 or 2).
    TleChecksum {
        /// Line number of the TLE (1 or 2) with the bad checksum.
        line: u8,
    },
    /// The satellite catalogue numbers in lines 1 and 2 differ.
    TleSatelliteMismatch,
    /// The mean motion is zero or negative.
    MeanMotionNonPositive,
    /// The eccentricity at the requested time is outside `[0, 1)` (SGP4 error 1
    /// or 3).
    EccentricityOutOfRange,
    /// The semi-latus rectum is negative at the requested time (SGP4 error 4).
    SemiLatusRectumNegative,
    /// The satellite has decayed: the radius at the requested time is below
    /// the Earth's surface (SGP4 error 6).
    Decayed,
}

impl fmt::Display for OrbitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrbitError::NonConvergence => f.write_str("Kepler's equation did not converge"),
            OrbitError::InvalidElements => f.write_str("orbital elements are out of range"),
            OrbitError::DegenerateState => {
                f.write_str("state vector has zero position or zero angular momentum")
            }
            OrbitError::InvalidTle { line } => write!(f, "TLE line {line} is malformed"),
            OrbitError::TleChecksum { line } => write!(f, "TLE line {line} has a bad checksum"),
            OrbitError::TleSatelliteMismatch => {
                f.write_str("TLE lines 1 and 2 have different satellite numbers")
            }
            OrbitError::MeanMotionNonPositive => f.write_str("mean motion is not positive"),
            OrbitError::EccentricityOutOfRange => {
                f.write_str("SGP4 eccentricity is outside the range 0 to 1")
            }
            OrbitError::SemiLatusRectumNegative => {
                f.write_str("SGP4 semi-latus rectum is negative")
            }
            OrbitError::Decayed => f.write_str("satellite has decayed below the Earth's surface"),
        }
    }
}

impl std::error::Error for OrbitError {}

/// Position and velocity in a Cartesian frame, in km and km/s.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StateVector {
    /// Position vector in km.
    pub position_km: [f64; 3],
    /// Velocity vector in km/s.
    pub velocity_km_s: [f64; 3],
}

/// An anomaly, in radians, either mean or true.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Anomaly {
    /// Mean anomaly in radians.
    Mean(f64),
    /// True anomaly in radians.
    True(f64),
}

/// Classical (Keplerian) orbital elements for an elliptic orbit.
///
/// Angles are in radians. The frame is the inertial frame of the state
/// vectors that the elements describe (for example the frame of
/// [`StateVector`]). The gravitational parameter is [`WGS72_MU_KM3_S2`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Keplerian {
    /// Semi-major axis in km (positive for an elliptic orbit).
    pub semi_major_axis_km: f64,
    /// Eccentricity, in `[0, 1)`.
    pub eccentricity: f64,
    /// Inclination in radians, in `[0, pi]`.
    pub inclination_rad: f64,
    /// Right ascension of the ascending node in radians.
    pub raan_rad: f64,
    /// Argument of perigee in radians.
    pub arg_perigee_rad: f64,
    /// Mean or true anomaly in radians.
    pub anomaly: Anomaly,
    /// Epoch of the elements (see the crate notes on time scales).
    pub epoch: JulianDate,
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm3(a: [f64; 3]) -> f64 {
    dot3(a, a).sqrt()
}

fn add3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale3(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// Velocity at the middle of three position vectors on one orbit, by Gibbs'
/// method (Vallado, *Fundamentals of Astrodynamics*, algorithm 52).
///
/// `r1`, `r2`, `r3` are positions in km in the same inertial frame, at three
/// times `t1 < t2 < t3`, and `mu` is the gravitational parameter in km^3/s^2
/// (for example [`WGS72_MU_KM3_S2`]). The result is the velocity at `r2` in
/// km/s, and is exact for an unperturbed two-body orbit. The method needs the
/// three positions to be well separated in angle: for closely spaced
/// observations use [`herrick_gibbs_velocity`] instead.
///
/// Returns [`OrbitError::DegenerateState`] if the positions are coplanar with
/// the origin in a degenerate way (zero cross products) and
/// [`OrbitError::InvalidElements`] for a non-positive `mu`.
pub fn gibbs_velocity(
    r1: [f64; 3],
    r2: [f64; 3],
    r3: [f64; 3],
    mu: f64,
) -> Result<[f64; 3], OrbitError> {
    if mu.is_nan() || mu <= 0.0 || !mu.is_finite() {
        return Err(OrbitError::InvalidElements);
    }
    let (n1, n2, n3) = (norm3(r1), norm3(r2), norm3(r3));
    if n1 == 0.0 || n2 == 0.0 || n3 == 0.0 {
        return Err(OrbitError::DegenerateState);
    }
    let c12 = cross(r1, r2);
    let c23 = cross(r2, r3);
    let c31 = cross(r3, r1);
    let d = add3(add3(c12, c23), c31);
    let nvec = add3(
        add3(scale3(cross(r2, r3), n1), scale3(cross(r3, r1), n2)),
        scale3(cross(r1, r2), n3),
    );
    let svec = add3(
        add3(scale3(r1, n2 - n3), scale3(r2, n3 - n1)),
        scale3(r3, n1 - n2),
    );
    let (nm, dm) = (norm3(nvec), norm3(d));
    if nm == 0.0 || dm == 0.0 || !nm.is_finite() || !dm.is_finite() {
        return Err(OrbitError::DegenerateState);
    }
    let k = (mu / (nm * dm)).sqrt();
    let v2 = add3(scale3(cross(d, r2), 1.0 / n2), svec);
    Ok(scale3(v2, k))
}

/// Velocity at the middle of three position vectors that are close in time,
/// by the Herrick-Gibbs approximation (Vallado, *Fundamentals of
/// Astrodynamics*, algorithm 54).
///
/// `r1`, `r2`, `r3` are positions in km at times `t1 < t2 < t3` in seconds
/// (any common origin), and `mu` is the gravitational parameter in km^3/s^2.
/// The result is the velocity at `r2` in km/s. Unlike [`gibbs_velocity`] this
/// needs no angular separation between the positions, so it suits closely
/// spaced observations. It is an expansion in the time spacing: the error
/// shrinks rapidly as the spacing decreases, and it is not exact for a
/// two-body orbit.
///
/// Returns [`OrbitError::InvalidElements`] for a non-positive `mu` or times
/// that are not strictly increasing, and [`OrbitError::DegenerateState`] for a
/// zero position vector.
pub fn herrick_gibbs_velocity(
    r1: [f64; 3],
    r2: [f64; 3],
    r3: [f64; 3],
    t1: f64,
    t2: f64,
    t3: f64,
    mu: f64,
) -> Result<[f64; 3], OrbitError> {
    if mu.is_nan() || mu <= 0.0 || !mu.is_finite() {
        return Err(OrbitError::InvalidElements);
    }
    if !(t1 < t2 && t2 < t3 && t3.is_finite()) {
        return Err(OrbitError::InvalidElements);
    }
    let (n1, n2, n3) = (norm3(r1), norm3(r2), norm3(r3));
    if n1 == 0.0 || n2 == 0.0 || n3 == 0.0 {
        return Err(OrbitError::DegenerateState);
    }
    let dt21 = t2 - t1;
    let dt31 = t3 - t1;
    let dt32 = t3 - t2;
    let c1 = -dt32 * (1.0 / (dt21 * dt31) + mu / (12.0 * n1 * n1 * n1));
    let c2 = (dt32 - dt21) * (1.0 / (dt21 * dt32) + mu / (12.0 * n2 * n2 * n2));
    let c3 = dt21 * (1.0 / (dt32 * dt31) + mu / (12.0 * n3 * n3 * n3));
    Ok(add3(add3(scale3(r1, c1), scale3(r2, c2)), scale3(r3, c3)))
}

/// Solves Kepler's equation `M = E - e sin E` for the eccentric anomaly `E`
/// by Newton's method, with the defaults [`KEPLER_TOLERANCE_RAD`] and
/// [`KEPLER_MAX_ITERATIONS`].
///
/// `mean_anomaly_rad` may be any real number; it is reduced to `[0, 2 pi)`.
/// Returns [`OrbitError::InvalidElements`] unless `0 <= e < 1`, and
/// [`OrbitError::NonConvergence`] if the iteration limit is reached.
pub fn solve_kepler(mean_anomaly_rad: f64, eccentricity: f64) -> Result<f64, OrbitError> {
    solve_kepler_limited(mean_anomaly_rad, eccentricity, KEPLER_MAX_ITERATIONS)
}

/// Kepler solver with an explicit iteration limit. Private helper used by
/// [`solve_kepler`] and by the tests of the non-convergence path.
fn solve_kepler_limited(
    mean_anomaly_rad: f64,
    eccentricity: f64,
    max_iterations: u32,
) -> Result<f64, OrbitError> {
    if !(0.0..1.0).contains(&eccentricity) || !mean_anomaly_rad.is_finite() {
        return Err(OrbitError::InvalidElements);
    }
    let m = mean_anomaly_rad.rem_euclid(TAU);
    // Starting guess: M for moderate eccentricity, pi near parabolic
    // eccentricities, which keeps Newton's method from overshooting.
    let mut e_anom = if eccentricity < 0.8 { m } else { PI };
    for _ in 0..max_iterations {
        let (sin_e, cos_e) = e_anom.sin_cos();
        let residual = e_anom - eccentricity * sin_e - m;
        let derivative = 1.0 - eccentricity * cos_e;
        let step = -residual / derivative;
        e_anom += step;
        if step.abs() <= KEPLER_TOLERANCE_RAD {
            return Ok(e_anom);
        }
    }
    Err(OrbitError::NonConvergence)
}

/// Converts a mean anomaly to a true anomaly for an elliptic orbit.
///
/// The result is in `[0, 2 pi)`. Errors are those of [`solve_kepler`].
pub fn mean_to_true_anomaly(mean_anomaly_rad: f64, eccentricity: f64) -> Result<f64, OrbitError> {
    let e_anom = solve_kepler(mean_anomaly_rad, eccentricity)?;
    let nu = 2.0
        * ((1.0 + eccentricity).sqrt() * (0.5 * e_anom).sin())
            .atan2((1.0 - eccentricity).sqrt() * (0.5 * e_anom).cos());
    Ok(nu.rem_euclid(TAU))
}

/// Converts a true anomaly to a mean anomaly for an elliptic orbit.
///
/// The result is in `[0, 2 pi)`. Returns [`OrbitError::InvalidElements`]
/// unless `0 <= e < 1`.
pub fn true_to_mean_anomaly(true_anomaly_rad: f64, eccentricity: f64) -> Result<f64, OrbitError> {
    if !(0.0..1.0).contains(&eccentricity) || !true_anomaly_rad.is_finite() {
        return Err(OrbitError::InvalidElements);
    }
    let e_anom = 2.0
        * ((1.0 - eccentricity).sqrt() * (0.5 * true_anomaly_rad).sin())
            .atan2((1.0 + eccentricity).sqrt() * (0.5 * true_anomaly_rad).cos());
    let m = e_anom - eccentricity * e_anom.sin();
    Ok(m.rem_euclid(TAU))
}

/// Dot product of two 3-vectors.
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Cross product of two 3-vectors.
/// Euclidean norm of a 3-vector.
fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

/// Signed angle from `a` to `b`, positive in the right-handed sense about
/// `axis` (which need not be unit length), in `[0, 2 pi)`.
fn signed_angle(a: [f64; 3], b: [f64; 3], axis: [f64; 3]) -> f64 {
    let sin_part = dot(cross(a, b), axis) / norm(axis);
    sin_part.atan2(dot(a, b)).rem_euclid(TAU)
}

impl Keplerian {
    /// Mean motion in rad/s, `sqrt(mu / a^3)`, with `mu` = [`WGS72_MU_KM3_S2`].
    pub fn mean_motion_rad_s(&self) -> f64 {
        (WGS72_MU_KM3_S2 / self.semi_major_axis_km.powi(3)).sqrt()
    }

    /// Orbital period in seconds, `2 pi / n`.
    pub fn period_s(&self) -> f64 {
        TAU / self.mean_motion_rad_s()
    }

    /// Returns the true anomaly in radians in `[0, 2 pi)`, solving Kepler's
    /// equation if the anomaly is stored as a mean anomaly.
    pub fn true_anomaly_rad(&self) -> Result<f64, OrbitError> {
        match self.anomaly {
            Anomaly::True(nu) => Ok(nu.rem_euclid(TAU)),
            Anomaly::Mean(m) => mean_to_true_anomaly(m, self.eccentricity),
        }
    }

    /// Converts the elements to a Cartesian state in the inertial frame of the
    /// elements, in km and km/s.
    ///
    /// Returns [`OrbitError::InvalidElements`] unless the semi-major axis is
    /// positive and `0 <= e < 1`. A mean anomaly is converted with
    /// [`mean_to_true_anomaly`].
    pub fn to_state(&self) -> Result<StateVector, OrbitError> {
        let a = self.semi_major_axis_km;
        let e = self.eccentricity;
        if a.is_nan() || a <= 0.0 || !(0.0..1.0).contains(&e) {
            return Err(OrbitError::InvalidElements);
        }
        let nu = self.true_anomaly_rad()?;
        let p = a * (1.0 - e * e);
        let r = p / (1.0 + e * nu.cos());
        // Position and velocity in the perifocal (PQW) frame.
        let r_pqw = [r * nu.cos(), r * nu.sin(), 0.0];
        let v_scale = (WGS72_MU_KM3_S2 / p).sqrt();
        let v_pqw = [-v_scale * nu.sin(), v_scale * (e + nu.cos()), 0.0];

        // Rotation from PQW to the inertial frame: Rz(-raan) Rx(-i) Rz(-argp).
        let (sin_o, cos_o) = self.raan_rad.sin_cos();
        let (sin_i, cos_i) = self.inclination_rad.sin_cos();
        let (sin_w, cos_w) = self.arg_perigee_rad.sin_cos();
        let rot = [
            [
                cos_o * cos_w - sin_o * sin_w * cos_i,
                -cos_o * sin_w - sin_o * cos_w * cos_i,
            ],
            [
                sin_o * cos_w + cos_o * sin_w * cos_i,
                -sin_o * sin_w + cos_o * cos_w * cos_i,
            ],
            [sin_w * sin_i, cos_w * sin_i],
        ];
        let apply = |v: [f64; 3]| -> [f64; 3] {
            [
                rot[0][0] * v[0] + rot[0][1] * v[1],
                rot[1][0] * v[0] + rot[1][1] * v[1],
                rot[2][0] * v[0] + rot[2][1] * v[1],
            ]
        };
        Ok(StateVector {
            position_km: apply(r_pqw),
            velocity_km_s: apply(v_pqw),
        })
    }

    /// Computes the elements of an elliptic orbit from a Cartesian state, with
    /// the true anomaly in the result.
    ///
    /// Degenerate geometry is handled by convention, as usual:
    /// - Circular inclined orbit: the argument of perigee is 0 and the true
    ///   anomaly is the argument of latitude.
    /// - Equatorial elliptic orbit: the RAAN is 0 and the argument of perigee is
    ///   the true longitude of perigee (measured in the sense of the orbit).
    /// - Circular equatorial orbit: the RAAN and argument of perigee are 0 and
    ///   the true anomaly is the true longitude.
    ///
    /// Returns [`OrbitError::DegenerateState`] for zero position or zero angular
    /// momentum (a rectilinear orbit), and [`OrbitError::InvalidElements`] for
    /// `e >= 1` or non-positive energy.
    pub fn from_state(state: &StateVector, epoch: JulianDate) -> Result<Self, OrbitError> {
        let r_vec = state.position_km;
        let v_vec = state.velocity_km_s;
        let r = norm(r_vec);
        let v2 = dot(v_vec, v_vec);
        if r.is_nan() || r <= 0.0 {
            return Err(OrbitError::DegenerateState);
        }
        let h_vec = cross(r_vec, v_vec);
        let h = norm(h_vec);
        if h.is_nan() || h <= 0.0 {
            return Err(OrbitError::DegenerateState);
        }
        let h_hat = [h_vec[0] / h, h_vec[1] / h, h_vec[2] / h];
        let mu = WGS72_MU_KM3_S2;

        // Eccentricity vector.
        let rdotv = dot(r_vec, v_vec);
        let e_vec = [
            ((v2 - mu / r) * r_vec[0] - rdotv * v_vec[0]) / mu,
            ((v2 - mu / r) * r_vec[1] - rdotv * v_vec[1]) / mu,
            ((v2 - mu / r) * r_vec[2] - rdotv * v_vec[2]) / mu,
        ];
        let e = norm(e_vec);
        if !(0.0..1.0).contains(&e) {
            return Err(OrbitError::InvalidElements);
        }
        let energy = 0.5 * v2 - mu / r;
        if energy.is_nan() || energy >= 0.0 {
            return Err(OrbitError::InvalidElements);
        }
        let a = -mu / (2.0 * energy);

        // Node vector n = k x h.
        let n_vec = [-h_vec[1], h_vec[0], 0.0];
        let n = norm(n_vec);
        let inclination = h_vec[0].hypot(h_vec[1]).atan2(h_vec[2]);
        let equatorial = n / h < DEGENERACY_EPS;
        let circular = e < DEGENERACY_EPS;

        let raan = if equatorial {
            0.0
        } else {
            n_vec[1].atan2(n_vec[0]).rem_euclid(TAU)
        };
        let (arg_perigee, anomaly) = match (equatorial, circular) {
            (false, false) => (
                signed_angle(n_vec, e_vec, h_hat),
                signed_angle(e_vec, r_vec, h_hat),
            ),
            (true, false) => (
                signed_angle([1.0, 0.0, 0.0], e_vec, h_hat),
                signed_angle(e_vec, r_vec, h_hat),
            ),
            (false, true) => (0.0, signed_angle(n_vec, r_vec, h_hat)),
            (true, true) => (0.0, signed_angle([1.0, 0.0, 0.0], r_vec, h_hat)),
        };

        Ok(Keplerian {
            semi_major_axis_km: a,
            eccentricity: e,
            inclination_rad: inclination,
            raan_rad: raan,
            arg_perigee_rad: arg_perigee,
            anomaly: Anomaly::True(anomaly),
            epoch,
        })
    }
}

/// Computes the TLE checksum digit of the first 68 characters of a TLE line.
///
/// The checksum is the sum of the digits in columns 1 to 68, with each minus
/// sign counted as 1 and every other character as 0, taken modulo 10. It is
/// stored in column 69.
pub fn tle_checksum(line: &str) -> u8 {
    let sum: u32 = line
        .bytes()
        .take(68)
        .map(|b| match b {
            b'0'..=b'9' => u32::from(b - b'0'),
            b'-' => 1,
            _ => 0,
        })
        .sum();
    (sum % 10) as u8
}

/// A parsed NORAD two-line element set.
///
/// Field names follow the NORAD format. Units are as printed in the TLE:
/// angles in degrees, mean motion in revolutions per day, and the drag term
/// `bstar` in inverse Earth radii. Fixed-column positions are taken from the
/// NORAD format (Hoots and Roehrich, Spacetrack Report No. 3, 1980, Section
/// "Two-line element set format"; the layout of Celestrak's format
/// description, which uses the same columns). Columns are 1-based and
/// inclusive in the comments below.
#[derive(Debug, Clone, PartialEq)]
pub struct Tle {
    /// Satellite catalogue number (line 1, columns 3-7; line 2, columns 3-7).
    pub satellite_number: u32,
    /// Classification, `U`, `C` or `S` (line 1, column 8).
    pub classification: char,
    /// International designator, for example `58002B` (line 1, columns 10-17).
    pub international_designator: String,
    /// Full epoch year, from the two-digit year in columns 19-20, with the
    /// convention that 57 to 99 mean 1957 to 1999 and 00 to 56 mean 2000 to 2056.
    pub epoch_year: i32,
    /// Epoch as a day of the year, where 1.0 is 00:00 on 1 January (line 1,
    /// columns 21-32).
    pub epoch_day: f64,
    /// First derivative of the mean motion divided by two, in rev/day^2
    /// (line 1, columns 34-43). Parsed but not used by [`Sgp4`].
    pub mean_motion_dot: f64,
    /// Second derivative of the mean motion divided by six, in rev/day^3
    /// (line 1, columns 45-52, implied decimal point and signed exponent).
    /// Parsed but not used by [`Sgp4`].
    pub mean_motion_ddot: f64,
    /// SGP4 drag term `B*` in inverse Earth radii (line 1, columns 54-61,
    /// implied decimal point and signed exponent).
    pub bstar: f64,
    /// Ephemeris type (line 1, column 63).
    pub ephemeris_type: u8,
    /// Element set number (line 1, columns 65-68).
    pub element_set_number: u32,
    /// Inclination in degrees (line 2, columns 9-16).
    pub inclination_deg: f64,
    /// Right ascension of the ascending node in degrees (line 2, columns 18-25).
    pub raan_deg: f64,
    /// Eccentricity, with the decimal point assumed (line 2, columns 27-33).
    pub eccentricity: f64,
    /// Argument of perigee in degrees (line 2, columns 35-42).
    pub arg_perigee_deg: f64,
    /// Mean anomaly in degrees (line 2, columns 44-51).
    pub mean_anomaly_deg: f64,
    /// Mean motion in revolutions per day (line 2, columns 53-63).
    pub mean_motion_rev_day: f64,
    /// Revolution number at epoch (line 2, columns 64-68).
    pub revolution_number: u32,
}

/// Returns the substring of 1-based inclusive columns `first..=last`.
fn columns(line: &str, first: usize, last: usize) -> Option<&str> {
    line.get(first - 1..last)
}

/// Parses a fixed-column number, trimming blanks. Blank fields are an error.
fn parse_field<T: core::str::FromStr>(line: &str, first: usize, last: usize) -> Option<T> {
    let text = columns(line, first, last)?.trim();
    if text.is_empty() {
        return None;
    }
    text.parse().ok()
}

/// Parses a field with an implied decimal point and a signed exponent, such as
/// `" 28098-4"` (= 0.28098e-4) or `"-11606-4"` (= -0.11606e-4). The field is
/// eight characters: a sign (or blank), five digits, a sign and one digit.
fn parse_implied_exponent(text: &str) -> Option<f64> {
    if text.len() != 8 || !text.is_ascii() {
        return None;
    }
    let (mantissa, exponent) = text.split_at(6);
    let sign = match mantissa.as_bytes()[0] {
        b' ' | b'+' => 1.0,
        b'-' => -1.0,
        _ => return None,
    };
    let digits = &mantissa[1..];
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value: f64 = format!("0.{digits}").parse().ok()?;
    let exp: i32 = exponent.trim().parse().ok()?;
    Some(sign * value * 10.0_f64.powf(f64::from(exp)))
}

/// Returns the Julian Date of 00:00 on 1 January of `year` (proleptic
/// Gregorian calendar), by the day-count algorithm of H. Hinnant.
fn jd_jan_first(year: i32) -> f64 {
    let y = i64::from(year) - 1; // January belongs to the previous "March year"
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (1 + 9) + 2) / 5; // day-of-year of 1 January, March-based
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468; // days since 1970-01-01
    days as f64 + 2_440_587.5
}

impl Tle {
    /// Parses a two-line element set. The lines must be ASCII. Characters after
    /// column 69 are ignored, as in the Python `sgp4` package.
    ///
    /// Each line is checked for: length (at least 69 characters), its line
    /// number, every fixed-column field, and the checksum in column 69. Both
    /// lines must carry the same satellite number.
    pub fn parse(line1: &str, line2: &str) -> Result<Self, OrbitError> {
        let bad1 = OrbitError::InvalidTle { line: 1 };
        let bad2 = OrbitError::InvalidTle { line: 2 };
        if !line1.is_ascii() || line1.len() < 69 || !line1.starts_with("1 ") {
            return Err(bad1);
        }
        if !line2.is_ascii() || line2.len() < 69 || !line2.starts_with("2 ") {
            return Err(bad2);
        }
        check_checksum(line1, 1)?;
        check_checksum(line2, 2)?;

        let satnum1: u32 = parse_field(line1, 3, 7).ok_or(bad1)?;
        let satnum2: u32 = parse_field(line2, 3, 7).ok_or(bad2)?;
        if satnum1 != satnum2 {
            return Err(OrbitError::TleSatelliteMismatch);
        }

        // Line 1.
        let classification = line1.as_bytes()[7] as char;
        let international_designator = columns(line1, 10, 17).ok_or(bad1)?.trim().to_string();
        let two_digit_year: i32 = parse_field(line1, 19, 20).ok_or(bad1)?;
        let epoch_year = if two_digit_year < 57 {
            two_digit_year + 2000
        } else {
            two_digit_year + 1900
        };
        let epoch_day: f64 = parse_field(line1, 21, 32).ok_or(bad1)?;
        let mean_motion_dot: f64 = parse_field(line1, 34, 43).ok_or(bad1)?;
        let mean_motion_ddot =
            parse_implied_exponent(columns(line1, 45, 52).ok_or(bad1)?).ok_or(bad1)?;
        let bstar = parse_implied_exponent(columns(line1, 54, 61).ok_or(bad1)?).ok_or(bad1)?;
        let ephemeris_type = match columns(line1, 63, 63).ok_or(bad1)?.trim() {
            "" => 0,
            text => text.parse().map_err(|_| bad1)?,
        };
        let element_set_number: u32 = parse_field(line1, 65, 68).ok_or(bad1)?;

        // Line 2.
        let inclination_deg: f64 = parse_field(line2, 9, 16).ok_or(bad2)?;
        let raan_deg: f64 = parse_field(line2, 18, 25).ok_or(bad2)?;
        let ecc_digits = columns(line2, 27, 33).ok_or(bad2)?.replace(' ', "0");
        let eccentricity: f64 = format!("0.{ecc_digits}").parse().map_err(|_| bad2)?;
        let arg_perigee_deg: f64 = parse_field(line2, 35, 42).ok_or(bad2)?;
        let mean_anomaly_deg: f64 = parse_field(line2, 44, 51).ok_or(bad2)?;
        let mean_motion_rev_day: f64 = parse_field(line2, 53, 63).ok_or(bad2)?;
        let revolution_number: u32 = parse_field(line2, 64, 68).ok_or(bad2)?;

        Ok(Tle {
            satellite_number: satnum1,
            classification,
            international_designator,
            epoch_year,
            epoch_day,
            mean_motion_dot,
            mean_motion_ddot,
            bstar,
            ephemeris_type,
            element_set_number,
            inclination_deg,
            raan_deg,
            eccentricity,
            arg_perigee_deg,
            mean_anomaly_deg,
            mean_motion_rev_day,
            revolution_number,
        })
    }

    /// Epoch as a Julian Date (the time scale is the caller's responsibility;
    /// see the module notes). The result is computed from the year and the
    /// day of year in double precision, so it is good to about 40 microseconds.
    pub fn epoch_jd(&self) -> JulianDate {
        JulianDate::from_f64(jd_jan_first(self.epoch_year) + self.epoch_day - 1.0)
    }
}

/// Checks the checksum digit in column 69 of a TLE line.
fn check_checksum(line: &str, line_number: u8) -> Result<(), OrbitError> {
    let stored = line.as_bytes()[68];
    if !stored.is_ascii_digit() {
        return Err(OrbitError::InvalidTle { line: line_number });
    }
    if u32::from(stored - b'0') != u32::from(tle_checksum(line)) {
        return Err(OrbitError::TleChecksum { line: line_number });
    }
    Ok(())
}

/// Period at or above which SGP4 uses the deep-space (SDP4) terms, in minutes.
const DEEP_SPACE_PERIOD_MIN: f64 = 225.0;

mod sdp4;

/// The SGP4 near-Earth propagator, initialised from a TLE.
///
/// Built with [`Sgp4::from_tle`] or [`Sgp4::from_tle_lines`]. Propagation is
/// with [`Sgp4::propagate`], which returns the TEME state at a time in minutes
/// from the TLE epoch. Initialisation follows `sgp4init` in the 2006 code, with
/// the improved ("i") operation mode. Orbits with a period of 225 minutes or
/// more also get the SDP4 deep-space terms.
#[derive(Debug, Clone, PartialEq)]
pub struct Sgp4 {
    epoch: JulianDate,
    xke: f64,
    j2: f64,
    radius_km: f64,
    bstar: f64,
    ecco: f64,
    inclo: f64,
    nodeo: f64,
    argpo: f64,
    mo: f64,
    no_unkozai: f64,
    mdot: f64,
    argpdot: f64,
    nodedot: f64,
    nodecf: f64,
    omgcof: f64,
    xmcof: f64,
    t2cof: f64,
    t3cof: f64,
    t4cof: f64,
    t5cof: f64,
    cc1: f64,
    cc4: f64,
    cc5: f64,
    d2: f64,
    d3: f64,
    d4: f64,
    delmo: f64,
    sinmao: f64,
    eta: f64,
    aycof: f64,
    xlcof: f64,
    con41: f64,
    x1mth2: f64,
    x7thm1: f64,
    isimp: bool,
    /// SDP4 state for deep-space orbits; `None` for near-Earth orbits.
    deep: Option<sdp4::DeepState>,
}

impl Sgp4 {
    /// Initialises the propagator from a parsed TLE with WGS-72 constants.
    ///
    /// Orbits with a period of 225 minutes or more are initialised with the
    /// SDP4 deep-space terms rather than rejected. Returns
    /// [`OrbitError::MeanMotionNonPositive`] for a zero or negative mean
    /// motion. Eccentricity, semi-latus rectum and decay are checked when
    /// propagating, as in the reference code.
    pub fn from_tle(tle: &Tle) -> Result<Self, OrbitError> {
        let radius = WGS72_RADIUS_EARTH_KM;
        let mu = WGS72_MU_KM3_S2;
        let j2 = WGS72_J2;
        let j4 = WGS72_J4;
        let j3oj2 = WGS72_J3 / WGS72_J2;
        let xke = 60.0 / (radius * radius * radius / mu).sqrt();
        let x2o3 = 2.0 / 3.0;

        // Converts TLE units: rev/day to rad/min, degrees to radians.
        let no_kozai = tle.mean_motion_rev_day / (1440.0 / TAU);
        if no_kozai.is_nan() || no_kozai <= 0.0 {
            return Err(OrbitError::MeanMotionNonPositive);
        }
        let ecco = tle.eccentricity;
        let inclo = tle.inclination_deg * DEG2RAD;
        let nodeo = tle.raan_deg * DEG2RAD;
        let argpo = tle.arg_perigee_deg * DEG2RAD;
        let mo = tle.mean_anomaly_deg * DEG2RAD;
        let bstar = tle.bstar;

        // Auxiliary epoch quantities (sgp4 _initl).
        let eccsq = ecco * ecco;
        let omeosq = 1.0 - eccsq;
        let rteosq = omeosq.sqrt();
        let cosio = inclo.cos();
        let cosio2 = cosio * cosio;

        // Un-Kozai the mean motion.
        let ak = (xke / no_kozai).powf(x2o3);
        let d1 = 0.75 * j2 * (3.0 * cosio2 - 1.0) / (rteosq * omeosq);
        let mut del = d1 / (ak * ak);
        let adel = ak * (1.0 - del * del - del * (1.0 / 3.0 + 134.0 * del * del / 81.0));
        del = d1 / (adel * adel);
        let no_unkozai = no_kozai / (1.0 + del);

        let deep_space = TAU / no_unkozai >= DEEP_SPACE_PERIOD_MIN;

        let ao = (xke / no_unkozai).powf(x2o3);
        let sinio = inclo.sin();
        let po = ao * omeosq;
        let con42 = 1.0 - 5.0 * cosio2;
        let con41 = -con42 - cosio2 - cosio2;
        let posq = po * po;
        let rp = ao * (1.0 - ecco);

        // Drag and secular-rate coefficients (sgp4init).
        let ss = 78.0 / radius + 1.0;
        let qzms2ttemp = (120.0 - 78.0) / radius;
        let qzms2t = qzms2ttemp * qzms2ttemp * qzms2ttemp * qzms2ttemp;
        let temp4 = 1.5e-12;

        let isimp = rp < 220.0 / radius + 1.0 || deep_space;
        let mut sfour = ss;
        let mut qzms24 = qzms2t;
        let perige = (rp - 1.0) * radius;
        if perige < 156.0 {
            sfour = perige - 78.0;
            if perige < 98.0 {
                sfour = 20.0;
            }
            let qzms24temp = (120.0 - sfour) / radius;
            qzms24 = qzms24temp * qzms24temp * qzms24temp * qzms24temp;
            sfour = sfour / radius + 1.0;
        }
        let pinvsq = 1.0 / posq;
        let tsi = 1.0 / (ao - sfour);
        let eta = ao * ecco * tsi;
        let etasq = eta * eta;
        let eeta = ecco * eta;
        let psisq = (1.0 - etasq).abs();
        let coef = qzms24 * tsi.powf(4.0);
        let coef1 = coef / psisq.powf(3.5);
        let cc2 = coef1
            * no_unkozai
            * (ao * (1.0 + 1.5 * etasq + eeta * (4.0 + etasq))
                + 0.375 * j2 * tsi / psisq * con41 * (8.0 + 3.0 * etasq * (8.0 + etasq)));
        let cc1 = bstar * cc2;
        let mut cc3 = 0.0;
        if ecco > 1.0e-4 {
            cc3 = -2.0 * coef * tsi * j3oj2 * no_unkozai * sinio / ecco;
        }
        let x1mth2 = 1.0 - cosio2;
        let cc4 = 2.0
            * no_unkozai
            * coef1
            * ao
            * omeosq
            * (eta * (2.0 + 0.5 * etasq) + ecco * (0.5 + 2.0 * etasq)
                - j2 * tsi / (ao * psisq)
                    * (-3.0 * con41 * (1.0 - 2.0 * eeta + etasq * (1.5 - 0.5 * eeta))
                        + 0.75
                            * x1mth2
                            * (2.0 * etasq - eeta * (1.0 + etasq))
                            * (2.0 * argpo).cos()));
        let cc5 = 2.0 * coef1 * ao * omeosq * (1.0 + 2.75 * (etasq + eeta) + eeta * etasq);
        let cosio4 = cosio2 * cosio2;
        let temp1 = 1.5 * j2 * pinvsq * no_unkozai;
        let temp2 = 0.5 * temp1 * j2 * pinvsq;
        let temp3 = -0.46875 * j4 * pinvsq * pinvsq * no_unkozai;
        let mdot = no_unkozai
            + 0.5 * temp1 * rteosq * con41
            + 0.0625 * temp2 * rteosq * (13.0 - 78.0 * cosio2 + 137.0 * cosio4);
        let argpdot = -0.5 * temp1 * con42
            + 0.0625 * temp2 * (7.0 - 114.0 * cosio2 + 395.0 * cosio4)
            + temp3 * (3.0 - 36.0 * cosio2 + 49.0 * cosio4);
        let xhdot1 = -temp1 * cosio;
        let nodedot = xhdot1
            + (0.5 * temp2 * (4.0 - 19.0 * cosio2) + 2.0 * temp3 * (3.0 - 7.0 * cosio2)) * cosio;
        let omgcof = bstar * cc3 * argpo.cos();
        let mut xmcof = 0.0;
        if ecco > 1.0e-4 {
            xmcof = -x2o3 * coef * bstar / eeta;
        }
        let nodecf = 3.5 * omeosq * xhdot1 * cc1;
        let t2cof = 1.5 * cc1;
        let xlcof = if (cosio + 1.0).abs() > 1.5e-12 {
            -0.25 * j3oj2 * sinio * (3.0 + 5.0 * cosio) / (1.0 + cosio)
        } else {
            -0.25 * j3oj2 * sinio * (3.0 + 5.0 * cosio) / temp4
        };
        let aycof = -0.5 * j3oj2 * sinio;
        let delmotemp = 1.0 + eta * mo.cos();
        let delmo = delmotemp * delmotemp * delmotemp;
        let sinmao = mo.sin();
        let x7thm1 = 7.0 * cosio2 - 1.0;

        // Higher-order drag terms (used only when not in simplified mode).
        let (mut d2, mut d3, mut d4, mut t3cof, mut t4cof, mut t5cof) =
            (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        if !isimp {
            let cc1sq = cc1 * cc1;
            d2 = 4.0 * ao * tsi * cc1sq;
            let temp = d2 * tsi * cc1 / 3.0;
            d3 = (17.0 * ao + sfour) * temp;
            d4 = 0.5 * temp * ao * tsi * (221.0 * ao + 31.0 * sfour) * cc1;
            t3cof = d2 + 2.0 * cc1sq;
            t4cof = 0.25 * (3.0 * d3 + cc1 * (12.0 * d2 + 10.0 * cc1sq));
            t5cof = 0.2
                * (3.0 * d4 + 12.0 * cc1 * d3 + 6.0 * d2 * d2 + 15.0 * cc1sq * (2.0 * d2 + cc1sq));
        }

        // Deep-space state (sgp4init): epoch in days since 1949 December 31 0h,
        // and sidereal time at epoch from SGP4's gstime.
        let jd = tle.epoch_jd();
        let deep = if deep_space {
            let epoch_days = (jd.day() - 2_433_281.5) + jd.fraction();
            let gsto = sdp4::gstime(jd.day() + jd.fraction());
            Some(sdp4::initialise(
                xke, epoch_days, ecco, eccsq, argpo, inclo, mo, mdot, no_unkozai, nodeo, nodedot,
                argpdot, gsto,
            ))
        } else {
            None
        };

        Ok(Sgp4 {
            epoch: jd,
            xke,
            j2,
            radius_km: radius,
            bstar,
            ecco,
            inclo,
            nodeo,
            argpo,
            mo,
            no_unkozai,
            mdot,
            argpdot,
            nodedot,
            nodecf,
            omgcof,
            xmcof,
            t2cof,
            t3cof,
            t4cof,
            t5cof,
            cc1,
            cc4,
            cc5,
            d2,
            d3,
            d4,
            delmo,
            sinmao,
            eta,
            aycof,
            xlcof,
            con41,
            x1mth2,
            x7thm1,
            isimp,
            deep,
        })
    }

    /// Parses two TLE lines and initialises the propagator. See [`Tle::parse`]
    /// and [`Sgp4::from_tle`].
    pub fn from_tle_lines(line1: &str, line2: &str) -> Result<Self, OrbitError> {
        Self::from_tle(&Tle::parse(line1, line2)?)
    }

    /// Epoch of the element set as a Julian Date.
    pub fn epoch(&self) -> JulianDate {
        self.epoch
    }

    /// Propagates to `minutes` after the TLE epoch (negative values are
    /// allowed) and returns the TEME position in km and velocity in km/s.
    ///
    /// Errors, as in the reference `sgp4` procedure:
    /// [`OrbitError::EccentricityOutOfRange`] (error 1 or 3),
    /// [`OrbitError::SemiLatusRectumNegative`] (error 4), and
    /// [`OrbitError::Decayed`] (error 6, radius below the surface).
    pub fn propagate(&self, minutes: f64) -> Result<StateVector, OrbitError> {
        let t = minutes;
        let x2o3 = 2.0 / 3.0;
        let vkmpersec = self.radius_km * self.xke / 60.0;

        // Secular gravity and atmospheric drag.
        let xmdf = self.mo + self.mdot * t;
        let argpdf = self.argpo + self.argpdot * t;
        let nodedf = self.nodeo + self.nodedot * t;
        let mut argpm = argpdf;
        let mut mm = xmdf;
        let t2 = t * t;
        let mut nodem = nodedf + self.nodecf * t2;
        let mut tempa = 1.0 - self.cc1 * t;
        let mut tempe = self.bstar * self.cc4 * t;
        let mut templ = self.t2cof * t2;
        if !self.isimp {
            let delomg = self.omgcof * t;
            let delmtemp = 1.0 + self.eta * xmdf.cos();
            let delm = self.xmcof * (delmtemp * delmtemp * delmtemp - self.delmo);
            let temp = delomg + delm;
            mm = xmdf + temp;
            argpm = argpdf - temp;
            let t3 = t2 * t;
            let t4 = t3 * t;
            tempa = tempa - self.d2 * t2 - self.d3 * t3 - self.d4 * t4;
            tempe += self.bstar * self.cc5 * (mm.sin() - self.sinmao);
            templ = templ + self.t3cof * t3 + t4 * (self.t4cof + t * self.t5cof);
        }
        let nm0 = self.no_unkozai;
        let mut em = self.ecco;
        let mut inclm = self.inclo;
        let mut nm = nm0;

        // Deep-space secular and resonance terms (SDP4 `dspace`).
        if let Some(ds) = &self.deep {
            let d = sdp4::dspace(
                ds,
                self.argpo,
                self.argpdot,
                t,
                self.no_unkozai,
                em,
                argpm,
                inclm,
                mm,
                nodem,
                nm,
            );
            em = d.em;
            inclm = d.inclm;
            argpm = d.argpm;
            mm = d.mm;
            nodem = d.nodem;
            nm = d.nm;
            if nm <= 0.0 {
                return Err(OrbitError::MeanMotionNonPositive);
            }
        }

        let am = (self.xke / nm).powf(x2o3) * tempa * tempa;
        let nm = self.xke / am.powf(1.5);
        em -= tempe;
        if !(-0.001..1.0).contains(&em) {
            return Err(OrbitError::EccentricityOutOfRange);
        }
        if em < 1.0e-6 {
            em = 1.0e-6;
        }
        mm += self.no_unkozai * templ;
        let xlm = mm + argpm + nodem;
        // The node keeps its sign (as in the reference code); the other angles
        // are reduced to [0, 2 pi).
        nodem = if nodem >= 0.0 {
            nodem % TAU
        } else {
            -((-nodem) % TAU)
        };
        argpm = argpm.rem_euclid(TAU);
        let xlm = xlm.rem_euclid(TAU);
        let mm = (xlm - argpm - nodem).rem_euclid(TAU);

        let sinim = inclm.sin();
        let cosim = inclm.cos();
        let mut ep = em;
        let mut xincp = inclm;
        let mut argpp = argpm;
        let mut nodep = nodem;
        let mut mp = mm;
        let (sinip, cosip, aycof, xlcof, con41, x1mth2, x7thm1);
        if let Some(ds) = &self.deep {
            // Lunar-solar periodics (SDP4 `dpper`), then the deep-space
            // orientation terms, which use the perturbed inclination.
            (ep, xincp, nodep, argpp, mp) = sdp4::dpper(ds, t, false, ep, xincp, nodep, argpp, mp);
            if xincp < 0.0 {
                xincp = -xincp;
                nodep += PI;
                argpp -= PI;
            }
            if !(0.0..=1.0).contains(&ep) {
                return Err(OrbitError::EccentricityOutOfRange);
            }
            let j3oj2 = WGS72_J3 / WGS72_J2;
            sinip = xincp.sin();
            cosip = xincp.cos();
            aycof = -0.5 * j3oj2 * sinip;
            xlcof = if (cosip + 1.0).abs() > 1.5e-12 {
                -0.25 * j3oj2 * sinip * (3.0 + 5.0 * cosip) / (1.0 + cosip)
            } else {
                -0.25 * j3oj2 * sinip * (3.0 + 5.0 * cosip) / 1.5e-12
            };
            let cosisq = cosip * cosip;
            con41 = 3.0 * cosisq - 1.0;
            x1mth2 = 1.0 - cosisq;
            x7thm1 = 7.0 * cosisq - 1.0;
        } else {
            sinip = sinim;
            cosip = cosim;
            aycof = self.aycof;
            xlcof = self.xlcof;
            con41 = self.con41;
            x1mth2 = self.x1mth2;
            x7thm1 = self.x7thm1;
        }

        // Long-period periodics.
        let axnl = ep * argpp.cos();
        let temp = 1.0 / (am * (1.0 - ep * ep));
        let aynl = ep * argpp.sin() + temp * aycof;
        let xl = mp + argpp + nodep + temp * xlcof * axnl;

        // Kepler's equation, Vallado's fixed-count Newton iteration. The sines
        // and cosines of the last evaluation are kept for the short-period
        // terms below, as in the reference code.
        let u = (xl - nodep).rem_euclid(TAU);
        let mut eo1 = u;
        let mut tem5: f64 = 9999.9;
        let mut ktr = 1;
        let mut sineo1 = 0.0;
        let mut coseo1 = 0.0;
        while tem5.abs() >= 1.0e-12 && ktr <= 10 {
            sineo1 = eo1.sin();
            coseo1 = eo1.cos();
            tem5 = 1.0 - coseo1 * axnl - sineo1 * aynl;
            tem5 = (u - aynl * coseo1 + axnl * sineo1 - eo1) / tem5;
            if tem5.abs() >= 0.95 {
                tem5 = if tem5 > 0.0 { 0.95 } else { -0.95 };
            }
            eo1 += tem5;
            ktr += 1;
        }

        // Short-period preliminary quantities.
        let ecose = axnl * coseo1 + aynl * sineo1;
        let esine = axnl * sineo1 - aynl * coseo1;
        let el2 = axnl * axnl + aynl * aynl;
        let pl = am * (1.0 - el2);
        if pl < 0.0 {
            return Err(OrbitError::SemiLatusRectumNegative);
        }
        let rl = am * (1.0 - ecose);
        let rdotl = am.sqrt() * esine / rl;
        let rvdotl = pl.sqrt() / rl;
        let betal = (1.0 - el2).sqrt();
        let temp = esine / (1.0 + betal);
        let sinu = am / rl * (sineo1 - aynl - axnl * temp);
        let cosu = am / rl * (coseo1 - axnl + aynl * temp);
        let mut su = sinu.atan2(cosu);
        let sin2u = (cosu + cosu) * sinu;
        let cos2u = 1.0 - 2.0 * sinu * sinu;
        let temp = 1.0 / pl;
        let temp1 = 0.5 * self.j2 * temp;
        let temp2 = temp1 * temp;

        // Short-period periodics.
        let mrt = rl * (1.0 - 1.5 * temp2 * betal * con41) + 0.5 * temp1 * x1mth2 * cos2u;
        su -= 0.25 * temp2 * x7thm1 * sin2u;
        let xnode = nodep + 1.5 * temp2 * cosip * sin2u;
        let xinc = xincp + 1.5 * temp2 * cosip * sinip * cos2u;
        let mvt = rdotl - nm * temp1 * x1mth2 * sin2u / self.xke;
        let rvdot = rvdotl + nm * temp1 * (x1mth2 * cos2u + 1.5 * con41) / self.xke;

        // Orientation vectors.
        let sinsu = su.sin();
        let cossu = su.cos();
        let snod = xnode.sin();
        let cnod = xnode.cos();
        let sini = xinc.sin();
        let cosi = xinc.cos();
        let xmx = -snod * cosi;
        let xmy = cnod * cosi;
        let ux = xmx * sinsu + cnod * cossu;
        let uy = xmy * sinsu + snod * cossu;
        let uz = sini * sinsu;
        let vx = xmx * cossu - cnod * sinsu;
        let vy = xmy * cossu - snod * sinsu;
        let vz = sini * cossu;

        if mrt < 1.0 {
            return Err(OrbitError::Decayed);
        }
        let mr = mrt * self.radius_km;
        Ok(StateVector {
            position_km: [mr * ux, mr * uy, mr * uz],
            velocity_km_s: [
                (mvt * ux + rvdot * vx) * vkmpersec,
                (mvt * uy + rvdot * vy) * vkmpersec,
                (mvt * uz + rvdot * vz) * vkmpersec,
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small deterministic generator (xorshift64*) for the random tests.
    struct Rng(u64);

    impl Rng {
        fn next_f64(&mut self) -> f64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            let bits = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11;
            bits as f64 / (1u64 << 53) as f64
        }

        fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * self.next_f64()
        }
    }

    const EPOCH: JulianDate = JulianDate::J2000;

    /// Vanguard 1 element set, first case of Vallado's SGP4 verification file
    /// (`SGP4-VER.TLE` in the Python `sgp4` package).
    const VANGUARD_L1: &str =
        "1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753";
    const VANGUARD_L2: &str =
        "2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667";

    /// Reference TEME states from Vallado's `tcppver.out` (WGS-72, opsmode i),
    /// for minutes since epoch 0, 360 and 720: position km, velocity km/s.
    const VANGUARD_REF: [(f64, [f64; 3], [f64; 3]); 3] = [
        (
            0.0,
            [7022.46529266, -1400.08296755, 0.03995155],
            [1.893841015, 6.405893759, 4.534807250],
        ),
        (
            360.0,
            [-7154.03120202, -3783.17682504, -3536.19412294],
            [4.741887409, -4.151817765, -2.093935425],
        ),
        (
            720.0,
            [-7134.59340119, 6531.68641334, 3260.27186483],
            [-4.113793027, -2.911922039, -2.557327851],
        ),
    ];

    fn assert_close(a: f64, b: f64, tol: f64, what: &str) {
        assert!((a - b).abs() <= tol, "{what}: {a} vs {b} (tol {tol})");
    }

    fn angle_close(a: f64, b: f64, tol: f64, what: &str) {
        let d = (a - b).rem_euclid(TAU);
        let d = d.min(TAU - d);
        assert!(d <= tol, "{what}: {a} vs {b} (angular difference {d})");
    }

    fn norm_of(v: [f64; 3]) -> f64 {
        norm(v)
    }

    #[test]
    fn kepler_circular_is_identity() {
        for k in 0..20 {
            let m = 0.3 * f64::from(k) - 2.0;
            let e_anom = solve_kepler(m, 0.0).unwrap();
            assert_close(e_anom.rem_euclid(TAU), m.rem_euclid(TAU), 1e-14, "E = M");
            let nu = mean_to_true_anomaly(m, 0.0).unwrap();
            assert_close(nu, m.rem_euclid(TAU), 1e-13, "nu = M for e = 0");
        }
    }

    #[test]
    fn kepler_residual_is_small_across_eccentricities() {
        for &e in &[0.0, 0.01, 0.2, 0.5, 0.8, 0.95, 0.99] {
            for k in 0..64 {
                let m = TAU * f64::from(k) / 64.0 + 0.001;
                let e_anom = solve_kepler(m, e).unwrap();
                let residual = e_anom - e * e_anom.sin() - m.rem_euclid(TAU);
                assert!(
                    residual.abs() < 1e-13,
                    "residual {residual} at e={e}, M={m}"
                );
            }
        }
    }

    #[test]
    fn anomaly_round_trip() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..500 {
            let e = rng.uniform(0.0, 0.95);
            let m = rng.uniform(0.0, TAU);
            let nu = mean_to_true_anomaly(m, e).unwrap();
            let m2 = true_to_mean_anomaly(nu, e).unwrap();
            angle_close(m2, m, 1e-12, "M -> nu -> M");
        }
    }

    #[test]
    fn kepler_rejects_bad_eccentricity_and_non_convergence() {
        assert_eq!(solve_kepler(1.0, 1.0), Err(OrbitError::InvalidElements));
        assert_eq!(solve_kepler(1.0, -0.1), Err(OrbitError::InvalidElements));
        assert_eq!(
            solve_kepler_limited(0.1, 0.9, 1),
            Err(OrbitError::NonConvergence)
        );
    }

    #[test]
    fn circular_orbit_state() {
        let a = 7000.0;
        let el = Keplerian {
            semi_major_axis_km: a,
            eccentricity: 0.0,
            inclination_rad: 51.6_f64.to_radians(),
            raan_rad: 1.0,
            arg_perigee_rad: 0.0,
            anomaly: Anomaly::True(0.0),
            epoch: EPOCH,
        };
        let v_circ = (WGS72_MU_KM3_S2 / a).sqrt();
        for k in 0..12 {
            let nu = TAU * f64::from(k) / 12.0;
            let el_k = Keplerian {
                anomaly: Anomaly::True(nu),
                ..el
            };
            let s = el_k.to_state().unwrap();
            assert_close(norm_of(s.position_km), a, 1e-9, "circular radius");
            assert_close(norm_of(s.velocity_km_s), v_circ, 1e-12, "circular speed");
            assert_close(dot(s.position_km, s.velocity_km_s), 0.0, 1e-8, "r.v = 0");
        }
    }

    #[test]
    fn vis_viva_holds_for_random_elements() {
        let mut rng = Rng(0x1234_5678_9ABC_DEF1);
        for _ in 0..300 {
            let el = Keplerian {
                semi_major_axis_km: rng.uniform(6_700.0, 40_000.0),
                eccentricity: rng.uniform(0.0, 0.9),
                inclination_rad: rng.uniform(0.0, PI),
                raan_rad: rng.uniform(0.0, TAU),
                arg_perigee_rad: rng.uniform(0.0, TAU),
                anomaly: Anomaly::Mean(rng.uniform(0.0, TAU)),
                epoch: EPOCH,
            };
            let s = el.to_state().unwrap();
            let r = norm_of(s.position_km);
            let v2 = dot(s.velocity_km_s, s.velocity_km_s);
            let expected = WGS72_MU_KM3_S2 * (2.0 / r - 1.0 / el.semi_major_axis_km);
            assert!(
                (v2 - expected).abs() <= 1e-10 * expected,
                "vis-viva: {v2} vs {expected}"
            );
        }
    }

    #[test]
    fn elements_round_trip_through_state() {
        let mut rng = Rng(0xDEAD_BEEF_0BAD_F00D);
        for _ in 0..1000 {
            let el = Keplerian {
                semi_major_axis_km: rng.uniform(6_800.0, 50_000.0),
                eccentricity: rng.uniform(0.01, 0.9),
                inclination_rad: rng.uniform(0.05, PI - 0.05),
                raan_rad: rng.uniform(0.0, TAU),
                arg_perigee_rad: rng.uniform(0.0, TAU),
                anomaly: Anomaly::True(rng.uniform(0.0, TAU)),
                epoch: EPOCH,
            };
            let s = el.to_state().unwrap();
            let back = Keplerian::from_state(&s, EPOCH).unwrap();
            assert_close(back.semi_major_axis_km, el.semi_major_axis_km, 1e-7, "a");
            assert_close(back.eccentricity, el.eccentricity, 1e-12, "e");
            assert_close(back.inclination_rad, el.inclination_rad, 1e-12, "i");
            angle_close(back.raan_rad, el.raan_rad, 1e-11, "RAAN");
            angle_close(back.arg_perigee_rad, el.arg_perigee_rad, 1e-10, "argp");
            let (Anomaly::True(nu_back), Anomaly::True(nu)) = (back.anomaly, el.anomaly) else {
                unreachable!()
            };
            angle_close(nu_back, nu, 1e-10, "nu");
        }
    }

    #[test]
    fn state_round_trip_through_elements() {
        let mut rng = Rng(0x0F0F_1234_5678_9ABC);
        for _ in 0..1000 {
            let el = Keplerian {
                semi_major_axis_km: rng.uniform(6_800.0, 50_000.0),
                eccentricity: rng.uniform(0.0, 0.9),
                inclination_rad: rng.uniform(0.0, PI),
                raan_rad: rng.uniform(0.0, TAU),
                arg_perigee_rad: rng.uniform(0.0, TAU),
                anomaly: Anomaly::Mean(rng.uniform(0.0, TAU)),
                epoch: EPOCH,
            };
            let s = el.to_state().unwrap();
            let el2 = Keplerian::from_state(&s, EPOCH).unwrap();
            let s2 = el2.to_state().unwrap();
            for k in 0..3 {
                assert_close(s.position_km[k], s2.position_km[k], 1e-6, "r round trip");
                assert_close(
                    s.velocity_km_s[k],
                    s2.velocity_km_s[k],
                    1e-9,
                    "v round trip",
                );
            }
        }
    }

    #[test]
    fn degenerate_geometry_round_trips() {
        // Circular equatorial orbit.
        let s = Keplerian {
            semi_major_axis_km: 7000.0,
            eccentricity: 0.0,
            inclination_rad: 0.0,
            raan_rad: 0.0,
            arg_perigee_rad: 0.0,
            anomaly: Anomaly::True(0.7),
            epoch: EPOCH,
        }
        .to_state()
        .unwrap();
        let el = Keplerian::from_state(&s, EPOCH).unwrap();
        let back = el.to_state().unwrap();
        for k in 0..3 {
            assert_close(s.position_km[k], back.position_km[k], 1e-8, "circ eq r");
            assert_close(
                s.velocity_km_s[k],
                back.velocity_km_s[k],
                1e-11,
                "circ eq v",
            );
        }
        // Elliptic equatorial orbit, retrograde.
        let s = Keplerian {
            semi_major_axis_km: 9000.0,
            eccentricity: 0.2,
            inclination_rad: PI,
            raan_rad: 0.0,
            arg_perigee_rad: 0.4,
            anomaly: Anomaly::True(1.1),
            epoch: EPOCH,
        }
        .to_state()
        .unwrap();
        let el = Keplerian::from_state(&s, EPOCH).unwrap();
        let back = el.to_state().unwrap();
        for k in 0..3 {
            assert_close(s.position_km[k], back.position_km[k], 1e-8, "eq r");
            assert_close(s.velocity_km_s[k], back.velocity_km_s[k], 1e-11, "eq v");
        }
        assert_close(el.inclination_rad, PI, 1e-12, "retrograde inclination");
    }

    #[test]
    fn from_state_rejects_degenerate_and_unbound_states() {
        let zero = StateVector {
            position_km: [0.0; 3],
            velocity_km_s: [1.0, 0.0, 0.0],
        };
        assert_eq!(
            Keplerian::from_state(&zero, EPOCH),
            Err(OrbitError::DegenerateState)
        );
        let radial = StateVector {
            position_km: [7000.0, 0.0, 0.0],
            velocity_km_s: [1.0, 0.0, 0.0],
        };
        assert_eq!(
            Keplerian::from_state(&radial, EPOCH),
            Err(OrbitError::DegenerateState)
        );
        let escape = StateVector {
            position_km: [7000.0, 0.0, 0.0],
            velocity_km_s: [0.0, 15.0, 0.0],
        };
        assert_eq!(
            Keplerian::from_state(&escape, EPOCH),
            Err(OrbitError::InvalidElements)
        );
    }

    #[test]
    fn checksum_matches_published_lines() {
        assert_eq!(tle_checksum(VANGUARD_L1), 3);
        assert_eq!(tle_checksum(VANGUARD_L2), 7);
    }

    #[test]
    fn checksum_is_enforced() {
        // Change the epoch day's last digit; the checksum no longer matches.
        let bad = VANGUARD_L1.replace("179.78495062", "179.78495063");
        assert_eq!(
            Tle::parse(&bad, VANGUARD_L2),
            Err(OrbitError::TleChecksum { line: 1 })
        );
        // A minus sign counts as 1: flip a digit into a minus sign.
        let bad2 = VANGUARD_L2.replace("34.2682", "-4.2682");
        assert_eq!(
            Tle::parse(VANGUARD_L1, &bad2),
            Err(OrbitError::TleChecksum { line: 2 })
        );
    }

    #[test]
    fn parses_fixed_columns() {
        let tle = Tle::parse(VANGUARD_L1, VANGUARD_L2).unwrap();
        assert_eq!(tle.satellite_number, 5);
        assert_eq!(tle.classification, 'U');
        assert_eq!(tle.international_designator, "58002B");
        assert_eq!(tle.epoch_year, 2000);
        assert_close(tle.epoch_day, 179.784_950_62, 1e-12, "epoch day");
        assert_close(tle.mean_motion_dot, 0.000_000_23, 1e-15, "ndot/2");
        assert_close(tle.mean_motion_ddot, 0.0, 0.0, "nddot");
        assert_close(tle.bstar, 2.8098e-5, 1e-15, "bstar");
        assert_eq!(tle.ephemeris_type, 0);
        assert_eq!(tle.element_set_number, 475);
        assert_close(tle.inclination_deg, 34.2682, 1e-12, "inclination");
        assert_close(tle.raan_deg, 348.7242, 1e-12, "raan");
        assert_close(tle.eccentricity, 0.185_966_7, 1e-12, "eccentricity");
        assert_close(tle.arg_perigee_deg, 331.7664, 1e-12, "argp");
        assert_close(tle.mean_anomaly_deg, 19.3264, 1e-12, "M");
        assert_close(tle.mean_motion_rev_day, 10.824_191_57, 1e-12, "n");
        assert_eq!(tle.revolution_number, 41366);
    }

    #[test]
    fn implied_exponent_fields() {
        assert_close(
            parse_implied_exponent(" 28098-4").unwrap(),
            2.8098e-5,
            1e-18,
            "+",
        );
        assert_close(
            parse_implied_exponent("-11606-4").unwrap(),
            -1.1606e-5,
            1e-18,
            "-",
        );
        assert_close(
            parse_implied_exponent(" 00000-0").unwrap(),
            0.0,
            0.0,
            "zero",
        );
        assert!(parse_implied_exponent("28098-4").is_none());
    }

    #[test]
    fn epoch_jd_is_correct_for_known_date() {
        // 2000-01-01 00:00 UT is JD 2451544.5; day 1.0 of 2000 is that instant.
        assert_close(jd_jan_first(2000), 2_451_544.5, 0.0, "2000 Jan 1");
        let tle = Tle::parse(VANGUARD_L1, VANGUARD_L2).unwrap();
        // 2000 is the year, day 179.78495062 is 28 June 2000 at about 18:50 UT.
        assert_close(
            tle.epoch_jd().to_f64(),
            2_451_544.5 + 178.784_950_62,
            1e-8,
            "epoch JD",
        );
    }

    #[test]
    fn sgp4_matches_published_reference_states() {
        let sat = Sgp4::from_tle_lines(VANGUARD_L1, VANGUARD_L2).unwrap();
        for (t, r_ref, v_ref) in VANGUARD_REF {
            let s = sat.propagate(t).unwrap();
            for k in 0..3 {
                // The reference is printed to 1e-8 km and 1e-9 km/s.
                assert_close(s.position_km[k], r_ref[k], 2e-8, "reference r");
                assert_close(s.velocity_km_s[k], v_ref[k], 2e-9, "reference v");
            }
        }
    }

    #[test]
    fn deep_space_orbit_initialises_and_propagates() {
        // Two revolutions per day: period 720 minutes, so the SDP4 terms apply.
        let l1 = "1 00900U 64063A   21001.00000000  .00000000  00000-0  00000-0 0  9995";
        let l2 = "2 00900  50.0000 100.0000 0010000 100.0000 260.0000  2.00000000    00";
        let l1 = with_checksum(l1);
        let l2 = with_checksum(l2);
        let sat = Sgp4::from_tle_lines(&l1, &l2).expect("deep-space orbit initialises");
        let state = sat.propagate(0.0).expect("propagates at epoch");
        let r = (state.position_km[0].powi(2)
            + state.position_km[1].powi(2)
            + state.position_km[2].powi(2))
        .sqrt();
        // A 720-minute, low-eccentricity orbit has a radius of about 26,500 km.
        assert!(r > 20_000.0 && r < 35_000.0, "radius {r} km");
    }

    #[test]
    fn gstime_at_j2000_is_the_sgp4_sidereal_time() {
        // SGP4's gstime at 2000-01-01 12h TT: about 280.46 degrees.
        let g = super::sdp4::gstime(2_451_545.0).to_degrees();
        assert!((g - 280.460_618).abs() < 1e-3, "gstime {g} deg");
    }

    /// Replaces column 69 with the computed checksum (for synthetic lines).
    fn with_checksum(line: &str) -> String {
        let body: String = line.chars().take(68).collect();
        format!("{body}{}", tle_checksum(&body))
    }

    #[test]
    fn decayed_orbit_returns_error() {
        // Mean motion 15.9 rev/day with e = 0.2: perigee is inside the Earth.
        // At epoch the mean anomaly is 260 degrees, so the satellite is above
        // the surface; it reaches perigee (mean anomaly 360 degrees) after
        // about 100 / (15.9 * 360 / 1440) = 25.2 minutes.
        let l1 = "1 99999U 99999A   20001.00000000  .00000000  00000-0  00000-0 0  9990";
        let l2 = "2 99999  51.6000 100.0000 2000000 100.0000 260.0000 15.90000000    00";
        let sat = Sgp4::from_tle_lines(&with_checksum(l1), &with_checksum(l2)).unwrap();
        assert!(sat.propagate(0.0).is_ok());
        assert_eq!(sat.propagate(25.2), Err(OrbitError::Decayed));
    }

    #[test]
    fn malformed_line_is_rejected() {
        assert_eq!(
            Tle::parse("1 short", VANGUARD_L2),
            Err(OrbitError::InvalidTle { line: 1 })
        );
        assert_eq!(
            Tle::parse(
                VANGUARD_L1,
                "3 not a line 2 at all, long enough to pass the length check........................"
            ),
            Err(OrbitError::InvalidTle { line: 2 })
        );
    }

    #[test]
    fn satellite_numbers_must_match() {
        let l2 = VANGUARD_L2.replace("00005", "00006");
        let l2 = with_checksum(&l2);
        assert_eq!(
            Tle::parse(VANGUARD_L1, &l2),
            Err(OrbitError::TleSatelliteMismatch)
        );
    }

    #[test]
    fn gibbs_recovers_the_middle_velocity_of_a_kepler_orbit() {
        // A moderately eccentric LEO-to-GEO-ish orbit; three true anomalies, 40 degrees apart.
        let base = |nu: f64| Keplerian {
            semi_major_axis_km: 9_000.0,
            eccentricity: 0.2,
            inclination_rad: 0.9,
            raan_rad: 0.4,
            arg_perigee_rad: 1.1,
            anomaly: Anomaly::True(nu),
            epoch: JulianDate::new(2_451_545.0, 0.0),
        };
        let s1 = base(0.2).to_state().unwrap();
        let s2 = base(0.9).to_state().unwrap();
        let s3 = base(1.6).to_state().unwrap();
        let v = gibbs_velocity(
            s1.position_km,
            s2.position_km,
            s3.position_km,
            WGS72_MU_KM3_S2,
        )
        .unwrap();
        for (k, vk) in v.iter().enumerate() {
            assert!(
                (vk - s2.velocity_km_s[k]).abs() < 1e-8,
                "component {k}: {} vs {}",
                v[k],
                s2.velocity_km_s[k]
            );
        }
        // Degenerate inputs are rejected.
        let z = [0.0; 3];
        assert_eq!(
            gibbs_velocity(z, s2.position_km, s3.position_km, WGS72_MU_KM3_S2),
            Err(OrbitError::DegenerateState)
        );
        assert_eq!(
            gibbs_velocity(s1.position_km, s2.position_km, s3.position_km, 0.0),
            Err(OrbitError::InvalidElements)
        );
    }

    #[test]
    fn herrick_gibbs_error_falls_with_spacing() {
        // LEO-like orbit; three states centred on mean anomaly 0.5 rad, spaced
        // by `h` seconds. The velocity error must shrink quickly with `h`.
        let elements = Keplerian {
            semi_major_axis_km: 7_000.0,
            eccentricity: 0.01,
            inclination_rad: 0.9,
            raan_rad: 0.4,
            arg_perigee_rad: 1.1,
            anomaly: Anomaly::Mean(0.5),
            epoch: JulianDate::new(2_451_545.0, 0.0),
        };
        let n = elements.mean_motion_rad_s();
        let mut errors = Vec::new();
        for h in [60.0_f64, 30.0, 15.0] {
            let at = |dt: f64| {
                let mut e = elements;
                e.anomaly = Anomaly::Mean(0.5 + n * dt);
                e.to_state().unwrap()
            };
            let (s1, s2, s3) = (at(-h), at(0.0), at(h));
            let v = herrick_gibbs_velocity(
                s1.position_km,
                s2.position_km,
                s3.position_km,
                -h,
                0.0,
                h,
                WGS72_MU_KM3_S2,
            )
            .unwrap();
            let err = norm3(add3(v, scale3(s2.velocity_km_s, -1.0)));
            errors.push(err);
        }
        // Halving the spacing cuts the error by about 2^4 = 16 (measured: 2.9e-6,
        // 1.8e-7, 1.2e-8 km/s). Accept a factor of at least 10 per halving.
        for pair in errors.windows(2) {
            assert!(
                pair[1] * 10.0 < pair[0],
                "errors {errors:?} do not fall fast enough"
            );
        }
        assert!(errors[2] < 1e-7, "15 s spacing error {}", errors[2]);

        let r = [7_000.0, 0.0, 0.0];
        assert_eq!(
            herrick_gibbs_velocity(r, r, r, 0.0, 0.0, 1.0, WGS72_MU_KM3_S2),
            Err(OrbitError::InvalidElements)
        );
        assert_eq!(
            herrick_gibbs_velocity([0.0; 3], r, r, 0.0, 1.0, 2.0, WGS72_MU_KM3_S2),
            Err(OrbitError::DegenerateState)
        );
    }
}
