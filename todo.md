# tpt-planetary TODO

Dual-licensed MIT OR Apache-2.0 · © 2026 TPT Solutions · Pure Rust, zero FFI · permissive dependencies only (no Apache-2.0-ONLY crates)

Legend: `- [ ]` open · `- [x]` done · **M** = milestone

---

## Phase 0: Project Setup & Compliance (Week 0-2)

### Repository & workspace
- [x] Initialise git repo with default branch `master` (`git init -b master`), `.gitignore`, `.gitattributes`
- [ ] Use `master` as the branch name in CI triggers, branch protection and release workflows
- [x] Create Cargo workspace root `Cargo.toml` (resolver 2, shared `[workspace.package]`, `[workspace.dependencies]`)
- [x] Scaffold crates: `tpt-planetary-core`, `tpt-planetary-array`, `tpt-planetary-astro`, `tpt-planetary-cli`, `bindings/tpt-planetary-py`
- [x] Decide and pin MSRV; add `rust-toolchain.toml`
- [x] Add `rustfmt.toml` and clippy config (`-D warnings`)
- [x] Set `license = "MIT OR Apache-2.0"` and `authors`/`repository` in every crate manifest
- [ ] Confirm `tpt-math` and `tpt-simd` versions on crates.io; pin in workspace deps (`tpt-math` 0.1.0 pinned; `tpt-simd` 0.1.1 deferred to Phase 3 because it needs Rust 1.95)
- [x] Verify `tpt-math` / `tpt-simd` license and `no_std` support (needed by core): `tpt-math` is MIT OR Apache-2.0 but **not** `no_std`; `tpt-simd` not checkable on MSRV 1.85 (see `docs/dependencies.md`)

### Licensing
- [x] Add `LICENSE-MIT` (© 2026 TPT Solutions)
- [x] Add `LICENSE-APACHE` (Apache-2.0 text)
- [x] Add license section to README ("MIT OR Apache-2.0" + contribution clause)
- [ ] Write `NOTICE`/third-party notices process (generate with cargo-about or similar)

### Dependency policy (cargo-deny)
- [x] Create `deny.toml`; fix the spec's config so it is valid for the current cargo-deny schema
- [x] Ensure Apache-2.0 is only accepted via dual-licensed crates (`MIT OR Apache-2.0`); add clarifications/exceptions to reject Apache-2.0-ONLY crates
- [x] Allow list: MIT, BSD-2-Clause, BSD-3-Clause, Zlib, Unicode-3.0, ISC, (MIT OR Apache-2.0)
- [x] Ban list: GPL-2.0/3.0, AGPL-3.0, LGPL-2.1/3.0, MPL-2.0
- [x] Ban FFI crates: `hdf5-sys`, `netcdf-sys`, `erfa-sys`, `cffi`, `zstd-sys`, `libz-sys`, `bindgen`-based wrappers
- [x] Fix spec's `[[bans.deny]]` TOML (one `name` per entry)
- [x] Add `cargo deny check` (licenses, bans, advisories, sources) to CI
- [x] Audit every candidate dependency for license + FFI before adding (record in `docs/dependencies.md`)
- [x] Mark approved pure-Rust codec crates (e.g. `ruzstd`, `miniz_oxide`, `lz4_flex`) and confirm licenses (all permissive; `ruzstd` needs Rust 1.87, see `docs/dependencies.md`)
- [x] Confirm `object_store` (or alternative) license; otherwise plan thin custom S3/GCS/Azure client (`object_store` 0.14.2 is MIT/Apache-2.0; transitive sweep still open below)

### CI / tooling
- [x] GitHub Actions (or chosen CI): build, test, clippy, fmt on Linux/macOS/Windows
- [x] CI job: `no_std` build of `tpt-planetary-core` (e.g. `thumbv7em-none-eabihf`)
- [x] CI job: `cargo deny check`
- [x] CI job: `cargo doc` with warnings denied
- [x] CI job: MSRV check
- [ ] Set up code coverage (cargo-llvm-cov) and fuzzing scaffold (cargo-fuzz) — PARTIAL: coverage job in CI and a fuzz scaffold in `fuzz/`; fuzzing not run (libFuzzer does not support Windows MSVC)
- [x] Set up benchmark harness (criterion or divan; check licenses)
- [x] Add `cargo-semver-checks`/release tooling plan

