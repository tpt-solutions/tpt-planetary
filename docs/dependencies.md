# Dependency audit log

Every dependency is recorded here before it is added to a crate manifest.
`cargo deny check` enforces the licence and ban rules in `deny.toml`. This file
records the reasoning behind each choice, which `deny.toml` cannot capture.

## Policy

- Allowed licences: MIT, BSD-2-Clause, BSD-3-Clause, Zlib, Unicode-3.0, ISC, or
  dual `MIT OR Apache-2.0`. Apache-2.0-only crates are rejected.
- Banned: GPL, LGPL, AGPL, MPL, and any crate that is a C FFI wrapper.
- `tpt-planetary-core` must build for `no_std` (checked with
  `thumbv7em-none-eabihf`). Every dependency it uses must pass that build.
- MSRV is 1.85. A dependency whose `rust-version` is higher is not adopted
  unless the MSRV is raised as a recorded decision.

## Core dependencies

| Crate | Version | Licence | MSRV | `no_std` | Used by | Status |
|---|---|---|---|---|---|---|
| `tpt-math` | 0.1.0 | MIT OR Apache-2.0 | 1.85 | **No.** Fails to build for `thumbv7em-none-eabihf`: it does not declare `#![no_std]` and requires `std`. | — (not yet used) | **Not adopted for `core`.** Use only from `std` crates until upstream adds `no_std`, or replace it in core. |
| `glam` | 0.34 | MIT OR Apache-2.0 | 1.85 | Yes, with `default-features = false, features = ["libm"]` | core (planned) | Approved. Verified: builds for `thumbv7em-none-eabihf`. |
| `tpt-simd` | 0.1.1 | MIT OR Apache-2.0 | **1.95** | Not checked (cannot build on MSRV 1.85) | array (Phase 3) | Licence OK. **Needs Rust 1.95**, above our MSRV. Adopting it means raising the MSRV, a decision for Phase 3. |

## Phase 1 candidates (licence and MSRV checked; not yet added)

| Crate | Version | Licence | MSRV | Notes |
|---|---|---|---|---|
| `object_store` | 0.14.2 | MIT/Apache-2.0 | 1.85 | Licence OK. Transitive dependencies still need a licence sweep before adoption. |
| `ruzstd` | 0.9.1 | MIT | **1.87** | Licence OK, but its MSRV is above 1.85. Decide whether to raise the MSRV or pin an older release. |
| `miniz_oxide` | 0.9.1 | MIT OR Zlib OR Apache-2.0 | not declared | Pure Rust deflate. Licence OK. |
| `flate2` | 1.1.11 | MIT OR Apache-2.0 | 1.67 | Use only with the pure-Rust `miniz_oxide` backend. |
| `lz4_flex` | 0.14.2 | MIT | 1.81 | Pure Rust LZ4. Licence OK. |
| `tokio` | 1.54.1 | MIT | 1.85 | Async runtime for the S3/HTTP stores. Feature-gate it. |
| `async-trait` | 0.1.92 | MIT OR Apache-2.0 | 1.71 | Licence OK. |
| `criterion` | 0.7.0 (0.8.2 latest) | Apache-2.0 OR MIT | 1.80 | Benchmark harness candidate. |
| `divan` | 0.1.21 | MIT OR Apache-2.0 | 1.80 | Benchmark harness candidate. |

## Adopted in `tpt-planetary-array` (Phase 1)

| Crate | Version | Licence | MSRV | Notes |
|---|---|---|---|---|
| `miniz_oxide` | 0.9.1 | MIT OR Zlib OR Apache-2.0 | none declared | Pure Rust DEFLATE/zlib. Builds on 1.85. Default features off, `with-alloc` on. |
| `lz4_flex` | 0.14.2 | MIT | 1.81 | Pure Rust LZ4. `alloc`, `safe-encode`, `safe-decode`. |
| `serde_json` | 1.0.154 | MIT OR Apache-2.0 | 1.71 | Zarr metadata. `alloc` only. Transitive: `serde_core`, `itoa`, `zmij` (MIT), `memchr` (Unlicense OR MIT), `adler2` (0BSD OR MIT OR Apache-2.0). |

Not adopted: `ruzstd` (needs Rust 1.87, above MSRV).

## Adopted in `tpt-planetary-py` (Phase 4)

| Crate | Version | Licence | MSRV | Notes |
|---|---|---|---|---|
| `pyo3` | 0.30.1 | MIT OR Apache-2.0 | 1.85 | `abi3-py39`, one wheel per platform. |
| `numpy` (rust-numpy) | 0.30.0 | BSD-2-Clause | 1.85 | NumPy interop. |
| `target-lexicon` | 0.13.6 | Apache-2.0 WITH LLVM-exception | (in pyo3-build-config) | Build-time only, pulled in by `pyo3-build-config`. **Not on the `deny.toml` allow list, and `cargo deny check` currently fails on it.** The spec bans Apache-2.0-only licences, so this needs a decision (see `todo.md`). |

## Adopted in benchmarks and fuzzing (dev-only)

| Crate | Version | Licence | MSRV | Notes |
|---|---|---|---|---|
| `divan` | 0.1.21 | MIT OR Apache-2.0 | 1.80 | Dev-dependency of core, array and astro. |
| `libfuzzer-sys` | 0.4.13 | (MIT OR Apache-2.0) AND NCSA | not declared | `fuzz/` crate only, excluded from the workspace. Does not run on Windows MSVC. |

## Phase 4 candidates

| Crate | Version | Licence | MSRV | Notes |
|---|---|---|---|---|
| `clap` | 4.6.7 | MIT OR Apache-2.0 | 1.85 | CLI parsing. |
| `pyo3` | 0.30.1 | MIT OR Apache-2.0 | 1.85 | Python bindings. |
| `numpy` (rust-numpy) | 0.30.0 | BSD-2-Clause | 1.85 | NumPy interop. |

## Developer tools (not linked into any crate)

| Tool | Licence | Notes |
|---|---|---|
| `cargo-deny` 0.20.2 | MIT OR Apache-2.0 | Installed. Used in CI. |
| `cargo-about` 0.8.4 | MIT OR Apache-2.0 | Candidate for generating the third-party NOTICE. |
| `cargo-llvm-cov` 0.6.21 | Apache-2.0 OR MIT | Candidate for coverage. |
| `cargo-fuzz` 0.13.2 | MIT OR Apache-2.0 | Candidate for the fuzzing scaffold. |
| `cargo-semver-checks` 0.42.0 | Apache-2.0 OR MIT | Candidate for release tooling. |

## Open decisions

- **MSRV.** The workspace pins 1.85. Two candidates need more: `tpt-simd`
  needs 1.95 and `ruzstd` 0.9.1 needs 1.87. Either raise the MSRV in a dated
  decision, or choose older versions.
- **`tpt-math` in `core`.** Not usable on `no_std` today. Either keep it out of
  `core`, or ask upstream for `#![no_std]`.
