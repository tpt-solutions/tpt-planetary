//! SDP4 (deep-space) states against the `sgp4` Python package.
//!
//! The fixture holds TLEs with periods of 225 minutes or more from the
//! verification set of the sgp4 package, and the TEME states it gives at fixed
//! times, from its pure-Python propagation. The package's C++ build agrees
//! with them to about 1e-9 km on the same set.

use std::fs;
use std::path::Path;

use tpt_planetary_astro::orbit::{OrbitError, Sgp4};

fn fixture() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sgp4_deep_ref.tsv");
    fs::read_to_string(&path).expect("reference file")
}

#[test]
fn deep_space_states_match_the_sgp4_reference() {
    let mut checked = 0usize;
    let mut errors = 0usize;
    let mut max_pos = 0.0f64;
    let mut max_vel = 0.0f64;
    for line in fixture().lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = line.split('\t').collect();
        let sat = Sgp4::from_tle_lines(f[0], f[1]).expect("deep-space TLE initialises");
        let minutes: f64 = f[2].parse().unwrap();
        let result = sat.propagate(minutes);
        match f[3] {
            "ok" => {
                let state = result.unwrap_or_else(|e| panic!("{minutes} min: {e}"));
                let want: Vec<f64> = f[4..10].iter().map(|x| x.parse().unwrap()).collect();
                for k in 0..3 {
                    max_pos = max_pos.max((state.position_km[k] - want[k]).abs());
                    max_vel = max_vel.max((state.velocity_km_s[k] - want[3 + k]).abs());
                }
                checked += 1;
            }
            "err1" | "err3" => {
                assert_eq!(
                    result,
                    Err(OrbitError::EccentricityOutOfRange),
                    "{minutes} min"
                );
                errors += 1;
            }
            "err4" => {
                assert_eq!(
                    result,
                    Err(OrbitError::SemiLatusRectumNegative),
                    "{minutes} min"
                );
                errors += 1;
            }
            other => panic!("unexpected status {other}"),
        }
    }
    assert!(checked > 200, "only {checked} states compared");
    assert!(errors > 0, "no error cases compared");
    // Measured: position 1.2e-9 km and velocity 2e-13 km/s against the reference
    // over 226 states and 14 error cases.
    assert!(max_pos < 1e-6, "position difference {max_pos} km");
    assert!(max_vel < 1e-9, "velocity difference {max_vel} km/s");
}