### Docs & community
- [x] README with goals, architecture diagram, status
- [x] `CONTRIBUTING.md` (issues-only policy; `SECURITY.md` and `CODE_OF_CONDUCT.md` dropped)
- [x] Move ADR-001/002/003 into `docs/adr/`
- [x] Add `CHANGELOG.md`
- [ ] Reserve crate names on crates.io and `tpt-planetary` on PyPI

---

## Phase 1: Core & Array Engine (Months 1-3)

### tpt-planetary-core (no_std + alloc)
- [x] `#![no_std]` + `alloc` feature layout; `std` feature flag
- [x] Physical and astronomical constants (IAU/CODATA, with sources cited)
- [x] Julian Date / Modified JD two-part (high-precision) representation
- [x] Calendar <-> JD conversion (Gregorian/Julian, proleptic rules)
- [x] Leap-second table (embedded, versioned, with expiry handling)
- [x] Time scales: UTC, TAI, TT
- [ ] Time scale: TDB (approximation + full series to be validated in Phase 2) — PARTIAL: not in core; astro has a 3-term approximation
- [x] Time scale conversion API with typed time-scale markers
- [x] Spherical trigonometry: angular separation, position angle, great-circle ops
- [x] Spherical <-> Cartesian conversions; angle newtypes (rad/deg/hour-angle), wrapping/normalisation
- [x] Quaternions (via `glam`/`tpt-math`) for rotations; SLERP; Euler/axis-angle conversion
- [ ] Geodesic/ellipsoid helpers using `tpt-math` (Vincenty/Karney, WGS84) — PARTIAL: geodetic and ECEF conversion with geocentric latitude only; no Vincenty or Karney inverse
- [x] Error types usable in `no_std`
- [ ] Unit tests, property tests, and golden-value tests vs known reference values (SOFA/ERFA published test vectors, as data only) — PARTIAL: table-driven tests only; no SOFA/ERFA vectors
- [x] Verify `cargo build --no-default-features` on an embedded target
- [ ] API docs with examples — PARTIAL: rustdoc and doctests in core; other crates partial

### tpt-planetary-array: data model
- [x] N-D shape, strides, dtype system (int/uint/float 8-64, bool, complex, string handling)
- [x] Endianness and fill-value handling
- [x] Chunk grid model and chunk-key encoding (default and `/`-separated)
- [x] Slicing/selection model (ranges, steps, integer index, fancy index later)
- [x] Chunk-aligned region planning (which chunks does a selection touch)
- [x] Dimension names, coordinates and attributes (metadata)

### tpt-planetary-array: Zarr
- [x] Zarr v2 metadata: `.zarray`, `.zgroup`, `.zattrs`, consolidated `.zmetadata`
- [x] Zarr v3 metadata: `zarr.json` for arrays and groups
- [ ] Zarr v3 codec pipeline (array->array, array->bytes, bytes->bytes) — PARTIAL: codec pipeline without transpose or sharding
- [ ] Zarr v3 sharding codec
- [ ] Zarr conformance test suite: read real v2 and v3 datasets (generated by zarr-python) as fixtures
- [x] Write support: create arrays, write chunks, partial-chunk updates
- [x] Hierarchy (groups) traversal

### tpt-planetary-array: compression (pure Rust)
- [x] Codec trait + registry
- [ ] zstd decode/encode via pure-Rust crate (decide on `ruzstd` and its encoder limits)
- [x] gzip/zlib via `miniz_oxide` / `flate2` (pure-Rust backend only)
- [x] LZ4 via `lz4_flex`
- [ ] Blosc-compatible decoding (pure Rust; evaluate need)
- [ ] Shuffle / bitshuffle / delta filters — PARTIAL: shuffle and delta only; no bitshuffle
- [ ] Codec benchmarks and correctness vs reference files — PARTIAL: gzip and lz4 round trips only; no reference files

### tpt-planetary-array: storage & async I/O
- [ ] Async `Store` trait (`async-trait` or native async-fn-in-trait; confirm licensing)
- [x] Local filesystem store
- [x] In-memory store (tests)
- [ ] HTTP(S) read-only store with range requests
- [ ] S3 store (SigV4 signing, credential chain, regions, anonymous access)
- [ ] Retry/backoff, concurrency limits, request coalescing of adjacent chunk ranges
- [ ] Chunk cache (LRU) with memory budget
- [ ] Async runtime choice and feature gating (e.g. tokio, MIT)
- [ ] Mock S3 (local) integration tests

