//! Tile-compressed images written by the crate, read back through its own reader.
//! With TPT_FITS_TILE_OUT set to a directory, the test also writes one file per
//! method for an external reader (astropy).

use tpt_planetary_astro::fits::{CompressedPixels, TileMethod, hdus, write_compressed_image};

/// A 2-D image of 37 by 23 pixels: a smooth ramp with a pseudo-random component.
fn image16(n: usize) -> Vec<i16> {
    (0..n)
        .map(|i| {
            let x = (i % 37) as i64;
            let y = (i / 37) as i64;
            let noise = ((i as i64).wrapping_mul(2_654_435_761) >> 7).rem_euclid(101);
            (x * 3 + y * 5 + noise - 300) as i16
        })
        .collect()
}

fn read_back(bytes: &[u8]) -> Vec<f64> {
    let hdu = hdus(bytes).nth(1).expect("extension").expect("valid HDU");
    assert!(hdu.is_compressed_image());
    hdu.compressed_values().expect("decompressed")
}

const METHODS: [TileMethod; 3] = [TileMethod::Rice1, TileMethod::Gzip1, TileMethod::Gzip2];

#[test]
fn signed_16_bit_images_round_trip_by_every_method() {
    let data = image16(37 * 23);
    for method in METHODS {
        let bytes =
            write_compressed_image(CompressedPixels::I16(&data), &[37, 23], &[16, 8], method)
                .unwrap();
        assert_eq!(bytes.len() % 2880, 0, "whole FITS blocks");
        let got = read_back(&bytes);
        let want: Vec<f64> = data.iter().map(|&v| f64::from(v)).collect();
        assert_eq!(got, want, "{method:?}");
    }
}

#[test]
fn unsigned_8_bit_and_signed_32_bit_images_round_trip() {
    let bytes8: Vec<u8> = (0..60u32).map(|i| (i * 37 % 256) as u8).collect();
    let bytes32: Vec<i32> = (0..60i64)
        .map(|i| {
            if i % 5 == 0 {
                i32::MIN
            } else if i % 7 == 0 {
                i32::MAX
            } else {
                (i * 104_729 - 2_000_000) as i32
            }
        })
        .collect();
    for method in METHODS {
        let a = write_compressed_image(CompressedPixels::U8(&bytes8), &[10, 6], &[4, 4], method)
            .unwrap();
        let got8 = read_back(&a);
        assert_eq!(
            got8,
            bytes8.iter().map(|&v| f64::from(v)).collect::<Vec<_>>(),
            "u8 {method:?}"
        );
        let b = write_compressed_image(CompressedPixels::I32(&bytes32), &[10, 6], &[4, 4], method)
            .unwrap();
        let got32 = read_back(&b);
        assert_eq!(
            got32,
            bytes32.iter().map(|&v| f64::from(v)).collect::<Vec<_>>(),
            "i32 {method:?}"
        );
    }
}

#[test]
fn a_three_dimensional_image_with_partial_tiles_round_trips() {
    let dims = [5usize, 4, 3];
    let n: usize = dims.iter().product();
    let data: Vec<i16> = (0..n).map(|i| ((i * 97) % 251) as i16 - 120).collect();
    for method in METHODS {
        let bytes = write_compressed_image(CompressedPixels::I16(&data), &dims, &[2, 3, 2], method)
            .unwrap();
        let got = read_back(&bytes);
        let want: Vec<f64> = data.iter().map(|&v| f64::from(v)).collect();
        assert_eq!(got, want, "{method:?}");
    }
}

#[test]
fn a_pixel_count_that_does_not_match_the_axes_is_rejected() {
    let data = [1i16, 2, 3];
    assert!(
        write_compressed_image(
            CompressedPixels::I16(&data),
            &[2, 2],
            &[1, 1],
            TileMethod::Rice1
        )
        .is_err()
    );
}

/// Writes one file per method to the directory in `TPT_FITS_TILE_OUT`, for astropy.
#[test]
fn writes_files_for_external_checks() {
    let Ok(dir) = std::env::var("TPT_FITS_TILE_OUT") else {
        return;
    };
    let data = image16(37 * 23);
    for (name, method) in [
        ("tile_rice.fits", TileMethod::Rice1),
        ("tile_gzip1.fits", TileMethod::Gzip1),
        ("tile_gzip2.fits", TileMethod::Gzip2),
    ] {
        let bytes =
            write_compressed_image(CompressedPixels::I16(&data), &[37, 23], &[16, 8], method)
                .unwrap();
        std::fs::write(std::path::Path::new(&dir).join(name), bytes).unwrap();
    }
}
