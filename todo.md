# tpt-planetary TODO

Dual-licensed MIT OR Apache-2.0 · © 2026 TPT Solutions · Pure Rust, zero FFI · permissive dependencies only (no Apache-2.0-ONLY crates)

Legend: `- [ ]` open · `- [x]` done · **M** = milestone

---

## Phase 0: Project Setup & Compliance (Week 0-2)

### Repository & workspace
- [ ] Initialise git repo with default branch `master` (`git init -b master`), `.gitignore`, `.gitattributes`
- [ ] Use `master` as the branch name in CI triggers, branch protection and release workflows
- [ ] Create Cargo workspace root `Cargo.toml` (resolver 2, shared `[workspace.package]`, `[workspace.dependencies]`)
- [ ] Scaffold crates: `tpt-planetary-core`, `tpt-planetary-array`, `tpt-planetary-astro`, `tpt-planetary-cli`, `bindings/tpt-planetary-py`
- [ ] Decide and pin MSRV; add `rust-toolchain.toml`
- [ ] Add `rustfmt.toml` and clippy config (`-D warnings`)
- [ ] Set `license = "MIT OR Apache-2.0"` and `authors`/`repository` in every crate manifest
- [ ] Confirm `tpt-math` and `tpt-simd` versions on crates.io; pin in workspace deps
- [ ] Verify `tpt-math` / `tpt-simd` license and `no_std` support (needed by core)

### Licensing
- [ ] Add `LICENSE-MIT` (© 2026 TPT Solutions)
- [ ] Add `LICENSE-APACHE` (Apache-2.0 text)
- [ ] Add license section to README ("MIT OR Apache-2.0" + contribution clause)
- [ ] Write `NOTICE`/third-party notices process (generate with cargo-about or similar)

### Dependency policy (cargo-deny)
- [ ] Create `deny.toml`; fix the spec's config so it is valid for the current cargo-deny schema
- [ ] Ensure Apache-2.0 is only accepted via dual-licensed crates (`MIT OR Apache-2.0`); add clarifications/exceptions to reject Apache-2.0-ONLY crates
- [ ] Allow list: MIT, BSD-2-Clause, BSD-3-Clause, Zlib, Unicode-3.0, ISC, (MIT OR Apache-2.0)
- [ ] Ban list: GPL-2.0/3.0, AGPL-3.0, LGPL-2.1/3.0, MPL-2.0
- [ ] Ban FFI crates: `hdf5-sys`, `netcdf-sys`, `erfa-sys`, `cffi`, `zstd-sys`, `libz-sys`, `bindgen`-based wrappers
- [ ] Fix spec's `[[bans.deny]]` TOML (one `name` per entry)
- [ ] Add `cargo deny check` (licenses, bans, advisories, sources) to CI
- [ ] Audit every candidate dependency for license + FFI before adding (record in `docs/dependencies.md`)
- [ ] Mark approved pure-Rust codec crates (e.g. `ruzstd`, `miniz_oxide`, `lz4_flex`) and confirm licenses
- [ ] Confirm `object_store` (or alternative) license; otherwise plan thin custom S3/GCS/Azure client

### CI / tooling
- [ ] GitHub Actions (or chosen CI): build, test, clippy, fmt on Linux/macOS/Windows
- [ ] CI job: `no_std` build of `tpt-planetary-core` (e.g. `thumbv7em-none-eabihf`)
- [ ] CI job: `cargo deny check`
- [ ] CI job: `cargo doc` with warnings denied
- [ ] CI job: MSRV check
- [ ] Set up code coverage (cargo-llvm-cov) and fuzzing scaffold (cargo-fuzz)
- [ ] Set up benchmark harness (criterion or divan; check licenses)
- [ ] Add `cargo-semver-checks`/release tooling plan

### Docs & community
- [ ] README with goals, architecture diagram, status
- [ ] `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`
- [ ] Move ADR-001/002/003 into `docs/adr/`
- [ ] Add `CHANGELOG.md`
- [ ] Reserve crate names on crates.io and `tpt-planetary` on PyPI

---

## Phase 1: Core & Array Engine (Months 1-3)