### tpt-planetary-array: lazy engine
- [x] Lazy array handle (metadata only, no data load)
- [x] Lazy execution graph (nodes: load, slice, elementwise, reduce)
- [x] Chunk-wise streaming executor (bounded memory)
- [x] Reductions: sum, mean, min, max, count, std (numerically stable, e.g. Welford) across chunks
- [x] Masked / NaN-aware (fill value) reductions
- [ ] Parallel chunk execution
- [ ] Memory-budget tests (prove no OOM on large synthetic data) — PARTIAL: bound checked through ExecStats block counts, not allocator measurement

### Phase 1 wrap-up
- [x] Minimal `tptplan array inspect` prototype (for milestone)
- [ ] **M1: `tptplan array inspect` reads a 100 GB Zarr dataset from S3 and computes a global mean in seconds without OOM**
- [ ] Phase 1 docs, examples, and release-candidate review — PARTIAL: user guide written; no release-candidate review

---

## Phase 2: Astronomy & Ephemeris (Months 4-6)

### Time (tpt-planetary-astro)
- [x] UTC <-> TAI <-> TT with leap seconds, handling the UTC day-length discontinuity
- [ ] UT1 and Earth Orientation Parameters (IERS Bulletin A/B/C04 parsing; bundled snapshot + user-supplied data) — PARTIAL: finals2000A parser with a column map from memory; no bundled snapshot; no UT1 scale type
- [ ] TDB <-> TT (Fairhead & Bretagnon series, high precision) — PARTIAL: leading 3 terms in astro; full series open
- [x] Earth Rotation Angle, GMST, GAST
- [ ] Validate against SOFA/ERFA published vectors and Astropy outputs (as test data only) — PARTIAL: checked against pyerfa 2.0.1.5 and astropy 8.0.2 (see docs/validation.md): leap steps, ERA, GMST, obliquity, ICRS-Galactic and WCS pass; TT-TDB differs by up to 14.9 us (3-term series); SPK, nutation and precession not validated

### Coordinate frames
- [x] Frame trait / transformation graph (typed frames, attributes such as obstime/location)
- [x] ICRS <-> Galactic
- [ ] ICRS <-> Ecliptic (mean and true) — PARTIAL: mean ecliptic only; true ecliptic partial
- [ ] IAU 2006 precession and IAU 2000A/B nutation
- [ ] Aberration, light deflection (optional), proper motion & parallax propagation
- [ ] Observer location (geodetic/ITRS), polar motion — PARTIAL: geodetic observer only; no ITRS or polar motion
- [ ] ICRS <-> CIRS <-> TIRS/ITRS
- [x] Local Horizontal (Alt/Az) with optional atmospheric refraction
- [ ] FK5 / FK4 legacy frames (decide scope)
- [ ] Accuracy tests (target: sub-milliarcsecond vs SOFA vectors) — PARTIAL: no SOFA vectors; the ERFA-based checks in docs/validation.md give sub-mas agreement for ERA, GMST and obliquity only

### Ephemeris (JPL DE440)
- [x] DE440/DE440s binary (SPK/DAF) file format parser
- [x] Segment/record lookup by epoch
- [x] Chebyshev polynomial evaluation (position and velocity)
- [x] Body/barycenter hierarchy (Sun, planets, Moon, EMB, Earth-Moon offset)
- [ ] Memory-mapped and in-memory loading; `no_std` slice-based API — PARTIAL: in-memory and byte-slice APIs; no memory mapping
- [ ] Decide how data files are obtained (user-supplied path / download helper); document licensing of JPL data
- [ ] Light-time correction and apparent positions — PARTIAL: one-iteration light time only; no apparent positions
- [ ] Validate against JPL Horizons reference values
- [ ] **M2: `tptplan astro ephemeris` computes Jupiter's position to sub-arcsecond accuracy for any timestamp**

### Phase 2 wrap-up
- [x] Earth-Sun distance helper (used by the climate/irradiance data-flow example)
- [ ] Docs, examples, and benchmarks vs Astropy — PARTIAL: docs and divan benches; no Astropy comparison

---

## Phase 3: Advanced Compute & WCS (Months 7-9)

