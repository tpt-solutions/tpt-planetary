//! Evaluates the celestial WCS in a FITS header file for the Python validation
//! suite (`bindings/tpt-planetary-py/tests/validation/test_wcs_projections.py`).
//! Not part of the public API.
//!
//! Usage: `wcs_dump <header-file> <p2w|w2p>`
//!
//! - `p2w`: each stdin line is `x y` (FITS 1-based pixel); each stdout line is
//!   `lon lat` in degrees, or `nan nan` outside the projection's domain.
//! - `w2p`: each stdin line is `lon lat` in degrees; each stdout line is `x y`
//!   (FITS 1-based pixel), or `nan nan` outside the domain.

use std::io::{self, BufRead, Write};

use tpt_planetary_astro::wcs::Wcs;

fn parse_two(line: &str) -> Option<(f64, f64)> {
    let mut it = line.split_whitespace().map(str::parse::<f64>);
    let a = it.next()?.ok()?;
    let b = it.next()?.ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((a, b))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: wcs_dump <header-file> <p2w|w2p>");
        std::process::exit(2);
    }
    let text = std::fs::read_to_string(&args[1]).expect("header file readable");
    let wcs = match Wcs::from_header(&text) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("header error: {e}");
            std::process::exit(1);
        }
    };
    let mode = args[2].as_str();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let line = line.expect("stdin readable");
        if line.trim().is_empty() {
            continue;
        }
        let Some((a, b)) = parse_two(&line) else {
            eprintln!("bad input line: {line}");
            std::process::exit(1);
        };
        let result = match mode {
            "p2w" => wcs.pixel_to_world(a, b),
            "w2p" => wcs.world_to_pixel(a, b),
            _ => {
                eprintln!("unknown mode: {mode}");
                std::process::exit(2);
            }
        };
        match result {
            Some((x, y)) => writeln!(out, "{x} {y}"),
            None => writeln!(out, "nan nan"),
        }
        .expect("stdout writable");
    }
}
