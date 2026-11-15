//! Micro-benchmarks for time conversions and spherical geometry.
//!
//! These measure speed only. They do not validate results.

use divan::black_box;
use tpt_planetary_core::angle::Radians;
use tpt_planetary_core::calendar::{CalendarSystem, CivilDate};
use tpt_planetary_core::jd::JulianDate;
use tpt_planetary_core::leap::EMBEDDED;
use tpt_planetary_core::scale::{Epoch, Tai, Utc};
use tpt_planetary_core::sphere::{SphericalCoord, angular_separation};

fn main() {
    divan::main();
}

/// Julian Date to calendar date and back, on the Gregorian calendar.
#[divan::bench]
fn jd_calendar_round_trip(bencher: divan::Bencher) {
    bencher.bench_local(|| {
        let date = CivilDate::new(2024, 2, 29);
        let jd = JulianDate::from_calendar(
            black_box(date),
            CalendarSystem::Gregorian,
            black_box(43_200.25),
        )
        .expect("valid date");
        let (back, secs) = jd.to_calendar(CalendarSystem::Gregorian).expect("in range");
        black_box((back, secs))
    });
}

/// UTC to TAI and back, using the embedded leap-second table.
#[divan::bench]
fn utc_tai_round_trip(bencher: divan::Bencher) {
    let utc = Epoch::<Utc>::from_jd(JulianDate::new(2_457_754.5, 0.25));
    bencher.bench_local(|| {
        let tai: Epoch<Tai> = black_box(utc).to_scale(&EMBEDDED).expect("in table");
        let back: Epoch<Utc> = tai.to_scale(&EMBEDDED).expect("in table");
        black_box(back)
    });
}

/// Great-circle angular separation between two sky positions.
#[divan::bench]
fn great_circle_separation(bencher: divan::Bencher) {
    let a = SphericalCoord::new(Radians(0.1), Radians(0.4));
    let b = SphericalCoord::new(Radians(2.9), Radians(-0.7));
    bencher.bench_local(|| angular_separation(black_box(a), black_box(b)));
}
