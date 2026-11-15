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
- MSRV is 1.95 (raised from 1.85 by decision on 2026-11-16, so that `tpt-simd` and
  `ruzstd` can be adopted). A dependency whose `rust-version` is higher is not
  adopted unless the MSRV is raised as a recorded decision.

## Core dependencies

| Crate | Version | Licence | MSRV | `no_std` | Used by | Status |
|---|---|---|---|---|---|---|
| `tpt-math` | 0.1.0 | MIT OR Apache-2.0 | 1.85 | **No.** Fails to build for `thumbv7em-none-eabihf`: it does not declare `#![no_std]` and requires `std`. | — (not yet used) | **Not adopted for `core`.** Use only from `std` crates until upstream adds `no_std`, or replace it in core. |
| `glam` | 0.34 | MIT OR Apache-2.0 | 1.85 | Yes, with `default-features = false, features = ["libm"]` | core (planned) | Approved. Verified: builds for `thumbv7em-none-eabihf`. |
| `tpt-simd-vector` | 0.1.1 | MIT OR Apache-2.0 | **1.95** | Yes (`#![no_std]`, `#![forbid(unsafe_code)]`). Built for `thumbv7em-none-eabihf` with `simd` on and off. | array, feature `simd` (Phase 3) | Adopted (see below). The umbrella `tpt-simd` is not used. |

## Phase 1 candidates (licence and MSRV checked; not yet added)

| Crate | Version | Licence | MSRV | Notes |
|---|---|---|---|---|
| `object_store` | 0.14.2 | MIT/Apache-2.0 | 1.85 | Licence OK. Transitive dependencies still need a licence sweep before adoption. |
| `ruzstd` | 0.9.1 | MIT | **1.87** | Adopted (see below). |
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
| `ruzstd` | 0.9.1 | MIT | 1.87 | Pure-Rust Zstandard. `default-features = false` with the `hash` feature only, which adds `twox-hash` 2.1.5 (MIT, MSRV 1.81); checksums are compared in our decode path because ruzstd does not compare them itself. `#![no_std]` with `alloc`; built for `thumbv7em-none-eabihf`. **Decoder:** RFC 8878 frames, multiple concatenated frames; tested against zstd CLI v1.5.7 output (levels 1, 3 and 19; checksummed and unchecksummed). **Content checksums are parsed but not verified** (the `hash` feature would add `twox-hash`, not adopted). **Encoder:** only `CompressionLevel::Fastest` ("roughly level 1") and `Uncompressed` work. `Default`, `Better` and `Best` call `unimplemented!()` and panic, so the codec rejects them before calling the encoder. Output of the encoder was checked by decoding it with zstd CLI v1.5.7 (4 KB and 300 KB inputs). Used by `codec::compress::ZstdCodec`, registry id `zstd`; no feature gate. |
| `tpt-simd-vector` | 0.1.1 | MIT OR Apache-2.0 | 1.95 | Fixed-width array-backed `Simd<f64, 4>`. Its only dependency is `libm` 0.2.16 (MIT, MSRV 1.63), which is already in `Cargo.lock` through `glam`. `no_std`, `forbid(unsafe_code)`. Used behind feature `simd` (default **off**; the scalar path is the default and the reference). The lane operations are safe loops that LLVM may auto-vectorise; there are no intrinsics, so any speed-up depends on the compiler. Not adopted from the same family: `tpt-simd` (umbrella), `tpt-simd-reduce` (not needed: the kernels are written against `Simd` directly), `tpt-simd-math` (f32 only). |

## Adopted in `tpt-planetary-py` (Phase 4)

| Crate | Version | Licence | MSRV | Notes |
|---|---|---|---|---|
| `pyo3` | 0.30.1 | MIT OR Apache-2.0 | 1.85 | `abi3-py39`, one wheel per platform. |
| `numpy` (rust-numpy) | 0.30.0 | BSD-2-Clause | 1.85 | NumPy interop. |
| `target-lexicon` | 0.13.6 (substitute) | MIT | 1.95 | **Replaced.** The upstream crate is Apache-2.0 WITH LLVM-exception, which the policy bans. The workspace patches it with the MIT-licensed, API-compatible crate at `../target-lexicon-mit` (`[patch.crates-io]` in the root `Cargo.toml`). Build-time only, used by `pyo3-build-config`. The patch uses a path outside this repository, so CI needs that path checked out or the substitute published. |

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

- **MSRV (decided).** Raised to 1.95 on 2026-11-16. This was required by the
  `target-lexicon` substitute (which declares 1.95) and admits `tpt-simd` (1.95)
  and `ruzstd` (1.87). The toolchain pin, CI MSRV job and fuzz crate follow it.
- **`tpt-math` in `core`.** Not usable on `no_std` today. Either keep it out of
  `core`, or ask upstream for `#![no_std]`.
- **ruzstd content checksums.** Verified in `ZstdCodec::decode` (stored versus recomputed XXH64 low 32 bits); covered by a corruption test.
  Turning it on adds `twox-hash` (needs its own licence and MSRV entry). Decide
  before zstd data is treated as integrity-checked.
- **zstd write policy.** The encoder accepts only level 1 (Zarr default). Other
  levels return `Unsupported` on write and still read. ruzstd has no level 3 or
  higher encoder.
- **`simd` default.** The `simd` feature is off by default. Turn it on only after
  the benchmark shows a gain on the target platforms.
