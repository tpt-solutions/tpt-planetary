//! Prints SGP4 TEME states for the validation suite. Not part of the public API.
//!
//! Usage: stdin has one record per line, `line1|line2|minutes`, where `line1`
//! and `line2` are the two TLE lines and `minutes` is the time since epoch.
//! For each record one line is printed:
//!
//! - `ok x y z vx vy vz`: position in km and velocity in km/s (TEME), written
//!   with the shortest representation that round-trips.
//! - `err <Variant>`: the [`OrbitError`] debug name, from initialisation or
//!   propagation.
//!
//! Run through `cargo run --example orbit_dump`. The Python test
//! `bindings/tpt-planetary-py/tests/validation/test_orbit.py` drives it.

use std::io::{self, BufRead, Write};

use tpt_planetary_astro::orbit::Sgp4;

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    for line in stdin.lock().lines() {
        let line = line?;
        let fields: Vec<&str> = line.split('|').collect();
        if fields.len() != 3 {
            writeln!(out, "err BadRecord")?;
            continue;
        }
        let Ok(minutes) = fields[2].trim().parse::<f64>() else {
            writeln!(out, "err BadTime")?;
            continue;
        };
        let result = Sgp4::from_tle_lines(fields[0].trim(), fields[1].trim())
            .and_then(|sat| sat.propagate(minutes));
        match result {
            Ok(state) => {
                let [x, y, z] = state.position_km;
                let [vx, vy, vz] = state.velocity_km_s;
                writeln!(out, "ok {x:?} {y:?} {z:?} {vx:?} {vy:?} {vz:?}")?;
            }
            Err(err) => writeln!(out, "err {err:?}")?,
        }
    }
    out.flush()
}
