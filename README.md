# tpt-planetary

A pure Rust engine for planetary-scale N-dimensional data and spherical
mathematics. It is the zero-FFI replacement for Astropy (time scales, coordinate
frames, ephemerides) and for NetCDF-C and HDF5 (chunked array I/O).

**Status:** Early development. Phases 0 to 2 are partly implemented: the
array engine reads and writes local Zarr v2/v3 and NetCDF-3, the time and frame
layer covers UTC/TAI/TT and some frames, and `tptplan` has three working
subcommands. Several items are partial, and most numerical results have not
been checked against reference software. The Python bindings build and pass their
test suite locally, but no wheels are published yet.
See the [user guide](docs/guide/index.md) for what works and what does not
([status and limitations](docs/guide/status-and-limitations.md)), and
`todo.md` for the roadmap.

## Documentation

- [User guide](docs/guide/index.md): getting started, time and frames, arrays,
  the command line, and status and limitations.
- [Architecture decision records](docs/adr/) (ADR-001 to ADR-003).
- [Dependency audit notes](docs/dependencies.md).
- `spec.txt` for the design specification and `todo.md` for the roadmap.

## Goals

- **Permissive licensing.** Every dependency must be MIT, BSD, Zlib, Unicode,
  ISC or dual MIT/Apache-2.0. GPL, LGPL, AGPL, MPL and Apache-2.0-only crates
  are rejected by `cargo deny`.
- **Zero FFI.** No bindings to HDF5, NetCDF, ERFA or other C libraries.
  Zarr is implemented natively.
- **Cloud-native (goal).** Lazy, chunk-streamed evaluation. Streaming from
  local Zarr works now; object-storage stores are not implemented yet.
- **`no_std` core.** `tpt-planetary-core` runs on embedded and flight-software
  targets.

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
