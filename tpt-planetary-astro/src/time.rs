//! Time-scale helpers: TDB <-> TT, Earth Rotation Angle, GMST and GAST.
//!
//! Julian dates are passed as two-part values `(hi, lo)` whose sum is the
//! Julian date, following the usual SOFA/ERFA convention. This keeps the
//! fractional-day precision that a single `f64` Julian date would lose.
//!
//! The TDB - TT model is the full geocentric Fairhead & Bretagnon (1990)
//! series, 787 terms as coded in ERFA's `dtdb` (see [`crate::tdb_data`]), plus
//! the JPL-mass adjustment terms that ERFA adds. The topocentric terms of
//! `dtdb` depend on the observer's position and are not included, so the
//! result is the geocentre value.

use core::f64::consts::{PI, TAU};

use crate::tdb_data::{FAIRHEAD_GROUP_ENDS, FAIRHEAD_TERMS};

/// Julian date of the J2000.0 epoch (JD 2451545.0 TT).
pub const J2000_JD: f64 = 2_451_545.0;

/// Days per Julian century.
pub const DAYS_PER_CENTURY: f64 = 36_525.0;

/// Days per Julian millennium.
pub const DAYS_PER_MILLENNIUM: f64 = 365_250.0;

/// Seconds per day.
pub const SECONDS_PER_DAY: f64 = 86_400.0;

/// Radians per arcsecond.
const RAD_PER_ARCSEC: f64 = PI / (180.0 * 3600.0);

/// JPL planetary-mass adjustments to the Fairhead & Bretagnon series, as in
/// ERFA `dtdb`: `(amplitude_s, frequency_rad_per_millennium, phase_rad)`.
const DTDB_MASS_TERMS: [(f64, f64, f64); 4] = [
    (0.000_65e-6, 6_069.776_754, 4.021_194),
    (0.000_33e-6, 213.299_095, 5.543_132),
    (-0.001_96e-6, 6_208.294_251, 5.696_701),
    (-0.001_73e-6, 74.781_599, 2.435_900),
];

/// Quadratic-in-T coefficient of the JPL mass adjustment, in seconds per
/// millennium squared.
const DTDB_MASS_T2: f64 = 0.036_38e-6;

/// Geocentric TDB - TT in seconds at the given TT Julian date.
///
/// Evaluates the full Fairhead & Bretagnon series with the JPL-mass
/// adjustments, as in ERFA `eraDtdb` with the observer at the geocentre. The
/// amplitudes are in seconds; the series has a magnitude of about 1.7 ms.
///
/// `tt_hi + tt_lo` is the TT Julian date.
#[must_use]
pub fn tdb_minus_tt_seconds(tt_hi: f64, tt_lo: f64) -> f64 {
    let t = ((tt_hi - J2000_JD) + tt_lo) / DAYS_PER_MILLENNIUM;
    // Group k holds the terms multiplied by t^k. Sum each group, then combine.
    let mut total = 0.0;
    let mut power = 1.0;
    let mut start = 0;
    for &end in &FAIRHEAD_GROUP_ENDS {
        let group: f64 = FAIRHEAD_TERMS[start..end]
            .iter()
            .map(|&(amp, freq, phase)| amp * (freq * t + phase).sin())
            .sum();
        total += group * power;
        power *= t;
        start = end;
    }
    let mass: f64 = DTDB_MASS_TERMS
        .iter()
        .map(|&(amp, freq, phase)| amp * (freq * t + phase).sin())
        .sum();
    total + mass + DTDB_MASS_T2 * t * t
}

/// Converts a TT Julian date to TDB, returned as a two-part Julian date.
///
/// The offset is applied to the second part, so `hi` is unchanged.
#[must_use]
pub fn tt_to_tdb(tt_hi: f64, tt_lo: f64) -> (f64, f64) {
    let dt_days = tdb_minus_tt_seconds(tt_hi, tt_lo) / SECONDS_PER_DAY;
    (tt_hi, tt_lo + dt_days)
}

/// Converts a TDB Julian date to TT, returned as a two-part Julian date.
///
/// Inverts [`tt_to_tdb`] with a fixed-point iteration. The inverse is accurate
/// to the level of the approximation of [`tdb_minus_tt_seconds`] because the
/// TDB - TT argument varies slowly.
#[must_use]
pub fn tdb_to_tt(tdb_hi: f64, tdb_lo: f64) -> (f64, f64) {
    // First guess: evaluate TDB - TT at the TDB epoch.
    let mut tt_lo = tdb_lo - tdb_minus_tt_seconds(tdb_hi, tdb_lo) / SECONDS_PER_DAY;
    for _ in 0..2 {
        let dt = tdb_minus_tt_seconds(tdb_hi, tt_lo) / SECONDS_PER_DAY;
        tt_lo = tdb_lo - dt;
    }
    (tdb_hi, tt_lo)
}

