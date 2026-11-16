# Arrays

`tpt-planetary-array` provides chunked N-dimensional arrays, Zarr v2 and v3
metadata and chunk I/O, pure-Rust codecs, a lazy expression graph with
chunk-streamed reductions, and a NetCDF-3 reader and writer with CF time
decoding.

The crate is `no_std` with `alloc` when its default `std` feature is
disabled. The store implementations, `cos_lat_weights`, and the NetCDF-3
module need `std`.

## Stores

The `Store` trait is synchronous. It maps `/`-separated keys to bytes, and it
has two implementations, both behind `std`:

- `FsStore::open(root)`: a local directory. Writes go to a temporary sibling
  file that is renamed into place.
- `MemoryStore::new()`: an in-memory store for tests.

There is no S3, HTTP or other network store, and no async trait. See
[Status and limitations](status-and-limitations.md).

## Zarr v2 and v3

- Arrays and groups are read and written as Zarr v2 (`.zarray`, `.zgroup`,
  `.zattrs`) or Zarr v3 (`zarr.json`). `ArrayMetadata` is the format-neutral
  model, and `ZarrFormat` selects the on-disk form.
- Consolidated v2 metadata (`.zmetadata`) can be written and read.
- Chunk keys use the default `c/0/1` encoding or the v2 `0.1` encoding. The
  v2 dimension separator is `.` by default.
- Dimension names are stored as `_ARRAY_DIMENSIONS` attributes in v2.
- Partial-chunk writes are supported. Unwritten chunks read back as the fill
  value; this was checked for float and integer fills in both v2 and v3.

### Codecs

The registry in `tpt_planetary_array::codec::Registry::builtin()` provides:

| Codec | Format | Implementation |
|-------|--------|----------------|
| `gzip` | v2 compressor, v3 `bytes->bytes` | `miniz_oxide` |
| `zlib` | v2 compressor, v3 `bytes->bytes` | `miniz_oxide` |
| `lz4` | v2 compressor (4-byte size prefix, Zarr v2 layout) | `lz4_flex` |
| `shuffle` | v2 filter | pure Rust |
| `delta` | v2 filter | pure Rust |

Zarr v3 `bytes` codec endianness is read and written. Not implemented: v3
`transpose`, the v3 sharding codec, zstd, Blosc, and bitshuffle. The codec
registry can take additional factories with `Registry::register`.

### Write and read

This example is the crate-level doctest in
`tpt-planetary-array/src/lib.rs`, so `cargo test -p tpt-planetary-array --doc`
compiles and runs it:

```rust
use tpt_planetary_array::{Array, MemoryStore, Selection, Slice};
use tpt_planetary_array::meta::{ArrayMetadata, ZarrFormat};
use tpt_planetary_array::{DType, FillValue};
use std::sync::Arc;

let store = Arc::new(MemoryStore::new());
let meta = ArrayMetadata::new(ZarrFormat::V2, vec![4, 6], vec![2, 3], DType::FLOAT64)
    .unwrap()
    .with_fill_value(FillValue::Float(f64::NAN));
let array = Array::create(store.clone(), "temperature", meta).unwrap();

let values: Vec<f64> = (0..24).map(f64::from).collect();
array.write_f64(&[Selection::ALL, Selection::ALL], &values).unwrap();

let row = array
    .read_f64(&[Selection::Index(1), Selection::Range(Slice::range(0, 3))])
    .unwrap();
assert_eq!(row, vec![6.0, 7.0, 8.0]);
```

`Array::open` reads the metadata and builds the codec pipeline. Reading and
writing are per selection; `read_bytes` and `write_bytes` give access to the
raw element bytes.

## Lazy expressions and reductions

`Expr::from_array(array)` builds a leaf node from an opened array. Creating it
reads no chunk data. Operations build a graph, and nothing is computed until a
reduction runs:

- structural: `slice`, `rolling_mean` and `block_mean` along one axis;
- scalar arithmetic: `add_scalar`, `mul_scalar`;
- elementwise: `arith` (`ArithOp::Add`, `Sub`, `Mul`, `Div`, and `Pow` with
  `std`), `compare`, `unary`,
  `valid_mask`, and `Expr::select` (where);
- reductions: `sum`, `count`, `mean`, `min`, `max`, `std` (Welford),
  `stats`, and `reduce_axes` over chosen axes, with an option to skip
  fill-valued and NaN elements;
- spherical weighting: `weighted_mean(weights)`, with
  `ops::spherical::cos_lat_weights` for cos(latitude) weights.

Reductions stream the output in blocks of at most `EvalOptions::block_elements`
elements (default 2^18), so memory is bounded by the block size and the
reduced output, not by the array size.

