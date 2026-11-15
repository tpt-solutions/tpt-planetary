//! Physical and astronomical constants.
//!
//! Every constant documents its source. Values are either exact by definition
//! (SI and IAU defining constants) or the published nominal or measured value
//! from the cited resolution or compilation. All quantities are SI unless the
//! name says otherwise.
//!
//! Sources used:
//! - **SI 2019**: BIPM, The International System of Units (9th ed.), exact
//!   defining constants.
//! - **CODATA 2018**: Tiesinga et al., "CODATA recommended values of the
//!   fundamental physical constants: 2018", Rev. Mod. Phys. 93 (2021).
//! - **IAU 2012 B2**: IAU Resolution B2 (2012), astronomical unit defined as
//!   exactly 149 597 870 700 m.
//! - **IAU 2015 B3**: IAU Resolution B3 (2015), nominal solar and planetary
//!   conversion constants.
//! - **WGS84**: NGA.STND.0036 (NIMA TR8350.2), World Geodetic System 1984.
//! - **IERS / SOFA**: TT - TAI = 32.184 s, the definition of Terrestrial Time
//!   (IAU 1991 Recommendation IV).

// ---------------------------------------------------------------------------
// SI and fundamental constants
// ---------------------------------------------------------------------------

/// Speed of light in vacuum, in metres per second.
///
/// Exact by definition (SI 2019; CODATA 2018 value 299 792 458 m s^-1).
pub const SPEED_OF_LIGHT: f64 = 299_792_458.0;

/// Newtonian constant of gravitation, in m^3 kg^-1 s^-2.
///
/// CODATA 2018: 6.67430(15) x 10^-11. Only the central value is stored.
pub const NEWTON_GRAVITATIONAL_CONSTANT: f64 = 6.674_30e-11;

/// Planck constant, in J s.
///
/// Exact by definition (SI 2019).
pub const PLANCK_CONSTANT: f64 = 6.626_070_15e-34;

/// Boltzmann constant, in J K^-1.
///
/// Exact by definition (SI 2019).
pub const BOLTZMANN_CONSTANT: f64 = 1.380_649e-23;

// ---------------------------------------------------------------------------
// Astronomical constants (IAU)
// ---------------------------------------------------------------------------

/// Astronomical unit, in metres.
///
/// IAU 2012 Resolution B2: exactly 149 597 870 700 m.
pub const ASTRONOMICAL_UNIT: f64 = 149_597_870_700.0;

/// Nominal heliocentric gravitational constant GM_sun, in m^3 s^-2.
///
/// IAU 2015 Resolution B3 nominal value 1.3271244 x 10^20 m^3 s^-2.
pub const GM_SUN_NOMINAL: f64 = 1.327_124_4e20;

/// Nominal solar radius, in metres.
///
/// IAU 2015 Resolution B3 nominal value 6.957 x 10^8 m.
pub const SOLAR_RADIUS_NOMINAL: f64 = 6.957e8;

/// Nominal solar luminosity, in watts.
///
/// IAU 2015 Resolution B3 nominal value 3.828 x 10^26 W.
pub const SOLAR_LUMINOSITY_NOMINAL: f64 = 3.828e26;

/// Nominal solar effective temperature, in kelvin.
///
/// IAU 2015 Resolution B3 nominal value 5772 K.
pub const SOLAR_EFFECTIVE_TEMPERATURE_NOMINAL: f64 = 5772.0;

// ---------------------------------------------------------------------------
// Earth (WGS84)
// ---------------------------------------------------------------------------

/// WGS84 semi-major axis (equatorial radius), in metres.
///
/// WGS84 (NGA.STND.0036): exactly 6 378 137.0 m.
pub const WGS84_SEMI_MAJOR_AXIS: f64 = 6_378_137.0;

/// WGS84 inverse flattening 1/f.
///
/// WGS84 (NGA.STND.0036): exactly 298.257223563.
pub const WGS84_INVERSE_FLATTENING: f64 = 298.257_223_563;

/// WGS84 Earth gravitational parameter GM, in m^3 s^-2 (includes atmosphere).
///
/// WGS84 (NGA.STND.0036): 3.986004418 x 10^14 m^3 s^-2.
pub const WGS84_GM: f64 = 3.986_004_418e14;

/// WGS84 nominal angular velocity of the Earth, in rad s^-1.
///
/// WGS84 (NGA.STND.0036): 7.292115 x 10^-5 rad s^-1.
pub const WGS84_ANGULAR_VELOCITY: f64 = 7.292_115e-5;

// ---------------------------------------------------------------------------
// Time constants
// ---------------------------------------------------------------------------

/// Seconds in a (SI) day.
pub const SECONDS_PER_DAY: f64 = 86_400.0;

/// Days in a Julian year (IAU definition: 365.25 days of 86 400 s).
pub const DAYS_PER_JULIAN_YEAR: f64 = 365.25;

/// Days in a Julian century.
pub const DAYS_PER_JULIAN_CENTURY: f64 = 36_525.0;

/// Julian Date of the epoch J2000.0 (2000-01-01T12:00:00 TT).
///
/// Definition of J2000.0 (IAU 1976 / IAU 2006).
pub const JD_J2000: f64 = 2_451_545.0;

/// Offset `JD - MJD` in days, so that `MJD = JD - 2400000.5`.
///
/// Definition of the Modified Julian Date.
pub const MJD_OFFSET: f64 = 2_400_000.5;

/// Constant offset TT - TAI, in seconds.
///
/// IAU 1991 Recommendation IV: TT = TAI + 32.184 s (exact by definition).
pub const TT_MINUS_TAI_SECONDS: f64 = 32.184;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_definitions_match_their_defining_numbers() {
        assert_eq!(SPEED_OF_LIGHT, 299_792_458.0);
        assert_eq!(ASTRONOMICAL_UNIT, 149_597_870_700.0);
        assert_eq!(WGS84_SEMI_MAJOR_AXIS, 6_378_137.0);
        assert_eq!(WGS84_INVERSE_FLATTENING, 298.257_223_563);
        assert_eq!(SECONDS_PER_DAY, 86_400.0);
        assert_eq!(DAYS_PER_JULIAN_YEAR * SECONDS_PER_DAY, 31_557_600.0);
        assert_eq!(DAYS_PER_JULIAN_CENTURY, 36_525.0);
        assert_eq!(TT_MINUS_TAI_SECONDS, 32.184);
    }

    #[test]
    fn wgs84_semi_minor_axis_matches_published_value() {
        // WGS84 defines b = 6 356 752.314 245 m (NGA.STND.0036).
        let a = WGS84_SEMI_MAJOR_AXIS;
        let f = 1.0 / WGS84_INVERSE_FLATTENING;
        let b = a * (1.0 - f);
        assert!((b - 6_356_752.314_245).abs() < 1e-6, "b = {b}");
    }
}
