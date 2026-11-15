# tpt-planetary user guide

tpt-planetary is a pure Rust library and command-line tool for planetary-scale
N-dimensional data (Zarr and NetCDF-3) and for time scales, coordinate frames
and ephemerides. It has no C or Fortran dependencies.

The project is early. Several components are partial, and some numerical
results have not been checked against reference software. Read
[Status and limitations](status-and-limitations.md) before relying on any
result.

## Pages

- [Getting started](getting-started.md): building, testing, the toolchain pin
  and the MSRV.
- [Time and frames](time-and-frames.md): `tpt-planetary-core` time scales and
  `tpt-planetary-astro` time helpers and frame transforms.
- [Arrays](arrays.md): Zarr v2 and v3 reading and writing, codecs, lazy
  reductions, and NetCDF-3 with CF time decoding.
- [Command line](cli.md): the `tptplan` subcommands and exit codes.
- [Status and limitations](status-and-limitations.md): what is implemented,
  partial, not validated, or not started.

## Crates

| Crate | Purpose |
|-------|---------|
| `tpt-planetary-core` | `no_std` constants, Julian dates, time scales, spherical math |
| `tpt-planetary-array` | N-D arrays, Zarr v2/v3, NetCDF-3, codecs, lazy expressions |
| `tpt-planetary-astro` | Time helpers, frames, ephemeris (SPK) reader, WCS |
| `tpt-planetary-cli` | The `tptplan` binary |
| `bindings/tpt-planetary-py` | Python bindings: builds and passes its pytest suite locally; no wheels published |

The Python bindings are built separately. This guide describes the Rust API;
the Python package is covered only in the status page.

## Examples

The Rust examples in these pages were compiled and run against the crates in
this workspace. They are not part of the test suite, except where a page says
the code is a doctest or is copied from `tpt-planetary-cli/tests/dataflow.rs`.
Output lines are copied from those runs.
