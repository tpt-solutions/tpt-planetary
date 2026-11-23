//! Reads the CK type 2 attitude file written in the tools/gen_ck_fixture.py layout
//! and compares the C-matrix and angular velocity with what SPICE's ckgpav returns.

use std::fs;
use std::path::Path;

use tpt_planetary_astro::ck::CkFile;
use tpt_planetary_astro::ephemeris::EphemerisError;

fn fixture() -> Vec<u8> {
    fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ck/attitude.bc"))
        .expect("fixture")
}

/// Rows of attitude_expected.tsv: (sclk, 9 matrix values, 3 angular velocity values).
fn expected() -> Vec<(f64, Vec<f64>)> {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ck/attitude_expected.tsv"),
    )
    .expect("expected values")
    .lines()
    .skip(1)
    .map(|line| {
        let values: Vec<f64> = line
            .split('\t')
            .map(|v| v.parse().expect("number"))
            .collect();
        (values[0], values[1..].to_vec())
    })
    .collect()
}

#[test]
fn segment_summary_is_read() {
    let bytes = fixture();
    let file = CkFile::new(&bytes).unwrap();
    let segs: Vec<_> = file.segments().map(|s| s.unwrap()).collect();
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].instrument, -999);
    assert_eq!(segs[0].frame, 1);
    assert_eq!(segs[0].data_type, 2);
    assert_eq!(segs[0].start, 1000.0);
    assert_eq!(segs[0].stop, 2000.0);
}

#[test]
fn attitude_matches_spice_at_every_sampled_epoch() {
    let bytes = fixture();
    let file = CkFile::new(&bytes).unwrap();
    let rows = expected();
    assert_eq!(rows.len(), 11);
    for (sclk, values) in rows {
        let att = file.attitude(-999, sclk).unwrap();
        for i in 0..3 {
            for j in 0..3 {
                let want = values[3 * i + j];
                let got = att.matrix[i][j];
                assert!(
                    (got - want).abs() < 1e-12,
                    "sclk {sclk} element ({i},{j}): got {got}, want {want}"
                );
            }
        }
        for k in 0..3 {
            let want = values[9 + k];
            assert!(
                (att.angular_velocity[k] - want).abs() < 1e-12,
                "sclk {sclk} angular velocity {k}"
            );
        }
        assert_eq!(att.frame, 1);
    }
}

/// Matrix of the scalar-first quaternion of a rotation by `angle` about `axis`,
/// computed independently of the crate (the generator's start quaternion of
/// interval 1, which is 0.8 rad about (1, 0.2, -0.4)).
fn second_interval_start_matrix() -> [[f64; 3]; 3] {
    let n = (1.0f64 + 0.04 + 0.16).sqrt();
    let axis = [1.0 / n, 0.2 / n, -0.4 / n];
    let half: f64 = 0.8 / 2.0;
    let (s, c) = (half.sin(), half.cos());
    let (q0, q1, q2, q3) = (c, s * axis[0], s * axis[1], s * axis[2]);
    [
        [
            1.0 - 2.0 * (q2 * q2 + q3 * q3),
            2.0 * (q1 * q2 - q0 * q3),
            2.0 * (q1 * q3 + q0 * q2),
        ],
        [
            2.0 * (q1 * q2 + q0 * q3),
            1.0 - 2.0 * (q1 * q1 + q3 * q3),
            2.0 * (q2 * q3 - q0 * q1),
        ],
        [
            2.0 * (q1 * q3 - q0 * q2),
            2.0 * (q2 * q3 + q0 * q1),
            1.0 - 2.0 * (q1 * q1 + q2 * q2),
        ],
    ]
}

#[test]
fn a_boundary_epoch_takes_the_interval_that_starts_there() {
    let bytes = fixture();
    let file = CkFile::new(&bytes).unwrap();
    // 1500 is the start of the second interval (and the stop of the first), so
    // the attitude there is that interval's start matrix exactly.
    let at = file.attitude(-999, 1500.0).unwrap();
    let want = second_interval_start_matrix();
    for (i, row) in want.iter().enumerate() {
        for (j, &w) in row.iter().enumerate() {
            assert!(
                (at.matrix[i][j] - w).abs() < 1e-12,
                "element ({i},{j}): got {}, want {w}",
                at.matrix[i][j]
            );
        }
    }
}

#[test]
fn the_final_instant_is_covered() {
    let bytes = fixture();
    let file = CkFile::new(&bytes).unwrap();
    assert!(file.attitude(-999, 2000.0).is_ok());
}

#[test]
fn an_unknown_instrument_is_not_found() {
    let bytes = fixture();
    let file = CkFile::new(&bytes).unwrap();
    assert!(matches!(
        file.attitude(-1, 1500.0),
        Err(EphemerisError::NotFound { target: -1 })
    ));
}

#[test]
fn a_time_outside_the_coverage_is_out_of_range() {
    let bytes = fixture();
    let file = CkFile::new(&bytes).unwrap();
    assert!(matches!(
        file.attitude(-999, 999.0),
        Err(EphemerisError::OutOfRange { .. })
    ));
    assert!(matches!(
        file.attitude(-999, 2001.0),
        Err(EphemerisError::OutOfRange { .. })
    ));
}

#[test]
fn an_spk_file_is_not_a_ck_file() {
    let mut bytes = fixture();
    bytes[..8].copy_from_slice(b"DAF/SPK ");
    assert!(matches!(CkFile::new(&bytes), Err(EphemerisError::NotSpk)));
}