/// Normalises an angle in radians to the interval `[0, 2*pi)`.
fn wrap_two_pi(angle: f64) -> f64 {
    let r = angle.rem_euclid(TAU);
    // rem_euclid can return TAU itself for tiny negative inputs.
    if r >= TAU { 0.0 } else { r }
}

/// Earth Rotation Angle in radians, in `[0, 2*pi)`, from UT1 (IAU 2000).
///
/// `ERA = 2*pi * (0.7790572732640 + 1.00273781191135448 * Tu)` with
/// `Tu = JD(UT1) - 2451545.0`. The day count is split so that the large
/// integer part of the Julian date does not dilute the fractional day.
#[must_use]
#[allow(clippy::excessive_precision)] // IAU 2000 rate constant kept at full published digits.
pub fn earth_rotation_angle(ut1_hi: f64, ut1_lo: f64) -> f64 {
    // Only the fractional part of Tu enters the turns modulo 1, because
    // 1.00273781191135448 * Tu = Tu + 0.00273781191135448 * Tu and the integer
    // part of Tu drops out. Multiplying the full Tu (about 3.6e4 days for 1900)
    // by 1.0027... before reducing mod 1 loses about 1e-11 rad to rounding.
    let tu = (ut1_hi - J2000_JD) + ut1_lo;
    let turns = 0.779_057_273_264 + tu.fract() + 0.002_737_811_911_354_48 * tu;
    wrap_two_pi(TAU * turns.rem_euclid(1.0))
}

/// Greenwich Mean Sidereal Time in radians, in `[0, 2*pi)`, IAU 2006.
///
/// Formulation: `GMST = ERA(UT1) + p(T)` where `p(T)` is the IAU 2006
/// polynomial in arcseconds (IERS Conventions 2010, Eq. 5.32):
///
/// `p(T) = 0.014506 + 4612.156534 T + 1.3915817 T^2 - 0.00000044 T^3
///         - 0.000029956 T^4 - 0.0000000368 T^5`
///
/// with `T` the Julian centuries of TT since J2000. The ERA part uses the UT1
/// Julian date and the polynomial uses the TT Julian date, both as two-part
/// values. Earth orientation parameters (polar motion, UT1-UTC) are not
/// applied here: the caller supplies UT1 directly.
#[must_use]
pub fn gmst_iau2006(ut1_hi: f64, ut1_lo: f64, tt_hi: f64, tt_lo: f64) -> f64 {
    let t = ((tt_hi - J2000_JD) + tt_lo) / DAYS_PER_CENTURY;
    let poly_arcsec = 0.014_506
        + t * (4_612.156_534
            + t * (1.391_581_7
                + t * (-0.000_000_44 + t * (-0.000_029_956 + t * -0.000_000_036_8))));
    wrap_two_pi(earth_rotation_angle(ut1_hi, ut1_lo) + poly_arcsec * RAD_PER_ARCSEC)
}

/// Greenwich Apparent Sidereal Time in radians, in `[0, 2*pi)`.
///
/// `GAST = GMST + equation of the equinoxes`, where the equation of the
/// equinoxes is `dpsi * cos(eps)`. `dpsi` is the nutation in longitude and
/// `eps` the mean obliquity, both in radians, and must come from a nutation
/// model supplied by the caller. It does not include the complementary terms of
/// the equation of the origins. For a self-contained computation from the time
/// alone, use [`gast_iau2006`], which does include them.
#[must_use]
pub fn gast(gmst_rad: f64, dpsi_rad: f64, mean_obliquity_rad: f64) -> f64 {
    wrap_two_pi(gmst_rad + dpsi_rad * mean_obliquity_rad.cos())
}

