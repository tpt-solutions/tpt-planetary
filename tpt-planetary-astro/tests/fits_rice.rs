//! Decompresses the tile-compressed images that astropy wrote
//! (tools/gen_fits_rice_fixture.py and tools/gen_fits_dither_fixture.py): RICE_1,
//! GZIP_1 and GZIP_2 integer images, quantized float images with and without
//! dithering, and compares every pixel with astropy's decompression.

use std::fs;
use std::path::Path;

use tpt_planetary_astro::fits::{FitsError, hdus};

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/fits")
            .join(name),
    )
    .expect("fixture")
}

/// (name, shape, pixel values) rows of an expectation table.
fn expected(table: &str) -> Vec<(String, String, Vec<f64>)> {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/fits")
            .join(table),
    )
    .expect("expected values")
    .lines()
    .map(|line| {
        let mut parts = line.split('\t');
        let name = parts.next().expect("name").to_string();
        let shape = parts.next().expect("shape").to_string();
        let values = parts
            .next()
            .expect("values")
            .split(' ')
            .map(|v| {
                if v == "nan" {
                    f64::NAN
                } else {
                    v.parse().expect("number")
                }
            })
            .collect();
        (name, shape, values)
    })
    .collect()
}

fn check(file: &str, name: &str, table: &str) {
    let bytes = fixture(file);
    let hdu = hdus(&bytes).nth(1).expect("extension").expect("valid HDU");
    assert!(hdu.is_compressed_image(), "{name}: not a compressed image");
    let got = hdu.compressed_values().unwrap();
    let (_, shape, want) = expected(table)
        .into_iter()
        .find(|(n, _, _)| n == name)
        .expect("expected row");
    assert_eq!(got.len(), want.len(), "{name}: pixel count ({shape})");
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert!(
            (g.is_nan() && w.is_nan()) || g == w,
            "{name}: pixel {i}: got {g}, want {w}"
        );
    }
}

#[test]
fn a_16_bit_tile_image_matches_astropy() {
    check("rice16.fits", "rice16", "rice_expected.tsv");
}

#[test]
fn a_32_bit_tile_image_matches_astropy() {
    check("rice32.fits", "rice32", "rice_expected.tsv");
}

#[test]
fn gzip_1_tile_images_match_astropy() {
    check("gzip1_16.fits", "gzip1_16", "rice_expected.tsv");
    check("gzip1_32.fits", "gzip1_32", "rice_expected.tsv");
}

#[test]
fn gzip_2_tile_images_match_astropy() {
    check("gzip2_16.fits", "gzip2_16", "rice_expected.tsv");
    check("gzip2_32.fits", "gzip2_32", "rice_expected.tsv");
}

#[test]
fn quantized_float_tile_images_match_astropy() {
    // Quantized to 16 levels per tile with no dithering; NaN is the blank value.
    check("flt_rice.fits", "flt_rice", "rice_expected.tsv");
    check("flt_gzip1.fits", "flt_gzip1", "rice_expected.tsv");
}

#[test]
fn dithered_float_tile_images_match_astropy() {
    // SUBTRACTIVE_DITHER_1 and _2 with ZDITHER0 = 7; dither 2 keeps zeros exact.
    check("dither1.fits", "dither1", "dither_expected.tsv");
    check("dither2.fits", "dither2", "dither_expected.tsv");
}

#[test]
fn a_plain_image_is_not_compressed() {
    let bytes = fixture("rice16.fits");
    // The primary HDU of the fixture is empty and not compressed.
    let primary = hdus(&bytes).next().unwrap().unwrap();
    assert!(!primary.is_compressed_image());
    assert!(matches!(
        primary.compressed_values(),
        Err(FitsError::InvalidHeader(_))
    ));
}
