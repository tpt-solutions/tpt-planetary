//! Micro-benchmarks for frame conversion and WCS pixel-to-world.
//!
//! All inputs are generated deterministically in memory. These measure
//! speed only and do not validate results.

use divan::black_box;
use tpt_planetary_astro::frames::{Galactic, Icrs, SkyCoord};
use tpt_planetary_astro::wcs::{Projection, Wcs, WcsParams};

fn main() {
    divan::main();
}

/// 10,000 ICRS positions on a regular grid across the sky.
fn icrs_grid() -> Vec<SkyCoord<Icrs>> {
    (0..10_000)
        .map(|i| {
            let ra = (i % 100) as f64 * 3.6;
            let dec = (i / 100) as f64 * 1.8 - 90.0;
            SkyCoord::from_degrees(ra, dec.clamp(-89.9, 89.9))
        })
        .collect()
}

/// ICRS to Galactic for 10,000 coordinates.
#[divan::bench]
fn icrs_to_galactic_10k(bencher: divan::Bencher) {
    let coords = icrs_grid();
    bencher.bench_local(|| {
        let mut acc = 0.0_f64;
        for c in black_box(&coords) {
            let g: SkyCoord<Galactic> = c.transform_to();
            acc += g.lon();
        }
        acc
    });
}

/// TAN projection pixel to world for 10,000 pixels (a 100x100 block).
#[divan::bench]
fn wcs_tan_pixel_to_world_10k(bencher: divan::Bencher) {
    let wcs = Wcs::new(WcsParams {
        crpix: [50.5, 50.5],
        crval: [83.63308, 22.01446],
        cd: [[-0.0002, 0.0], [0.0, 0.0002]],
        projection: Projection::Tan,
        lonpole: None,
        latpole: None,
        pv2: [None, None],
    })
    .expect("valid WCS");
    bencher.bench_local(|| {
        let mut acc = 0.0_f64;
        for y in 0..100 {
            for x in 0..100 {
                if let Some((ra, _dec)) =
                    wcs.pixel_to_world(black_box(x as f64), black_box(y as f64))
                {
                    acc += ra;
                }
            }
        }
        acc
    });
}