/// Greenwich Apparent Sidereal Time in radians, in `[0, 2*pi)`, computed from
/// UT1 and TT with the IAU 2000A_R06 nutation of [`crate::nutation`].
///
/// `GAST = GMST_IAU2006 + equation of the equinoxes`, where the equation of the
/// equinoxes is [`crate::nutation::equation_of_equinoxes`]: `dpsi * cos(eps0)`
/// plus the complementary terms of the equation of the origins. Agrees with
/// `erfa.gst06a` to about 6 uas over 1900-2100, the published-model residual
/// (see `docs/validation.md`).
/// Polar motion and the TIO locator are not applied.
#[must_use]
pub fn gast_iau2006(ut1_hi: f64, ut1_lo: f64, tt_hi: f64, tt_lo: f64) -> f64 {
    let gmst = gmst_iau2006(ut1_hi, ut1_lo, tt_hi, tt_lo);
    wrap_two_pi(gmst + crate::nutation::equation_of_equinoxes(tt_hi, tt_lo))
}

/// Mean obliquity of the ecliptic at J2000.0, in arcseconds (IAU 2006).
///
/// Source: Hilton et al. 2006 (Celest. Mech. Dyn. Astron. 94, 351-367) and
/// IERS Conventions 2010, Eq. 5.40. Recalled from memory, unvalidated.
pub const OBLIQUITY_J2000_ARCSEC_IAU2006: f64 = 84_381.406;

/// Mean obliquity of the ecliptic in arcseconds at a TT epoch (IAU 2006).
///
/// Polynomial in `T`, Julian centuries of TT since J2000 (IERS Conventions
/// 2010, Eq. 5.40, from Hilton et al. 2006):
///
/// `eps0 = 84381.406 - 46.836769 T - 0.0001831 T^2 + 0.00200340 T^3
///         - 0.000000576 T^4 - 0.0000000434 T^5`
///
/// Source: as cited above. The coefficients were recalled from memory and have
/// not been checked against the published tables; treat them as unvalidated.
/// The value at J2000.0 is exactly [`OBLIQUITY_J2000_ARCSEC_IAU2006`].
#[must_use]
pub fn mean_obliquity_iau2006_arcsec(tt_hi: f64, tt_lo: f64) -> f64 {
    let t = ((tt_hi - J2000_JD) + tt_lo) / DAYS_PER_CENTURY;
    OBLIQUITY_J2000_ARCSEC_IAU2006
        + t * (-46.836_769
            + t * (-0.000_183_1 + t * (0.002_003_40 + t * (-0.000_000_576 + t * -0.000_000_043_4))))
}