### Map algebra (tpt-simd)
- [ ] Integrate `tpt-simd` into `tpt-planetary-array`
- [ ] Vectorised elementwise ops (+, -, *, /, pow, abs, sqrt, exp, log, trig) — PARTIAL: scalar code only; no SIMD, tpt-simd is blocked by its Rust 1.95 requirement
- [x] Vectorised comparisons, masks, `where`/select
- [ ] SIMD reductions (sum, mean, min, max, std) with scalar fallbacks — PARTIAL: scalar reductions only
- [x] Broadcasting rules (N-D)
- [ ] Axis-wise reductions and group-by style aggregations (e.g. monthly climatology) — PARTIAL: axis reductions done; monthly climatology group-by not done
- [x] Rolling/window ops and resampling along time
- [x] Operation fusion in lazy graph (e.g. `array * 2 + 1` in one pass)
- [x] Spherical-aware weighting (cos(lat) area weights) for global means
- [ ] Correctness tests vs scalar reference; benchmark vs NumPy/xarray — PARTIAL: scalar reference tests done; no benchmarks

### NetCDF compatibility (pure Rust)
- [x] NetCDF3 Classic / 64-bit offset reader
- [x] NetCDF3 writer
- [ ] CF-conventions metadata handling (time units, calendars, coordinates) — PARTIAL: time units and standard, proleptic_gregorian and noleap calendars only
- [x] NetCDF-style API surface (dimensions, variables, attributes)
- [ ] Interop tests using files produced by netCDF4/xarray — PARTIAL: scipy-generated fixtures only; no netCDF4 or xarray files

### WCS (tpt-planetary-astro)
- [x] FITS header WCS keyword parser (CRPIX, CRVAL, CDELT, CD/PC matrix, CTYPE)
- [x] Linear pixel <-> intermediate world transform
- [x] Projections: TAN (gnomonic)
- [x] Projections: SIN (orthographic/slant)
- [x] Projections: AIT (Hammer-Aitoff)
- [x] Native <-> celestial spherical rotation (LONPOLE/LATPOLE)
- [ ] Round-trip pixel->sky->pixel accuracy tests vs wcslib/astropy reference values — PARTIAL: synthetic round trips only; no wcslib comparison
- [ ] SIP distortion (decide scope; may defer to Phase 5)

### Climate indices
- [ ] Anomaly and climatology computation (lazy)
- [ ] El Nino indices (e.g. Nino3.4 SST anomaly) as a worked, lazy pipeline
- [ ] **M3: `tptplan array math` performs a complex multi-variable climate index calculation (e.g. El Nino index) lazily**

---

## Phase 4: CLI, Python Bindings & Release (Months 10-12)

### tpt-planetary-cli (`tptplan`)
- [x] `clap`-based CLI skeleton, global flags (`--verbose`, `--format json|table`), shell completions
- [ ] `tptplan array inspect <uri>` (shape, dtype, chunks, attrs; local/S3/HTTP) — PARTIAL: local Zarr only; s3 and https return exit 3
- [ ] `tptplan array math` (expression evaluation, lazy, output to Zarr/NetCDF3)
- [ ] `tptplan array convert` (Zarr <-> NetCDF3)
- [ ] `tptplan astro convert` (time scales and coordinate frames) — PARTIAL: utc, tai and tt only; no frames
- [ ] `tptplan ephemeris query` (body, epoch, observer, output frame) — PARTIAL: needs an SPK kernel; the successful query path is untested
- [x] Progress reporting, error messages, exit codes
- [x] CLI integration tests; man pages / help text
- [ ] Prebuilt binaries for Linux/macOS/Windows (release workflow)

### tpt-planetary-py (PyO3, MIT)
- [x] Confirm `pyo3` / `maturin` license and abi3 policy
- [x] PyO3 module scaffold, `maturin` build config
- [x] Bind core: time scales, angles, spherical math
- [ ] Bind astro: frames, ephemeris, WCS — PARTIAL: ICRS and Galactic frames and WCS only; no ephemeris
- [ ] Bind array: open Zarr/NetCDF3, slice, lazy ops, compute — PARTIAL: local Zarr only; float64 reads and reductions; no NetCDF3 and no remote stores
- [ ] NumPy interop (zero-copy where possible) via `numpy` crate — PARTIAL: copies on read, no zero-copy path
- [ ] xarray / Dask integration (backend entrypoint or adapter) and SunPy/Astropy interop notes
- [ ] Async/GIL handling for S3 streaming — PARTIAL: GIL released for reads and reductions; no S3 streaming
- [x] Type stubs (`.pyi`) and Python tests (pytest)
- [ ] Wheels for manylinux/musllinux, macOS (x86_64, arm64), Windows via CI — PARTIAL: wheel and sdist workflow written in .github/workflows/wheels.yml, not yet run on a CI runner; the sdist may not package the sibling crates
- [ ] Jupyter example notebook (the climate data-flow example with Matplotlib)

