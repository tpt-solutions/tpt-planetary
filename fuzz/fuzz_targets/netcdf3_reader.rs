#![no_main]

use libfuzzer_sys::fuzz_target;
use tpt_planetary_array::netcdf3::Reader;

fuzz_target!(|data: &[u8]| {
    if let Ok(reader) = Reader::new(data) {
        let names: Vec<String> = reader.variables().iter().map(|v| v.name.clone()).collect();
        for name in names {
            let _ = reader.read_values(&name);
        }
    }
});
