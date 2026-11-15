//! Time-scale helpers: TDB <-> TT, Earth Rotation Angle, GMST and GAST.
//!
//! Julian dates are passed as two-part values `(hi, lo)` whose sum is the
//! Julian date, following the usual SOFA/ERFA convention. This keeps the
//! fractional-day precision that a single `f64` Julian date would lose.
//!
//! The TDB - TT model uses only the three leading geocentric terms of the
//! Fairhead & Bretagnon series. It is an approximation: the omitted terms
//! contribute on the order of tens of microseconds and the full series is not
//! implemented here. Validation against the full series is tracked as a
//! Phase 2 item.

use core::f64::consts::{PI, TAU};

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

/// Leading terms of the geocentric TDB - TT series.
///
/// Each entry is `(amplitude_s, frequency_rad_per_millennium, phase_rad)`, and
/// the term contributes `amplitude * sin(frequency * t + phase)`, where `t` is
/// Julian millennia since J2000 TT. Only the first three terms are included.
/// Their values are transcribed from the Fairhead & Bretagnon / ERFA `dtdb`
/// leading terms; they have not been checked against the source tables here.
const DTDB_LEADING_TERMS: [(f64, f64, f64); 3] = [
    (1.656_674_564e-3, 6_283.075_849_991, 6.240_054_195),
    (22.417_471e-6, 5_753.384_884_897, 4.296_977_442),
    (13.839_792e-6, 12_566.151_699_983, 6.196_904_410),
];

/// Approximate TDB - TT in seconds at the given TT Julian date.
///
/// Uses only the leading terms listed in the module source (three of the
/// Fairhead & Bretagnon series). The result is an approximation accurate to
/// the level of the omitted terms, not the full series.
///
/// `tt_hi + tt_lo` is the TT Julian date.
#[must_use]
pub fn tdb_minus_tt_seconds(tt_hi: f64, tt_lo: f64) -> f64 {
    let t = ((tt_hi - J2000_JD) + tt_lo) / DAYS_PER_MILLENNIUM;
    DTDB_LEADING_TERMS
        .iter()
        .map(|&(amp, freq, phase)| amp * (freq * t + phase).sin())
        .sum()
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
    let tu = (ut1_hi - J2000_JD) + ut1_lo;
    let turns = 0.779_057_273_264 + 1.002_737_811_911_354_48 * tu;
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
/// equinoxes is approximated by `dpsi * cos(eps)`. `dpsi` is the nutation in
/// longitude and `eps` the mean obliquity, both in radians, and must come from
/// a nutation model supplied by the caller. The complementary terms of the
/// equation of the origins are omitted.
#[must_use]
pub fn gast(gmst_rad: f64, dpsi_rad: f64, mean_obliquity_rad: f64) -> f64 {
    wrap_two_pi(gmst_rad + dpsi_rad * mean_obliquity_rad.cos())
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
    fn dtdb_at_j2000_matches_hand_evaluated_leading_terms() {
        // Evaluate the three leading terms by hand at t = 0:
        // sin(6.240054195), sin(4.296977442), sin(6.196904410).
        let expected = 1.656_674_564e-3 * 6.240_054_195_f64.sin()
            + 22.417_471e-6 * 4.296_977_442_f64.sin()
            + 13.839_792e-6 * 6.196_904_410_f64.sin();
        let got = tdb_minus_tt_seconds(J2000_JD, 0.0);
        assert!((got - expected).abs() < 1e-15);
        // The sum is negative (about -93 microseconds) at J2000 with these terms.
        assert!(got < 0.0 && got > -100e-6);
    }

    #[test]
    fn dtdb_bounded_by_leading_amplitude() {
        // Sweep one decade of centuries; the 3-term sum is bounded by its amplitudes.
        let bound = 1.656_674_564e-3 + 22.417_471e-6 + 13.839_792e-6;
        for k in 0..200 {
            let jd = J2000_JD + f64::from(k) * 37.3;
            assert!(tdb_minus_tt_seconds(jd, 0.0).abs() <= bound + 1e-12);
        }
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
    fn gast_adds_equation_of_equinoxes() {
        let gmst = 1.0;
        let dpsi = 1e-5;
        let eps = 0.409_092_6;
        let g = gast(gmst, dpsi, eps);
        assert!((g - (gmst + dpsi * eps.cos())).abs() < EPS);
        assert!(gast(0.0, 0.0, eps) == 0.0);
    }
}