The example below was run in this workspace, using in-memory expressions:

```rust
use tpt_planetary_array::Expr;
use tpt_planetary_array::ops::Reduction;

let e = Expr::from_values(vec![2, 3], vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap();
let y = e.mul_scalar(2.0).add_scalar(1.0);
println!("mean(2x+1) = {:?}", y.mean().unwrap());
let r = e.reduce_axes(&[1], Reduction::Sum, false).unwrap();
println!("row sums shape={:?} values={:?}", r.shape, r.values);
println!("max = {:?}", e.max().unwrap());
```

```text
mean(2x+1) = Some(8.0)
row sums shape=[2] values=[6.0, 15.0]
max = Some(6.0)
```

The weighted mean of a box from an opened Zarr array is the central step of
the data-flow test, `tpt-planetary-cli/tests/dataflow.rs`. The relevant lines
are:

```rust
let weights = Expr::from_values(
    vec![box_lats.len() as u64, 1],
    cos_lat_weights(&box_lats),
)
.expect("weights");
let weighted_mean = box_expr
    .weighted_mean(&weights)
    .expect("weighted mean")
    .expect("non-zero weight");
```

Weights of shape `[nlat, 1]` broadcast against a `[nt, nlat, nlon]` field.
`weighted_mean` returns `None` if no element is valid or the total weight is
zero.

Memory is bounded in the sense that each reduction holds one block, and
`stats_with` returns `ExecStats` counters that tests can read. There is no
allocator-level measurement. Parallel chunk execution is not implemented.

## NetCDF-3

`tpt_planetary_array::netcdf3` reads and writes NetCDF-3 Classic (CDF-1) and
64-bit offset (CDF-2) files, in pure Rust. The writer produces a complete file
in a `Vec<u8>`. The reader parses a complete file held in a byte slice.

Supported: dimensions (including one unlimited record dimension), global and
variable attributes, and record variables. Not supported: CDF-5, NetCDF-4 (HDF5)
files, streaming files with `numrecs = 0xFFFFFFFF`, and the NetCDF-4 data
model (groups and user-defined types).

This example is the module-level doctest in
`tpt-planetary-array/src/netcdf3/mod.rs`:

```rust
use tpt_planetary_array::netcdf3::{Format, NcValues, Reader, Writer};

let mut writer = Writer::new(Format::Classic);
let x = writer.add_dimension("x", Some(3)).unwrap();
writer
    .add_variable("v", &[x], Vec::new(), NcValues::Float(vec![1.5, 2.5, 3.5]))
    .unwrap();
let bytes = writer.finish().unwrap();

let reader = Reader::new(&bytes).unwrap();
assert_eq!(reader.read_f64("v").unwrap(), vec![1.5, 2.5, 3.5]);
```

Test coverage uses files generated by SciPy's `netcdf` module
(`tpt-planetary-array/src/netcdf3/testdata`). Files written by netCDF4 or
xarray have not been read.

### CF time coordinates

`netcdf3::cf` parses CF `"<unit> since <reference>"` strings and converts
numeric offsets to calendar dates. Units are seconds, minutes, hours and days.
The supported calendars are:

- `standard` and `gregorian`: Julian before 1582-10-15, Gregorian after;
- `proleptic_gregorian`;
- `noleap` and `365_day`.

Other calendar names, such as `360_day` and `all_leap`, return an
`Unsupported` error. `time_units` and `decode_times` read the time variable
of a `Reader`. The example below was run in this workspace:

```rust
use tpt_planetary_array::netcdf3::cf::{Calendar, TimeUnits};

let units = TimeUnits::parse("days since 1850-01-01", Calendar::Standard).unwrap();
let t = units.decode(60_000.5).unwrap();
println!("decode = {:?}", t);
let units2 = TimeUnits::parse("hours since 2000-01-01 00:00:00", Calendar::NoLeap).unwrap();
println!("noleap decode = {:?}", units2.decode(24.0 * 366.0).unwrap());
```

```text
decode = CfDateTime { year: 2014, month: 4, day: 11, hour: 12, minute: 0, second: 0.0 }
noleap decode = CfDateTime { year: 2001, month: 1, day: 2, hour: 0, minute: 0, second: 0.0 }
```

The `noleap` result shows that 366 days from 2000-01-01 falls on 2001-01-02
in a calendar with no leap days.

The `Array` API and the NetCDF-3 API are separate. There is no conversion
between Zarr and NetCDF-3 through the CLI: `tptplan array convert` converts in
either direction for int8 to float64 variables.
