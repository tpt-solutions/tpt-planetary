#![no_main]

use libfuzzer_sys::fuzz_target;
use tpt_planetary_astro::wcs::Wcs;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        if let Ok(wcs) = Wcs::from_header(text) {
            let _ = wcs.pixel_to_world(0.0, 0.0);
        }
    }
});