/// Mean obliquity of the ecliptic in radians at a TT epoch (IAU 2006).
///
/// See [`mean_obliquity_iau2006_arcsec`] for the polynomial and its source.
#[must_use]
pub fn mean_obliquity_iau2006_rad(tt_hi: f64, tt_lo: f64) -> f64 {
    mean_obliquity_iau2006_arcsec(tt_hi, tt_lo) * RAD_PER_ARCSEC
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-12;

    #[test]
    fn obliquity_at_j2000_is_84381_406_arcsec() {
        let eps = mean_obliquity_iau2006_arcsec(J2000_JD, 0.0);
        assert_eq!(eps, 84_381.406);
        assert!(
            (mean_obliquity_iau2006_rad(J2000_JD, 0.0) - 84_381.406 * RAD_PER_ARCSEC).abs() < EPS
        );
    }

    #[test]
    fn obliquity_rate_is_about_46_8_arcsec_per_century() {
        // One Julian century after J2000 the linear term dominates: about -46.84 arcsec.
        let eps = mean_obliquity_iau2006_arcsec(J2000_JD + DAYS_PER_CENTURY, 0.0);
        assert!(
            (eps - (84_381.406 - 46.836_769 - 0.000_183_1 + 0.002_003_40
                - 0.000_000_576
                - 0.000_000_043_4))
                .abs()
                < 1e-9
        );
        assert!((eps - 84_334.571).abs() < 1e-3);
    }

    #[test]
    fn dtdb_matches_erfa_dtdb_reference_values() {
        // Reference values from erfa.dtdb (pyerfa 2.0.1.5), geocentre
        // (ut = elong = u = v = 0). Arguments are (TT hi, TT lo).
        let cases: [(f64, f64, f64); 6] = [
            (2_451_545.0, 0.0, -9.930_719_894_379_447e-5),
            (2_451_545.0, 0.25, -9.207_139_216_018_104e-5),
            (2_415_020.5, 0.0, -1.846_023_201_048_501_5e-5),
            (2_469_807.5, 0.5, -6.589_933_159_075_999e-5),
            (2_488_069.5, 0.0, -8.994_766_298_524_835e-5),
            (2_433_282.5, 0.123_456, -6.711_697_302_277_03e-5),
        ];
        for (hi, lo, expected) in cases {
            let got = tdb_minus_tt_seconds(hi, lo);
            // Agreement to 1 ns; the cases are 10 to 100 us in size.
            assert!(
                (got - expected).abs() < 1e-9,
                "TT {hi} + {lo}: got {got}, erfa {expected}"
            );
        }
    }

    #[test]
    fn dtdb_peak_is_about_one_point_seven_milliseconds() {
        // The series peaks near 1.66 ms, the amplitude of its leading term.
        let mut peak: f64 = 0.0;
        for k in 0..2_000 {
            let jd = J2000_JD + f64::from(k) * 37.3;
            peak = peak.max(tdb_minus_tt_seconds(jd, 0.0).abs());
        }
        assert!(peak > 1.6e-3 && peak < 1.8e-3, "peak {peak}");
    }

    #[test]
    fn tt_tdb_round_trip() {
        for k in 0..50 {
            let hi = 2_458_000.5;
            let lo = f64::from(k) * 0.0137;
            let (tdb_hi, tdb_lo) = tt_to_tdb(hi, lo);
            let (tt_hi, tt_lo) = tdb_to_tt(tdb_hi, tdb_lo);
            // Round trip must recover TT to well below a microsecond (1 us = 1.2e-11 d).
            assert!((tt_hi - hi).abs() < EPS);
            assert!((tt_lo - lo).abs() < 1e-11, "k={k} diff={}", tt_lo - lo);
        }
    }

    #[test]
    fn era_at_j2000_is_280_4606_degrees() {
        // Analytic: 0.7790572732640 turns = 280.46061838 degrees exactly by formula.
        let era = earth_rotation_angle(J2000_JD, 0.0).to_degrees();
        let expected = (0.779_057_273_264 * 360.0) % 360.0;
        assert!((era - expected).abs() < 1e-9);
        assert!((era - 280.460_618_38).abs() < 1e-7);
    }

    #[test]
    fn era_rate_is_one_revolution_per_sidereal_day() {
        // Advancing UT1 by one solar day advances ERA by 360.9856 degrees (mod 360).
        let a = earth_rotation_angle(J2000_JD, 0.25);
        let b = earth_rotation_angle(J2000_JD + 1.0, 0.25);
        let step = (b - a).rem_euclid(TAU).to_degrees();
        // The extra rotation per solar day is 0.00273781191135448 turns = 0.98561229 deg.
        assert!((step - 0.985_612_288).abs() < 1e-6, "step={step}");
    }

    #[test]
    fn gmst_at_j2000_noon_ut1_is_about_18h41m50s() {
        // JD 2451545.0 is 2000 Jan 1 12h UT. Sanity check against the commonly
        // tabulated value GMST = 18h 41m 50.5s there. Not a SOFA/ERFA test vector.
        let (tt_hi, tt_lo) = (J2000_JD, 64.184 / SECONDS_PER_DAY);
        let gmst = gmst_iau2006(J2000_JD, 0.0, tt_hi, tt_lo);
        let seconds = gmst.to_degrees() / 15.0 * 3600.0;
        let expected = 18.0 * 3600.0 + 41.0 * 60.0 + 50.5;
        assert!((seconds - expected).abs() < 0.5, "got {seconds} s");
    }

    #[test]
    fn gast_iau2006_is_gmst_plus_equation_of_equinoxes() {
        let (ut1_hi, ut1_lo) = (2_460_000.5, 0.25);
        let (tt_hi, tt_lo) = (2_460_000.5, 0.25 + 69.184 / SECONDS_PER_DAY);
        let gmst = gmst_iau2006(ut1_hi, ut1_lo, tt_hi, tt_lo);
        let ee = crate::nutation::equation_of_equinoxes(tt_hi, tt_lo);
        let expected = wrap_two_pi(gmst + ee);
        let got = gast_iau2006(ut1_hi, ut1_lo, tt_hi, tt_lo);
        assert!((got - expected).abs() < EPS);
        // The equation of the equinoxes is at most about 18 arcseconds in magnitude.
        assert!(ee.abs() < 20.0 * RAD_PER_ARCSEC);
    }

    #[test]
    fn gast_adds_equation_of_equinoxes() {
        let gmst = 1.0;
        let dpsi = 1e-5;
        let eps = 0.409_092_6;
        let g = gast(gmst, dpsi, eps);
        assert!((g - (gmst + dpsi * eps.cos())).abs() < EPS);
        assert!(gast(0.0, 0.0, eps) == 0.0);
    }
}
