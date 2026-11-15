# Getting started

## Toolchain and MSRV

- The toolchain is pinned in `rust-toolchain.toml`: channel `1.95.0`, with the
  `rustfmt` and `clippy` components. `rustup` picks it up automatically inside
  the repository.
- The minimum supported Rust version (MSRV) is **1.95**. It is declared as
  `rust-version` in the workspace `Cargo.toml`, and CI builds the workspace on
  1.95.0 in a separate job.
- The edition is 2024.
- `tpt-simd` needs Rust 1.95, so it is not a dependency. Revisiting the MSRV
  when it is added is recorded in `docs/dependencies.md`.

## Build

From the repository root:

```sh
cargo build --workspace
```

The `no_std` build of the core crate, which CI checks against the
`thumbv7em-none-eabihf` target when that target is installed:

```sh
cargo build -p tpt-planetary-core --no-default-features
```

`bindings/tpt-planetary-py` is a workspace member, and its work is in
progress. If it does not build in your checkout, build the four Rust crates
by name, for example `cargo build -p tpt-planetary-cli`.

The command-line tool is built as the `tptplan` binary:

```sh
cargo build -p tpt-planetary-cli
./target/debug/tptplan --help
```

## Test

```sh
cargo test --workspace
```

This runs unit tests, the crate doctests, the CLI tests in
`tpt-planetary-cli/tests/cli.rs`, and the end-to-end data-flow test:

```sh
cargo test -p tpt-planetary-cli --test dataflow
```

The data-flow test writes a small synthetic climate-like Zarr dataset to a
temporary directory, runs a lazy box mean with cos(lat) weights, converts a
timestamp to TDB, and checks the results against analytic values. Its
tolerances are described in the file header.

## Checks that CI runs

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --workspace --no-deps
```

CI also runs `cargo deny check` (licence, ban and source policy in
`deny.toml`) and a coverage job. Both are configured in
`.github/workflows/ci.yml`.

## Licence

The crates are dual-licensed under MIT OR Apache-2.0. See `LICENSE-MIT` and
`LICENSE-APACHE` in the repository root.

## Next

- [Time and frames](time-and-frames.md)
- [Arrays](arrays.md)
- [Command line](cli.md)