### tpt-planetary-core (no_std + alloc)
- [ ] `#![no_std]` + `alloc` feature layout; `std` feature flag
- [ ] Physical and astronomical constants (IAU/CODATA, with sources cited)
- [ ] Julian Date / Modified JD two-part (high-precision) representation
- [ ] Calendar <-> JD conversion (Gregorian/Julian, proleptic rules)
- [ ] Leap-second table (embedded, versioned, with expiry handling)
- [ ] Time scales: UTC, TAI, TT
- [ ] Time scale: TDB (approximation + full series to be validated in Phase 2)
- [ ] Time scale conversion API with typed time-scale markers
- [ ] Spherical trigonometry: angular separation, position angle, great-circle ops
- [ ] Spherical <-> Cartesian conversions; angle newtypes (rad/deg/hour-angle), wrapping/normalisation
- [ ] Quaternions (via `glam`/`tpt-math`) for rotations; SLERP; Euler/axis-angle conversion
- [ ] Geodesic/ellipsoid helpers using `tpt-math` (Vincenty/Karney, WGS84)
- [ ] Error types usable in `no_std`
- [ ] Unit tests, property tests, and golden-value tests vs known reference values (SOFA/ERFA published test vectors, as data only)
- [ ] Verify `cargo build --no-default-features` on an embedded target
- [ ] API docs with examples

### tpt-planetary-array: data model
- [ ] N-D shape, strides, dtype system (int/uint/float 8-64, bool, complex, string handling)
- [ ] Endianness and fill-value handling
- [ ] Chunk grid model and chunk-key encoding (default and `/`-separated)
- [ ] Slicing/selection model (ranges, steps, integer index, fancy index later)
- [ ] Chunk-aligned region planning (which chunks does a selection touch)
- [ ] Dimension names, coordinates and attributes (metadata)

### tpt-planetary-array: Zarr
- [ ] Zarr v2 metadata: `.zarray`, `.zgroup`, `.zattrs`, consolidated `.zmetadata`
- [ ] Zarr v3 metadata: `zarr.json` for arrays and groups
- [ ] Zarr v3 codec pipeline (array->array, array->bytes, bytes->bytes)
- [ ] Zarr v3 sharding codec
- [ ] Zarr conformance test suite: read real v2 and v3 datasets (generated by zarr-python) as fixtures
- [ ] Write support: create arrays, write chunks, partial-chunk updates
- [ ] Hierarchy (groups) traversal

### tpt-planetary-array: compression (pure Rust)
- [ ] Codec trait + registry
- [ ] zstd decode/encode via pure-Rust crate (decide on `ruzstd` and its encoder limits)
- [ ] gzip/zlib via `miniz_oxide` / `flate2` (pure-Rust backend only)
- [ ] LZ4 via `lz4_flex`
- [ ] Blosc-compatible decoding (pure Rust; evaluate need)
- [ ] Shuffle / bitshuffle / delta filters
- [ ] Codec benchmarks and correctness vs reference files

### tpt-planetary-array: storage & async I/O
- [ ] Async `Store` trait (`async-trait` or native async-fn-in-trait; confirm licensing)
- [ ] Local filesystem store
- [ ] In-memory store (tests)
- [ ] HTTP(S) read-only store with range requests
- [ ] S3 store (SigV4 signing, credential chain, regions, anonymous access)
- [ ] Retry/backoff, concurrency limits, request coalescing of adjacent chunk ranges
- [ ] Chunk cache (LRU) with memory budget
- [ ] Async runtime choice and feature gating (e.g. tokio, MIT)
- [ ] Mock S3 (local) integration tests

### tpt-planetary-array: lazy engine
- [ ] Lazy array handle (metadata only, no data load)
- [ ] Lazy execution graph (nodes: load, slice, elementwise, reduce)
- [ ] Chunk-wise streaming executor (bounded memory)
- [ ] Reductions: sum, mean, min, max, count, std (numerically stable, e.g. Welford) across chunks
- [ ] Masked / NaN-aware (fill value) reductions
- [ ] Parallel chunk execution
- [ ] Memory-budget tests (prove no OOM on large synthetic data)

### Phase 1 wrap-up
- [ ] Minimal `tptplan array inspect` prototype (for milestone)
- [ ] **M1: `tptplan array inspect` reads a 100 GB Zarr dataset from S3 and computes a global mean in seconds without OOM**
- [ ] Phase 1 docs, examples, and release-candidate review

---

## Phase 2: Astronomy & Ephemeris (Months 4-6)

