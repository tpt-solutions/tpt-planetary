//! A Zarr v3 array of the core `float16` data type: written and read back through
//! the array API, with the chunk-by-chunk path and a fill value.

use std::sync::Arc;

use tpt_planetary_array::meta::NodeMetadata;
use tpt_planetary_array::meta::v3::parse_node;
use tpt_planetary_array::{Array, MemoryStore, Selection, Store};

#[test]
fn float16_arrays_round_trip_through_the_array_api() {
    let json = r#"{
        "zarr_format": 3, "node_type": "array",
        "shape": [3, 4], "data_type": "float16",
        "chunk_grid": {"name": "regular", "configuration": {"chunk_shape": [2, 3]}},
        "chunk_key_encoding": {"name": "default", "configuration": {"separator": "/"}},
        "fill_value": "NaN",
        "codecs": [{"name": "bytes", "configuration": {"endian": "little"}}]
    }"#;
    let NodeMetadata::Array(meta) = parse_node(json.as_bytes()).unwrap() else {
        panic!("expected an array");
    };
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let array = Array::create(store, "h", meta).unwrap();
    let data: Vec<f64> = (0..12).map(|i| f64::from(i) * 0.5 - 2.0).collect();
    array
        .write_f64(&[Selection::ALL, Selection::ALL], &data)
        .unwrap();
    let back = array.read_f64(&[Selection::ALL, Selection::ALL]).unwrap();
    assert_eq!(back, data, "exact in float16");
}
