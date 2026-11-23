//! `TransformContext::from_utc_and_eop`: TT from the leap-second table, UT1 and
//! polar motion from an EOP table (finals2000A excerpt).

use std::fs;
use std::path::Path;

use tpt_planetary_astro::eop::{EopError, EopTable, FINALS2000A_COLUMNS, parse_finals2000a};
use tpt_planetary_astro::frames::{ContextError, TransformContext};
use tpt_planetary_core::calendar::CivilDate;
use tpt_planetary_core::leap::EMBEDDED;
use tpt_planetary_core::scale::{CivilTime, Epoch, Utc};

fn records() -> Vec<tpt_planetary_astro::eop::EopRecord> {
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/eop")
            .join("finals_excerpt.all"),
    )
    .expect("fixture");
    parse_finals2000a(&text, &FINALS2000A_COLUMNS).unwrap()
}

fn utc(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: f64) -> Epoch<Utc> {
    Epoch::<Utc>::from_civil(
        CivilTime {
            date: CivilDate { year, month, day },
            hour,
            minute,
            second,
        },
        &EMBEDDED,
    )
    .unwrap()
}

fn row(
    records: &[tpt_planetary_astro::eop::EopRecord],
    mjd: f64,
) -> tpt_planetary_astro::eop::EopRecord {
    *records.iter().find(|r| r.mjd == mjd).expect("row")
}

#[test]
fn tt_is_utc_plus_69_184_seconds_in_2017() {
    let records = records();
    let eop = EopTable::new(&records).unwrap();
    let u = utc(2017, 9, 5, 12, 0, 0.0);
    let ctx = TransformContext::from_utc_and_eop(u, &eop, &EMBEDDED).unwrap();
    // TT - UTC = 37 leap seconds (TAI - UTC) + 32.184 s (TT - TAI).
    let tt_minus_utc =
        (ctx.tt_hi - u.jd().day()) * 86400.0 + (ctx.tt_lo - u.jd().fraction()) * 86400.0;
    assert!((tt_minus_utc - 69.184).abs() < 1e-6, "{tt_minus_utc}");
}

#[test]
fn ut1_and_polar_motion_are_the_table_interpolation() {
    let records = records();
    let eop = EopTable::new(&records).unwrap();
    let u = utc(2017, 9, 5, 12, 0, 0.0);
    let ctx = TransformContext::from_utc_and_eop(u, &eop, &EMBEDDED).unwrap();

    // Noon of 2017-09-05 is halfway between the 0h rows of 09-05 and 09-06.
    let a = row(&records, 58001.0);
    let b = row(&records, 58002.0);
    let ut1_minus_utc = (ctx.ut1_lo - u.jd().fraction()) * 86400.0;
    assert!(
        (ut1_minus_utc - (a.dut1_seconds + b.dut1_seconds) / 2.0).abs() < 1e-9,
        "{ut1_minus_utc}"
    );
    let xp_arcsec = ctx.xp / tpt_planetary_astro::eop::ARCSEC_TO_RAD;
    let yp_arcsec = ctx.yp / tpt_planetary_astro::eop::ARCSEC_TO_RAD;
    assert!((xp_arcsec - (a.xp_arcsec + b.xp_arcsec) / 2.0).abs() < 1e-9);
    assert!((yp_arcsec - (a.yp_arcsec + b.yp_arcsec) / 2.0).abs() < 1e-9);
}

#[test]
fn an_epoch_outside_the_eop_table_is_an_eop_error() {
    let records = records();
    let eop = EopTable::new(&records).unwrap();
    // 2017-10-12 lies in a gap of the excerpt.
    let u = utc(2017, 10, 12, 12, 0, 0.0);
    match TransformContext::from_utc_and_eop(u, &eop, &EMBEDDED) {
        Err(ContextError::Eop(EopError::Gap { .. })) => {}
        other => panic!("expected an EOP gap error, got {other:?}"),
    }
}