### Time (tpt-planetary-astro)
- [ ] UTC <-> TAI <-> TT with leap seconds, handling the UTC day-length discontinuity
- [ ] UT1 and Earth Orientation Parameters (IERS Bulletin A/B/C04 parsing; bundled snapshot + user-supplied data)
- [ ] TDB <-> TT (Fairhead & Bretagnon series, high precision)
- [ ] Earth Rotation Angle, GMST, GAST
- [ ] Validate against SOFA/ERFA published vectors and Astropy outputs (as test data only)

### Coordinate frames
- [ ] Frame trait / transformation graph (typed frames, attributes such as obstime/location)
- [ ] ICRS <-> Galactic
- [ ] ICRS <-> Ecliptic (mean and true)
- [ ] IAU 2006 precession and IAU 2000A/B nutation
- [ ] Aberration, light deflection (optional), proper motion & parallax propagation
- [ ] Observer location (geodetic/ITRS), polar motion
- [ ] ICRS <-> CIRS <-> TIRS/ITRS
- [ ] Local Horizontal (Alt/Az) with optional atmospheric refraction
- [ ] FK5 / FK4 legacy frames (decide scope)
- [ ] Accuracy tests (target: sub-milliarcsecond vs SOFA vectors)

### Ephemeris (JPL DE440)
- [ ] DE440/DE440s binary (SPK/DAF) file format parser
- [ ] Segment/record lookup by epoch
- [ ] Chebyshev polynomial evaluation (position and velocity)
- [ ] Body/barycenter hierarchy (Sun, planets, Moon, EMB, Earth-Moon offset)
- [ ] Memory-mapped and in-memory loading; `no_std` slice-based API
- [ ] Decide how data files are obtained (user-supplied path / download helper); document licensing of JPL data
- [ ] Light-time correction and apparent positions
- [ ] Validate against JPL Horizons reference values
- [ ] **M2: `tptplan astro ephemeris` computes Jupiter's position to sub-arcsecond accuracy for any timestamp**

### Phase 2 wrap-up
- [ ] Earth-Sun distance helper (used by the climate/irradiance data-flow example)
- [ ] Docs, examples, and benchmarks vs Astropy

---

## Phase 3: Advanced Compute & WCS (Months 7-9)

### Map algebra (tpt-simd)
- [ ] Integrate `tpt-simd` into `tpt-planetary-array`
- [ ] Vectorised elementwise ops (+, -, *, /, pow, abs, sqrt, exp, log, trig)
- [ ] Vectorised comparisons, masks, `where`/select
- [ ] SIMD reductions (sum, mean, min, max, std) with scalar fallbacks
- [ ] Broadcasting rules (N-D)
- [ ] Axis-wise reductions and group-by style aggregations (e.g. monthly climatology)
- [ ] Rolling/window ops and resampling along time
- [ ] Operation fusion in lazy graph (e.g. `array * 2 + 1` in one pass)
- [ ] Spherical-aware weighting (cos(lat) area weights) for global means
- [ ] Correctness tests vs scalar reference; benchmark vs NumPy/xarray

### NetCDF compatibility (pure Rust)
- [ ] NetCDF3 Classic / 64-bit offset reader
- [ ] NetCDF3 writer
- [ ] CF-conventions metadata handling (time units, calendars, coordinates)
- [ ] NetCDF-style API surface (dimensions, variables, attributes)
- [ ] Interop tests using files produced by netCDF4/xarray

### WCS (tpt-planetary-astro)
- [ ] FITS header WCS keyword parser (CRPIX, CRVAL, CDELT, CD/PC matrix, CTYPE)
- [ ] Linear pixel <-> intermediate world transform
- [ ] Projections: TAN (gnomonic)
- [ ] Projections: SIN (orthographic/slant)
- [ ] Projections: AIT (Hammer-Aitoff)
- [ ] Native <-> celestial spherical rotation (LONPOLE/LATPOLE)
- [ ] Round-trip pixel->sky->pixel accuracy tests vs wcslib/astropy reference values
- [ ] SIP distortion (decide scope; may defer to Phase 5)

### Climate indices
- [ ] Anomaly and climatology computation (lazy)
- [ ] El Nino indices (e.g. Nino3.4 SST anomaly) as a worked, lazy pipeline
- [ ] **M3: `tptplan array math` performs a complex multi-variable climate index calculation (e.g. El Nino index) lazily**

---

## Phase 4: CLI, Python Bindings & Release (Months 10-12)

