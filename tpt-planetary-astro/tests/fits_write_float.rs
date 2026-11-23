//! Quantized float tile-compressed images written by the crate and read back
//! through its own reader, within the quantization step. With
//! TPT_FITS_FLOAT_OUT set to a directory, the test also writes one file per
//! quantization for an external reader (astropy).

use tpt_planetary_astro::fits::{Quantization, TileMethod, hdus, write_compressed_float_image};

const DIMS: [usize; 2] = [37, 23];
const TILES: [usize; 2] = [16, 8];
const LEVEL: f64 = 64.0;

/// A float image: a smooth field with a few exact zeros and one undefined pixel.
fn image() -> Vec<f32> {
    let mut data: Vec<f32> = (0..DIMS[0] * DIMS[1])
        .map(|i| {
            let x = (i % DIMS[0]) as f32;
            let y = (i / DIMS[0]) as f32;
            0.4 * (x / 6.0).sin() + 0.2 * (y / 4.0).cos() + ((i * 37) % 11) as f32 * 1e-3
        })
        .collect();
    for i in (0..data.len()).step_by(29) {
        data[i] = 0.0;
    }
    data[100] = f32::NAN;
    data
}

fn read_back(bytes: &[u8]) -> Vec<f64> {
    let hdu = hdus(bytes).nth(1).expect("extension").expect("valid HDU");
    assert!(hdu.is_compressed_image());
    hdu.compressed_values().expect("decompressed")
}

fn range(data: &[f32]) -> f64 {
    let (lo, hi) = data
        .iter()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| {
            (lo.min(f64::from(v)), hi.max(f64::from(v)))
        });
    hi - lo
}

const QUANTIZATIONS: [Quantization; 3] = [
    Quantization::NoDither,
    Quantization::SubtractiveDither1,
    Quantization::SubtractiveDither2,
];
const METHODS: [TileMethod; 3] = [TileMethod::Rice1, TileMethod::Gzip1, TileMethod::Gzip2];

#[test]
fn quantized_floats_read_back_within_one_step() {
    let data = image();
    // The tile range is at most the image range, so one global step bounds every tile.
    let step = range(&data) / LEVEL;
    for quantization in QUANTIZATIONS {
        for method in METHODS {
            let bytes =
                write_compressed_float_image(&data, &DIMS, &TILES, method, quantization, LEVEL, 7)
                    .unwrap();
            assert_eq!(bytes.len() % 2880, 0, "whole FITS blocks");
            let got = read_back(&bytes);
            assert_eq!(got.len(), data.len());
            for (i, (&g, &x)) in got.iter().zip(&data).enumerate() {
                if x.is_nan() {
                    assert!(
                        g.is_nan(),
                        "{quantization:?} {method:?}: pixel {i} should be undefined"
                    );
                    continue;
                }
                let want = f64::from(x);
                assert!(
                    (g - want).abs() <= step + 1e-6,
                    "{quantization:?} {method:?}: pixel {i}: got {g}, want {want}, step {step}"
                );
            }
        }
    }
}

#[test]
fn dither_two_keeps_zeros_exact() {
    let data = image();
    let bytes = write_compressed_float_image(
        &data,
        &DIMS,
        &TILES,
        TileMethod::Rice1,
        Quantization::SubtractiveDither2,
        LEVEL,
        3,
    )
    .unwrap();
    let got = read_back(&bytes);
    for (i, &x) in data.iter().enumerate() {
        if x == 0.0 {
            assert_eq!(got[i], 0.0, "pixel {i}");
        }
    }
}

#[test]
fn a_constant_tile_is_exact() {
    let data = vec![2.5f32; 16 * 8];
    for quantization in QUANTIZATIONS {
        let bytes = write_compressed_float_image(
            &data,
            &[16, 8],
            &[16, 8],
            TileMethod::Rice1,
            quantization,
            LEVEL,
            1,
        )
        .unwrap();
        let got = read_back(&bytes);
        assert!(got.iter().all(|&v| v == 2.5), "{quantization:?}");
    }
}

#[test]
fn a_dither_needs_a_positive_seed_and_a_positive_level() {
    let data = image();
    assert!(
        write_compressed_float_image(
            &data,
            &DIMS,
            &TILES,
            TileMethod::Rice1,
            Quantization::SubtractiveDither1,
            LEVEL,
            0
        )
        .is_err()
    );
    assert!(
        write_compressed_float_image(
            &data,
            &DIMS,
            &TILES,
            TileMethod::Rice1,
            Quantization::NoDither,
            0.0,
            1
        )
        .is_err()
    );
}

/// Writes one file per quantization for astropy, in the directory in `TPT_FITS_FLOAT_OUT`.
#[test]
fn writes_files_for_external_checks() {
    let Ok(dir) = std::env::var("TPT_FITS_FLOAT_OUT") else {
        return;
    };
    let data = image();
    for (name, quantization) in [
        ("float_nodither.fits", Quantization::NoDither),
        ("float_dither1.fits", Quantization::SubtractiveDither1),
        ("float_dither2.fits", Quantization::SubtractiveDither2),
    ] {
        let bytes = write_compressed_float_image(
            &data,
            &DIMS,
            &TILES,
            TileMethod::Rice1,
            quantization,
            LEVEL,
            7,
        )
        .unwrap();
        std::fs::write(std::path::Path::new(&dir).join(name), bytes).unwrap();
    }
}
