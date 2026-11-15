#![no_main]

use libfuzzer_sys::fuzz_target;
use tpt_planetary_array::meta::v2::parse_array;

fuzz_target!(|data: &[u8]| {
    // Without attributes, then with the same bytes as `.zattrs`.
    let _ = parse_array(data, None);
    let _ = parse_array(data, Some(data));
});