### tpt-planetary-cli (`tptplan`)
- [ ] `clap`-based CLI skeleton, global flags (`--verbose`, `--format json|table`), shell completions
- [ ] `tptplan array inspect <uri>` (shape, dtype, chunks, attrs; local/S3/HTTP)
- [ ] `tptplan array math` (expression evaluation, lazy, output to Zarr/NetCDF3)
- [ ] `tptplan array convert` (Zarr <-> NetCDF3)
- [ ] `tptplan astro convert` (time scales and coordinate frames)
- [ ] `tptplan ephemeris query` (body, epoch, observer, output frame)
- [ ] Progress reporting, error messages, exit codes
- [ ] CLI integration tests; man pages / help text
- [ ] Prebuilt binaries for Linux/macOS/Windows (release workflow)

### tpt-planetary-py (PyO3, MIT)
- [ ] Confirm `pyo3` / `maturin` license and abi3 policy
- [ ] PyO3 module scaffold, `maturin` build config
- [ ] Bind core: time scales, angles, spherical math
- [ ] Bind astro: frames, ephemeris, WCS
- [ ] Bind array: open Zarr/NetCDF3, slice, lazy ops, compute
- [ ] NumPy interop (zero-copy where possible) via `numpy` crate
- [ ] xarray / Dask integration (backend entrypoint or adapter) and SunPy/Astropy interop notes
- [ ] Async/GIL handling for S3 streaming
- [ ] Type stubs (`.pyi`) and Python tests (pytest)
- [ ] Wheels for manylinux/musllinux, macOS (x86_64, arm64), Windows via CI
- [ ] Jupyter example notebook (the climate data-flow example with Matplotlib)

### Benchmarks & validation
- [ ] Benchmarks vs xarray/netCDF4 (open, slice, mean, S3 streaming)
- [ ] Benchmarks vs Astropy (time conversion, frame transforms, ephemeris)
- [ ] Publish reproducible benchmark methodology and results
- [ ] End-to-end test of spec section 7 data-flow example
- [ ] Final licence/dependency audit (`cargo deny`, SBOM, third-party notices)
- [ ] Final `no_std` verification of `tpt-planetary-core`

### Docs & release
- [ ] User guide, API docs (docs.rs), tutorials, migration guides (Astropy / NetCDF4 / xarray -> tpt-planetary)
- [ ] Complete `CHANGELOG.md`, version bump to 0.1.0
- [ ] Publish crates in dependency order: core -> array -> astro -> cli
- [ ] Publish `tpt-planetary` wheels to PyPI
- [ ] Tag release and publish GitHub release notes
- [ ] **M4: v0.1.0 released on crates.io and PyPI with published benchmarks**

---

## Phase 5: Post-Release (v0.2+)

- [ ] Read-only HDF5 / NetCDF4 subset reader (superblock, B-trees, contiguous/chunked, deflate), pure Rust
- [ ] Write HDF5 (evaluate demand)
- [ ] GCS and Azure Blob stores
- [ ] Zarr v3 extensions (more codecs, data types) and Zarr spec updates
- [ ] Parquet/Arrow-concept interop via permissively-licensed pure-Rust implementation (or custom)
- [ ] More WCS projections (ARC, ZEA, STG, CAR, MOL, HPX, etc.) and SIP/TPV distortion
- [ ] DE441 and other ephemerides; SPICE kernel (SPK/CK/PCK) support
- [ ] Orbital mechanics: SGP4/TLE propagation, Keplerian elements, orbit determination helpers
- [ ] FITS image I/O (pure Rust)
- [ ] GPU-accelerated compute (if/when tpt-simd supports)
- [ ] Distributed execution / Dask-like scheduler
- [ ] Ongoing: dependency license re-audits, MSRV policy, security advisories, community issue triage

---

## Open questions / risks to track
- [ ] Confirm `object_store` license and transitive deps; fall back to custom client if any are Apache-2.0-ONLY
- [ ] Confirm pure-Rust zstd *encoder* quality (ruzstd) is acceptable; otherwise decide write-side policy
- [ ] Confirm `tpt-math`/`tpt-simd` are `no_std`-compatible
- [ ] Decide handling and redistribution of IERS EOP data and JPL DE440 files (size, licence/terms)
- [ ] 12-month solo timeline is aggressive; re-plan at end of each phase
