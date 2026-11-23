# tpt-planetary

A pure Rust engine for planetary-scale N-dimensional data and spherical
mathematics. It is the zero-FFI replacement for Astropy (time scales, coordinate
frames, ephemerides) and for NetCDF-C and HDF5 (chunked array I/O).

**Status:** Early development, version 0.1.0. Nothing is published yet: not to
crates.io, and the Python bindings are not on PyPI.

- **Arrays:** local Zarr v2 and v3 read and write, including sharding and the
  transpose codec, checked against zarr-python fixtures. Read-only access over
  plain `http://`, and S3-compatible endpoints over `http://` (no TLS yet). NetCDF-3 read and
  write. A read-only HDF5 subset reader is in progress.
- **Time and frames:** UTC, TAI, TT and TDB, with ICRS, Galactic and ecliptic
  transforms. Some of these are checked against ERFA, Astropy or pyerfa; others,
  including several constants and the Earth-Sun series, are not yet validated.
- **Ephemerides:** an SPK reader exists, but the Jupiter accuracy target is not
  yet validated against JPL Horizons.
- **`tptplan`:** the command-line tool has working `array`, `astro` and
  `ephemeris` subcommands. Some options return exit code 3 (not implemented).
- **Python:** the bindings build with maturin and pass their pytest suite
  locally.

None of the four milestones in `todo.md` is met yet. The
[user guide](docs/guide/index.md) says what works and what does not
([status and limitations](docs/guide/status-and-limitations.md)), and
`todo.md` is the roadmap.

## Crates

| Crate | Purpose | README |
| --- | --- | --- |
| `tpt-planetary-core` | `no_std` constants, Julian dates, time scales, spherical math | [README](tpt-planetary-core/README.md) |
| `tpt-planetary-array` | N-D arrays, Zarr v2/v3, NetCDF-3, codecs, lazy expressions | [README](tpt-planetary-array/README.md) |
| `tpt-planetary-astro` | Time helpers, frames, ephemerides, WCS | [README](tpt-planetary-astro/README.md) |
| `tpt-planetary-cli` | The `tptplan` command-line tool | [README](tpt-planetary-cli/README.md) |
| `bindings/tpt-planetary-py` | Python bindings (PyO3) | [README](bindings/tpt-planetary-py/README.md) |

## Documentation

- [User guide](docs/guide/index.md): getting started, time and frames, arrays,
  the command line, and status and limitations.
- [Architecture decision records](docs/adr/) (ADR-001 to ADR-004).
- [Dependency audit notes](docs/dependencies.md) and [validation notes](docs/validation.md).
- [Changelog](CHANGELOG.md).
- `spec.txt` for the design specification and `todo.md` for the roadmap.

## Goals

- **Permissive licensing.** Every dependency must be MIT, BSD, Zlib, Unicode,
  ISC or dual MIT/Apache-2.0. GPL, LGPL, AGPL, MPL and Apache-2.0-only crates
  are rejected by `cargo deny`.
- **Zero FFI.** No bindings to HDF5, NetCDF, ERFA or other C libraries.
  Zarr, the codecs and the time and frame code are implemented natively.
- **Cloud-native (goal).** Lazy, chunk-streamed evaluation with bounded memory.
  Local Zarr works now. HTTP and S3 reads work for plain-HTTP endpoints; HTTPS
  and the instance-metadata credential chain are not implemented.
- **`no_std` core.** `tpt-planetary-core` builds without `std` for embedded and
  flight-software targets.

## Architecture

```
tpt-planetary-core    constants, time scales, spherical math   (no_std)
        |
        +--> tpt-planetary-array   chunked N-D arrays, Zarr, lazy engine
        |
        +--> tpt-planetary-astro   frames, ephemeris, WCS
                    |
                    +--> tpt-planetary-cli    `tptplan` binary
                    +--> bindings/tpt-planetary-py   PyO3 bindings
```

## Building

The toolchain is pinned in `rust-toolchain.toml` (MSRV 1.95).

```sh
cargo build --workspace
cargo test --workspace
cargo test -p tpt-planetary-cli --test dataflow           # end-to-end pipeline test
cargo build -p tpt-planetary-core --no-default-features   # no_std build
```

The Python bindings are built separately with maturin:

```sh
cd bindings/tpt-planetary-py
pip install maturin numpy
maturin develop
pytest tests -q
```

## Licence

Dual-licensed under either of

- MIT licence (`LICENSE-MIT`)
- Apache License, Version 2.0 (`LICENSE-APACHE`)

at your option.

Copyright © 2026 TPT Solutions.

### Contributions

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this work, as defined in the Apache-2.0 licence, shall be
dual-licensed as above, without any additional terms or conditions.