### Benchmarks & validation
- [ ] Benchmarks vs xarray/netCDF4 (open, slice, mean, S3 streaming)
- [ ] Benchmarks vs Astropy (time conversion, frame transforms, ephemeris) — PARTIAL: time conversion and frame transform benchmarks done against astropy (docs/benchmarks.md); no ephemeris comparison
- [ ] Publish reproducible benchmark methodology and results
- [ ] End-to-end test of spec section 7 data-flow example — PARTIAL: scaled-down test on synthetic data; no SPK kernel, so no kernel-based ephemeris check
- [ ] Final licence/dependency audit (`cargo deny`, SBOM, third-party notices) — PARTIAL: cargo deny fails on target-lexicon (licence decision pending)
- [ ] Final `no_std` verification of `tpt-planetary-core` — PARTIAL: core and array no_std builds pass locally; the CI job has not run

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
- [ ] Confirm `object_store` license and transitive deps; fall back to custom client if any are Apache-2.0-ONLY (top-level licence confirmed; transitive sweep still open)
- [ ] Confirm pure-Rust zstd *encoder* quality (ruzstd) is acceptable; otherwise decide write-side policy
- [ ] Confirm `tpt-math`/`tpt-simd` are `no_std`-compatible (answer so far: `tpt-math` is **not**; `tpt-simd` untested)
- [ ] Decide handling and redistribution of IERS EOP data and JPL DE440 files (size, licence/terms)
- [ ] 12-month solo timeline is aggressive; re-plan at end of each phase
- [x] Leap-second table checked against IERS `leap-seconds.list` (all 28 entries match); `valid_until` set to 2027-06-28 from that file's expiry line. Re-check before each release, because after expiry UTC conversions return `LeapSecondTableExpired`.
- [ ] Verify the Earth-Sun coefficients and the TDB series beyond three terms against primary sources (IERS, IAU, CODATA, Fairhead & Bretagnon). All were written from memory and are unvalidated. (The 28 TAI-UTC entries are now checked against the IERS list.) — PARTIAL: the Earth-Sun helper agrees with `erfa.epv00` to 9.99e-5 AU over 1900-2100 (docs/validation.md), but its coefficients are not checked against a primary source, the SPK-based Earth-Sun distance is not covered, and the TDB series beyond three terms is still open
- [x] Check `FINALS2000A_COLUMNS` in `tpt-planetary-astro/src/eop.rs` against the IERS finals2000A readme. The column positions were written from memory. (Checked against the readme column table, retrieved 2026-11-16: all columns matched except the LOD flag at column 79, which is blank in the readme; the flag was removed and `parses_readme_layout_line` covers it. No data file was parsed.)
- [ ] Rename or fix `icrs_to_ecliptic_at_epoch` in `tpt-planetary-astro/src/frames.rs`. It rotates by the epoch obliquity but keeps the J2000 equinox, so it is not the mean ecliptic of date.
- [ ] Check the W = R2(xp) R1(yp) polar-motion sign convention in astro against a reference. — PARTIAL: sign and axis order confirmed against `erfa.pom00` with s' = 0 (W equals its transpose to double precision; ERFA documents pom00 as TIRS -> ITRS). The TIO locator s' is still omitted (about 5.9e-11 rad at 2026), so the full IAU 2000 matrix is not matched.
- [ ] **Licence decision:** `cargo deny check` fails on `target-lexicon` 0.13.6 (Apache-2.0 WITH LLVM-exception, a build-time dependency of PyO3). The spec bans Apache-2.0-only licences. Options: a narrowly scoped `deny.toml` exception with a written rationale, or a different policy. Not changed without your decision.
- [ ] Library bugs fixed in this pass, with the remaining gaps: `to_calendar` and `Epoch::to_civil` now return `Result` and reject |day| > 5e11 (a 1e12 limit overflowed the i32 year). Array chunk geometry uses saturating multiplies; WCS longitude folds 360.0 to 0.0. Regression tests cover the calendar only; the array and WCS cases have no test yet.
