# tpt-planetary TODO

Dual-licensed MIT OR Apache-2.0 · © 2026 TPT Solutions · Pure Rust, zero FFI · permissive dependencies only (no Apache-2.0-ONLY crates)

Legend: `- [ ]` open · `- [x]` done · **M** = milestone

---

## Phase 0: Project Setup & Compliance (Week 0-2)

### Repository & workspace
- [x] Initialise git repo with default branch `master` (`git init -b master`), `.gitignore`, `.gitattributes`
- [ ] Use `master` as the branch name in CI triggers, branch protection and release workflows — PARTIAL: `ci.yml` and `wheels.yml` trigger on `master`; no release workflow exists yet; branch protection is a GitHub setting and is not changed here
- [x] Create Cargo workspace root `Cargo.toml` (resolver 2, shared `[workspace.package]`, `[workspace.dependencies]`)
- [x] Scaffold crates: `tpt-planetary-core`, `tpt-planetary-array`, `tpt-planetary-astro`, `tpt-planetary-cli`, `bindings/tpt-planetary-py`
- [x] Decide and pin MSRV; add `rust-toolchain.toml`
- [x] Add `rustfmt.toml` and clippy config (`-D warnings`)
- [x] Set `license = "MIT OR Apache-2.0"` and `authors`/`repository` in every crate manifest
- [x] Confirm `tpt-math` and `tpt-simd` versions on crates.io; pin in workspace deps (`tpt-math` 0.1.0 is the crates.io latest and is pinned in the workspace; `tpt-simd-vector` 0.1.1 is the crates.io latest and is pinned in `tpt-planetary-array` for the `simd` feature; the umbrella `tpt-simd` is not used, see `docs/dependencies.md`)
- [x] Verify `tpt-math` / `tpt-simd` license and `no_std` support (needed by core): `tpt-math` is MIT OR Apache-2.0 but **not** `no_std`; `tpt-simd` not checkable on MSRV 1.85 (see `docs/dependencies.md`)

### Licensing
- [x] Add `LICENSE-MIT` (© 2026 TPT Solutions)
- [x] Add `LICENSE-APACHE` (Apache-2.0 text)
- [x] Add license section to README ("MIT OR Apache-2.0" + contribution clause)
- [x] Write `NOTICE`/third-party notices process (generate with cargo-about or similar) — `about.toml`, `about.hbs`, `THIRD-PARTY-NOTICES.md` and `docs/notices.md`; a CI check that the file is up to date is not yet written

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
- [ ] Time scale: TDB (approximation + full series to be validated in Phase 2) — PARTIAL: not in core (no TDB marker). The full 787-term series is in astro (`time::tdb_minus_tt_seconds`), which replaces the three-term approximation in ADR-004 for the astro crate; see the Phase 2 TDB item.
- [x] Time scale conversion API with typed time-scale markers
- [x] Spherical trigonometry: angular separation, position angle, great-circle ops
- [x] Spherical <-> Cartesian conversions; angle newtypes (rad/deg/hour-angle), wrapping/normalisation
- [x] Quaternions (via `glam`/`tpt-math`) for rotations; SLERP; Euler/axis-angle conversion
- [ ] Geodesic/ellipsoid helpers using `tpt-math` (Vincenty/Karney, WGS84) — PARTIAL: geodetic and ECEF conversion, and Vincenty's inverse and direct geodesics; no Karney. DECIDED (ADR-004): Vincenty only for 0.1; the helpers do not use `tpt-math`, which is not used anywhere.
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
- [x] Zarr v3 codec pipeline (array->array, array->bytes, bytes->bytes) — DONE: `transpose` and `sharding_indexed` are built by `ArrayMetadata::pipeline` with stage-order checks, and a transpose is allowed inside a shard's inner chain (see sharding item)
- [x] Zarr v3 sharding codec — DONE: index parse (`end`/`start`, crc32c), inner-chunk extract, encode with missing chunks and fill-value reads. Index-driven partial reads cover the pipelines `sharding_indexed` alone and `transpose` followed by `sharding_indexed` (the array reads the index with `Store::size` and `Store::get_range`, then only the inner chunks a selection touches; tests check the byte counts). Inner chains take a `transpose` before their bytes codec, plus bytes->bytes codecs, and are checked against zarr-python (`v3_sharded_inner_transpose`). A bytes->bytes codec around a whole shard is still read whole, because a compressed shard cannot be range-read; that is inherent, not a gap
- [x] Zarr conformance test suite: read real v2 and v3 datasets (generated by zarr-python) as fixtures — `tpt-planetary-array/tests/fixtures/gen_zarr_fixtures.py` writes 21 stores with zarr-python 3.4.2 and numcodecs 0.17.1 (v2: none, zlib, gzip, lz4, zstd, Blosc with lz4/zstd/zlib/blosclz and byte or bit shuffle, Blosc split and large-chunk cases, float NaN fill, partial writes that leave chunks missing; v3: bytes with zstd and gzip, transpose, sharding with a gzip inner chain). `tests/zarr_conformance.rs` compares every value with zarr-python's output. Not yet: datasets from other writers, and v3 codecs beyond these.
- [x] Write support: create arrays, write chunks, partial-chunk updates
- [x] Hierarchy (groups) traversal

### tpt-planetary-array: compression (pure Rust)
- [x] Codec trait + registry
- [x] zstd decode/encode via pure-Rust crate (decide on `ruzstd` and its encoder limits) — PARTIAL: decode verified (checksums too); encode at level 1 only; ruzstd 0.9.1 (MIT)
- [x] gzip/zlib via `miniz_oxide` / `flate2` (pure-Rust backend only)
- [x] LZ4 via `lz4_flex`
- [ ] Blosc-compatible decoding (pure Rust; evaluate need) — PARTIAL (encode added for LZ4, zlib and zstd): decodes memcpy, LZ4, zlib, zstd, blosclz and snappy, with byte and bit shuffle and split blocks. The zarr-python fixtures check lz4, zlib, zstd and blosclz, including split and large blocks. Snappy is checked against the cramjam raw-format reference. The blosclz far-distance back-reference (above 8191 bytes) is decoded and checked against a python-blosc frame (`testdata/blosclz_far.blosc`, from `tools/gen_blosc_far_fixture.py`). Encoding: LZ4, zlib and zstd, with byte or bit shuffle and the same split rule as decoding (`BloscCodec`, and the Zarr v2 writer emits the codec id). zarr-python reads the frames this crate writes (`examples/write_blosc_zarr.rs`, `tests/fixtures/check_blosc_written.py`). blosclz and snappy are decode-only.
- [x] Shuffle / bitshuffle / delta filters — PARTIAL: bitshuffle checked against a hand-computed transform and round trips only; its numcodecs/bitshuffle-library framing was written from memory and has no reference fixture
- [ ] Codec benchmarks and correctness vs reference files — PARTIAL: reference fixtures from numcodecs and zarr-python now cover gzip, zlib, zstd, lz4 and Blosc (see the Zarr conformance item); snappy uses cramjam vectors. Timing benchmarks now exist for gzip, lz4, zstd and Blosc (lz4, byte shuffle) round trips of 1 MiB (`tpt-planetary-array/benches/array.rs`: about 5.4 ms, 0.20 ms, 4.1 ms and 1.8 ms on this Windows machine).

### tpt-planetary-array: storage & async I/O
- [x] Async `Store` trait (`async-trait` or native async-fn-in-trait; confirm licensing) — `store/async_store.rs` defines `AsyncStore` with native `async fn`-style methods (`impl Future + Send`, no `async-trait` crate). Every `Store` gets it through a blanket impl with ready futures. `block_on` is a small thread-parking executor for tests and scripts. No async array reader yet.
- [x] Local filesystem store
- [x] In-memory store (tests)
- [ ] HTTP(S) read-only store with range requests — PARTIAL: `HttpStore` (`store/http.rs`) does `http://` GET and byte-range requests over `TcpStream` (Content-Length, chunked and read-until-close framing; 429/5xx mapped to transient errors); `Store::get_range` is a new trait method with a default. HTTPS is rejected: it needs a TLS implementation, which is not in the crate. Tested against a local TCP server.
- [ ] S3 store (SigV4 signing, credential chain, regions, anonymous access) — PARTIAL: `S3Store` (`store/s3.rs`) with path-style GET, ranged GET, HEAD-style size via Range, PUT, DELETE and paged ListObjectsV2. `store/sigv4.rs` implements SHA-256, HMAC-SHA256 and SigV4, checked against the RFC 4231 and AWS documentation vectors (including the GET Object example signature). Credentials are explicit or from `AWS_*` environment variables; anonymous access is supported. Only `http://` endpoints work (for example a local S3-compatible server): no TLS, no instance-metadata or profile credential chain, no multipart upload.
- [ ] Retry/backoff, concurrency limits, request coalescing of adjacent chunk ranges — PARTIAL: retry with exponential backoff (`retry::RetryStore`, transient errors only, injected sleep) is done; concurrency limits are done (`store::LimitedStore`, a permit pool with a test that peak concurrency stays within the limit); range coalescing is done for the inner chunks of one shard (ranges within 4 KiB merge into one request, with a test that counts requests). For ordinary (unsharded) chunks there is nothing to coalesce: each chunk is its own key, and the `Store` trait cannot merge reads across keys, so that part is closed as not applicable; concurrency is handled by `LimitedStore`
- [x] Chunk cache (LRU) with memory budget (`Array::with_chunk_cache`, decoded chunks, `std` feature)
- [x] Async runtime choice and feature gating (e.g. tokio, MIT) — DECIDED (ADR-004): no runtime dependency; `AsyncStore` is runtime-agnostic and `block_on` is provided for tests and scripts.
- [ ] Mock S3 (local) integration tests — PARTIAL: `S3Store` is tested against a fake HTTP server that records request heads (signature headers, paths, paging and error codes), and `tptplan array inspect s3://...` is tested end to end in the unit tests. Not yet run against MinIO or another real S3-compatible server; `tests/fixtures` has no S3 data.

### tpt-planetary-array: lazy engine
- [x] Lazy array handle (metadata only, no data load)
- [x] Lazy execution graph (nodes: load, slice, elementwise, reduce)
- [x] Chunk-wise streaming executor (bounded memory)
- [x] Reductions: sum, mean, min, max, count, std (numerically stable, e.g. Welford) across chunks
- [x] Masked / NaN-aware (fill value) reductions
- [x] Parallel chunk execution (`threads` feature, off by default; `EvalOptions::workers`; results bit-identical to the serial path; the executor is serial otherwise)
- [ ] Memory-budget tests (prove no OOM on large synthetic data) — PARTIAL: bound checked through ExecStats block counts, not allocator measurement

### Phase 1 wrap-up
- [x] Minimal `tptplan array inspect` prototype (for milestone)
- [ ] **M1: `tptplan array inspect` reads a 100 GB Zarr dataset from S3 and computes a global mean in seconds without OOM**
- [ ] Phase 1 docs, examples, and release-candidate review — PARTIAL: user guide written; no release-candidate review

---

## Phase 2: Astronomy & Ephemeris (Months 4-6)

### Time (tpt-planetary-astro)
- [x] UTC <-> TAI <-> TT with leap seconds, handling the UTC day-length discontinuity
- [ ] UT1 and Earth Orientation Parameters (IERS Bulletin A/B/C04 parsing; bundled snapshot + user-supplied data) — PARTIAL: the finals2000A column map was checked against the IERS readme and against real rows (`tests/eop_finals.rs`): UT1-UTC agrees with astropy's IERS-B table to 30 us and polar motion to 1 mas at the sampled epochs. The UT1 scale is in `tptplan astro convert` (`--eop FILE`, user-supplied, as ADR-004 decides; no bundled snapshot). C04 is parsed (`parse_c04`, `parse_eop_text`, checked against the finals rows for shared dates). Not yet: UT1 in the frame transforms beyond ERA. Gaps of more than a day are an error by design
- [x] TDB <-> TT (Fairhead & Bretagnon series, high precision) — DONE: the full 787-term series with the JPL-mass terms (`src/tdb_data.rs`, generated by `tools/gen_tdb_table.py` from ERFA's `dtdb.c`, which is derived from SOFA), geocentre only. Matches `erfa.dtdb` to a maximum difference below 0.0005 us on 400 random TT epochs in 2000-2026 plus fixed epochs (`test_time_scales.py`; the Rust unit test checks six epochs from 1900 to 2100 to 1 ns). The topocentric terms are not included, and the coefficients come from ERFA's copy rather than the 1990 paper.
- [ ] Earth Rotation Angle, GMST, GAST — PARTIAL: ERA and GMST are validated against pyerfa. GAST (`time::gast_iau2006`) includes the complementary terms of the equation of the origins, and agrees with `erfa.gst06a` to the published-model residual of about 6 uas.
- [ ] Validate against SOFA/ERFA published vectors and Astropy outputs (as test data only) — PARTIAL: checked against pyerfa 2.0.1.5 and astropy 8.0.2 (see docs/validation.md): leap steps, ERA, GMST, obliquity, ICRS-Galactic and WCS pass; TT-TDB now agrees with erfa.dtdb to below 0.0005 us (full series); SPK not validated; nutation (IAU 2000A_R06) validated against pyerfa `nut06a`, see docs/validation.md; IAU 2006 precession validated against pyerfa (see docs/validation.md)

### Coordinate frames
- [x] Frame trait / transformation graph (typed frames, attributes such as obstime/location)
- [x] ICRS <-> Galactic
- [ ] ICRS <-> Ecliptic (mean and true) — PARTIAL: mean ecliptic of J2000 (`MeanEcliptic`), mean ecliptic of date (`precession::icrs_to_mean_ecliptic_of_date`, IAU 2006 precession and mean obliquity, checked: the ecliptic pole moves 47.0" per century) and true ecliptic of date (`nutation::icrs_to_true_ecliptic_of_date`, nutation added to the obliquity; no frame bias). The CLI exposes them as `ecliptic-date` and `true-ecliptic-date`, with `--epoch` (TT, ISO 8601) required.
- [x] IAU 2006 precession and IAU 2000A/B nutation — DONE: IAU 2006 precession (zeta_A, z_A, theta_A and P(t)) is in `tpt-planetary-astro/src/precession.rs`, validated against pyerfa `p06e`/`bp06`. IAU 2000A_R06 nutation (full luni-solar and planetary series, Tables 5.3a/5.3b, generated by `tools/gen_nutation_table.py`) is in `src/nutation.rs`, validated against pyerfa `nut06a`, `num06a` and `ee06a` (see docs/validation.md). IAU 2000B is in `nutation::nutation_angles_2000b` (77 terms from ERFA `nut00b.c` via `tools/gen_nut00b_table.py`, matching `erfa.nut00b` to 1e-13 rad; 2.3 mas maximum from IAU 2000A over 1900-2100). The complementary terms of the equation of the equinoxes (Table 5.2e) are in `src/cio.rs`, validated, and used by `equation_of_equinoxes` and GAST; the frame bias is now available in the nutation chain as `nutation::true_equator_of_date_matrix_with_bias` (N P B; the existing matrix stays bias-free). The CIP and CIO series are in `src/cio.rs` (see the ICRS <-> CIRS item).
- [x] Aberration, light deflection (optional), proper motion & parallax propagation — DONE in `astrometry.rs`: annual aberration (checked against ERFA `ab`: 0.0005 arcsec with exact velocities, 0.02 arcsec over 1900-2100 with the Keplerian Earth velocity), light deflection by the Sun for a source at infinity (checked against ERFA `ld`: 3e-8 arcsec), proper motion (along the great circle) and parallax. Relativistic aberration terms (0.002 arcsec) are omitted. The Alt/Az transform still excludes these corrections (its validation predates them).
- [ ] Observer location (geodetic/ITRS), polar motion — PARTIAL: `frames::GeodeticObserver` on WGS84 with its ITRS position, and `icrs_to_altaz_at` at a given epoch; polar motion and UT1 are caller inputs; no EOP data is bundled
- [ ] ICRS <-> CIRS <-> TIRS/ITRS — PARTIAL: matrices only. `src/cio.rs` gives `icrs_to_cirs`, `cirs_to_tirs`, `tirs_to_itrs` and `icrs_to_itrs` (IAU 2006/2000A, IERS 2010 Tables 5.2a, 5.2b, 5.2d and Eq. 5.13). Given ERFA's components the chain matches `erfa.c2t06a` to 2e-18; end to end it differs by 3.6 uas, the same as the disagreement between ERFA's own X, Y routes (docs/validation.md). Typed `Cirs`, `Tirs` and `Itrs` frames with `transform_at` (all twelve pairs, epoch and polar motion in a `TransformContext`) are in `src/frames.rs`: the composition equals `icrs_to_itrs` and round trips are the identity (unit tests). Open: UT1 and polar motion from EOP data; IAU 2000B; the polar-motion history term of s' (Eq. 5.4), see the next sub-item.
  - [x] TIO locator polar-motion term (Eq. 5.4) — DECIDED: follow ERFA. The chapter's Eq. 5.4 has an unstated integration constant, so it cannot be implemented unambiguously; ERFA's `pom00` (which uses `sp00`) uses only the secular s' = -47 uas per century, and the crate matches it (Eq. 5.13). The polar-motion history term is therefore not included, and the matrix agrees with ERFA's CIO-based chain
  - [x] CIP coordinates X, Y (Tables 5.2a, 5.2b): match `erfa.xy06` to 1.1e-6 uas over 1900-2100
  - [x] CIO locator s (Table 5.2d): matches `erfa.s06` to 3.2e-9 uas
  - [x] TIO locator s' = -47 uas t (Eq. 5.13): matches `erfa.sp00`
  - [x] Frame bias matrix (Eqs. 5.17, 5.21, 5.33): matches `erfa.bp00` to 7e-13
  - [x] EO complementary terms (Table 5.2e, `cio::eo_complementary_terms`): match `erfa.eect00` to 4e-11 uas; wired into `equation_of_equinoxes` and GAST
  - [x] Generated tables with pinned checksums and row-count checks (`tools/gen_cio_table.py`)
- [ ] Local Horizontal (Alt/Az) with optional atmospheric refraction — PARTIAL: `frames::icrs_to_altaz_at` (ICRS to geodetic Alt/Az via the CIO chain) with no refraction, validated against `erfa.apco` without annual aberration (max 0.37 arcsec) and against Astropy after removing the annual aberration (max 0.36 arcsec); see docs/validation.md. Refraction is in `frames::icrs_to_altaz_refracted_at` with the ERFA `refco` constants (`refraction_coefficients`, matching `erfa.refco` to 1e-15 relative), and its zenith distance matches `erfa.atco13` to 0.003 arcsec up to 75 degrees. Not yet done: the refracted Alt/Az is not validated end to end against Astropy, and the CLI and Python bindings do not expose it.
- [x] FK5 / FK4 legacy frames (decide scope) — DECIDED (ADR-004): out of scope for 0.1; ICRS is the base frame.
- [ ] Accuracy tests (target: sub-milliarcsecond vs SOFA vectors) — PARTIAL: no SOFA vectors; the ERFA-based checks in docs/validation.md give sub-mas agreement for ERA, GMST and obliquity only

### Ephemeris (JPL DE440)
- [x] DE440/DE440s binary (SPK/DAF) file format parser
- [x] Segment/record lookup by epoch
- [x] Chebyshev polynomial evaluation (position and velocity)
- [x] Body/barycenter hierarchy (Sun, planets, Moon, EMB, Earth-Moon offset)
- [ ] Memory-mapped and in-memory loading; `no_std` slice-based API — PARTIAL: in-memory and byte-slice APIs; no memory mapping
- [x] Decide how data files are obtained (user-supplied path / download helper); document licensing of JPL data — DECIDED (ADR-004): user-supplied paths only; no data files and no download helper ship with the crate.
- [ ] Light-time correction and apparent positions — PARTIAL (apparent directions now done, see the apparent item below): `ephemeris::astrometric` gives the converged light-time corrected direction (RA, Dec, distance, light time, no stellar aberration), which agrees with JPL Horizons astrometric positions to 0.015 arcsec at five epochs (`test_ephemeris_horizons.py`). Apparent positions are in `ephemeris::apparent_direction` (Sun light deflection, then ERFA's `eraAb` aberration with the relativistic terms, from the kernel's observer state), checked against Horizons apparent positions at five epochs to 0.11 arcsec. The residual has a systematic RA offset of about +0.05 arcsec. An independent pyerfa chain (ld, ab, pnm06a on the kernel vectors) reproduces it to 1e-4 arcsec, so the code matches its inputs; light deflection moves it by up to 0.07 arcsec and is the leading candidate for the model difference
- [x] Validate against JPL Horizons reference values — DONE for astrometric RA and Dec of Jupiter and Saturn at 2017, 2025 and 1950 (kernel-dependent: the test needs `TPT_SPK_KERNEL`, a user-supplied DE440-style kernel). Measured maximum 0.014 arcsec, against Horizons' own output precision of 0.036 arcsec. Geometric states agree with an independent SPK evaluation (jplephem) to 0.05 km
- [ ] **M2: `tptplan astro ephemeris` computes Jupiter's position to sub-arcsecond accuracy for any timestamp** — the astrometric direction is sub-arcsecond at the sampled epochs (see above); it is not checked for every timestamp in the kernel, so the milestone stays open. The CLI command is `tptplan ephemeris query jupiter EPOCH --kernel PATH --astrometric`

### Phase 2 wrap-up
- [x] Earth-Sun distance helper (used by the climate/irradiance data-flow example)
- [ ] Docs, examples, and benchmarks vs Astropy — PARTIAL: docs and divan benches; no Astropy comparison

---

## Phase 3: Advanced Compute & WCS (Months 7-9)

### Map algebra (tpt-simd)
- [x] Integrate `tpt-simd` into `tpt-planetary-array` — via the optional `simd` feature (tpt-simd-vector, not the umbrella crate); no measured speedup on this machine
- [ ] Vectorised elementwise ops (+, -, *, /, pow, abs, sqrt, exp, log, trig) — PARTIAL: add, sub, mul, div, abs and sqrt are vectorised under the simd feature; pow, exp, log, trig and comparisons are scalar only
- [x] Vectorised comparisons, masks, `where`/select
- [ ] SIMD reductions (sum, mean, min, max, std) with scalar fallbacks — PARTIAL: full-array sum, mean, min and max are vectorised under the simd feature; per-axis reductions are scalar
- [x] Broadcasting rules (N-D)
- [ ] Axis-wise reductions and group-by style aggregations (e.g. monthly climatology) — PARTIAL: axis reductions, fixed-period climatology/anomaly (`Expr::climatology`, `Expr::anomaly`), and group means on integer labels (`Expr::group_mean`, NaN-aware) are done. Calendar-month labels from CF time coordinates exist (`netcdf3::calendar_month_labels`, all CF calendars except months/years units); per-axis SIMD reductions are not done
- [x] Rolling/window ops and resampling along time
- [x] Operation fusion in lazy graph (e.g. `array * 2 + 1` in one pass)
- [x] Spherical-aware weighting (cos(lat) area weights) for global means
- [ ] Correctness tests vs scalar reference; benchmark vs NumPy/xarray — PARTIAL: scalar reference tests done; no benchmarks

### NetCDF compatibility (pure Rust)
- [x] NetCDF3 Classic / 64-bit offset reader
- [x] NetCDF3 writer
- [ ] CF-conventions metadata handling (time units, calendars, coordinates) — PARTIAL: time units days, hours, minutes and seconds; all CF calendars except `none` (standard/gregorian, proleptic_gregorian, noleap/365_day, all_leap/366_day, 360_day, julian); units of months or years and coordinate/bounds handling are not done
- [x] NetCDF-style API surface (dimensions, variables, attributes)
- [x] Interop tests using files produced by netCDF4/xarray — PARTIAL but in place: `tests/netcdf3_interop.rs` reads NetCDF-3 classic and 64-bit files written by netCDF4 (`fixtures/gen_netcdf3_fixtures.py`), and a file written by this crate's writer is read back by Rust and checked by netCDF4 and xarray (`fixtures/check_netcdf_writer.py`). No NetCDF-4 (HDF5) files; those are read by `hdf5` (see the Phase 5 item).

### WCS (tpt-planetary-astro)
- [x] FITS header WCS keyword parser (CRPIX, CRVAL, CDELT, CD/PC matrix, CTYPE)
- [x] Linear pixel <-> intermediate world transform
- [x] Projections: TAN (gnomonic)
- [x] Projections: SIN (orthographic/slant)
- [x] Projections: AIT (Hammer-Aitoff)
- [x] Native <-> celestial spherical rotation (LONPOLE/LATPOLE)
- [ ] Round-trip pixel->sky->pixel accuracy tests vs wcslib/astropy reference values — PARTIAL: synthetic round trips only; no wcslib comparison
- [x] SIP distortion (decide scope; may defer to Phase 5) — DECIDED and implemented: forward SIP (A, B) is applied, and the world-to-pixel inverse uses AP/BP when present or Newton iteration otherwise (`wcs.rs`). TPV is not implemented.

### Climate indices
- [x] Anomaly and climatology computation (lazy) — for a fixed period along one axis (`Expr::climatology`, `Expr::anomaly`); calendar-aware grouping is open under the Phase 3 group-by item
- [x] El Nino indices (e.g. Nino3.4 SST anomaly) as a worked, lazy pipeline — DONE (Python bindings `nino34` and `nino34_relative` on a `ZarrArray`, tested in `bindings/tpt-planetary-py/tests/test_climate.py`): `ops::climate` (monthly anomaly, area-weighted box mean, Niño-3.4 and the relative index that subtracts the 20°S–20°N mean). The field is a lazy expression over a Zarr array and only the per-month results are materialised. Tested against a direct computation on a 60-month synthetic Zarr field (`tests/climate_index.rs`); the relative index reads the same field twice, one region at a time, not two variables
- [ ] **M3: `tptplan array math` performs a complex multi-variable climate index calculation (e.g. El Nino index) lazily** — the calculation exists as a library pipeline (see above), but `tptplan array math` is elementwise only, so the index is a separate command, `tptplan array index nino34|nino34-relative --input FIELD --lat-start .. --lat-step .. --lon-start .. --lon-step ..` (`ops::climate`, tested against a direct computation). The milestone text names `array math`; the milestone stays open until its wording is settled, since widening `array math`'s grammar would blur its elementwise contract

---

## Phase 4: CLI, Python Bindings & Release (Months 10-12)

### tpt-planetary-cli (`tptplan`)
- [x] `clap`-based CLI skeleton, global flags (`--verbose`, `--format json|table`), shell completions
- [ ] `tptplan array inspect <uri>` (shape, dtype, chunks, attrs; local/S3/HTTP) — PARTIAL: local Zarr, `http://` URLs, and `s3://bucket/key` (needs `TPT_S3_ENDPOINT` set to an http:// endpoint; credentials from `AWS_*` variables) work; `https://` returns exit 3 because no TLS is built in. Tested end to end with a local HTTP server; S3 is tested by the fake-server unit tests.
- [x] `tptplan array math` (expression evaluation, lazy, output to Zarr/NetCDF3) — PARTIAL but complete for the grammar: `+ - * /`, unary minus, parentheses and scalar operands. Output is a float64 Zarr array (chunk by chunk, v2 or v3) or a NetCDF-3 file when the name ends in `.nc` (the whole result is held in memory for that case).
- [x] `tptplan array convert` (Zarr <-> NetCDF3) — numeric int8/int16/int32/float32/float64 only; a NetCDF-3 file becomes a Zarr group with one array per variable, and a Zarr array or flat group becomes a NetCDF-3 file; no CF time decoding, NetCDF-4, or remote paths
- [ ] `tptplan astro convert` (time scales and coordinate frames) — PARTIAL: utc, tai, tt and tdb (TDB through the full series, geocentre); frames icrs, galactic, ecliptic (mean J2000), and the mean and true ecliptic of date (with `--epoch`, TT); UT1 with `--eop FILE` (a user-supplied finals2000A file); no TCB or TCG, and no other frames
- [ ] `tptplan ephemeris query` (body, epoch, observer, output frame) — PARTIAL: with a kernel, the geometric query (agreeing with jplephem to 0.05 km) and `--astrometric` (agreeing with Horizons to 0.015 arcsec) both work; the kernel is user-supplied
- [x] Progress reporting, error messages, exit codes
- [x] CLI integration tests; man pages / help text
- [ ] Prebuilt binaries for Linux/macOS/Windows (release workflow)

### tpt-planetary-py (PyO3, MIT)
- [x] Confirm `pyo3` / `maturin` license and abi3 policy
- [x] PyO3 module scaffold, `maturin` build config
- [x] Bind core: time scales, angles, spherical math
- [ ] Bind astro: frames, ephemeris, WCS, SGP4/SDP4 (`Satellite`, checked against the sgp4 reference rows), PCK orientation — PARTIAL (scope per ADR-004): ICRS and Galactic frames and WCS are bound and their validation suite runs. The SPK kernel is bound (`open_spk`, `SpkKernel.state`, `astrometric`, `apparent`, `et_from_tdb`), tested against Horizons when a kernel is supplied; the rest of the ephemeris API is not bound.
- [ ] Bind array: open Zarr/NetCDF3/HDF5 (HDF5 read-only through `open_hdf5`, tested against the h5py fixtures), slice, lazy ops, compute — PARTIAL: local Zarr with slicing, float64 reads and reductions; NetCDF-3 files can be opened, listed and read whole or by selection (`open_netcdf3`; the whole variable is decoded first, so no partial I/O); no remote stores
- [ ] NumPy interop (zero-copy where possible) via `numpy` crate — PARTIAL: copies on read, no zero-copy path
- [ ] xarray / Dask integration (backend entrypoint or adapter) and SunPy/Astropy interop notes
- [ ] Async/GIL handling for S3 streaming — PARTIAL: GIL released for reads and reductions; no S3 streaming
- [x] Type stubs (`.pyi`) and Python tests (pytest) — the suite now runs: 321 passed, 3 xfailed with astropy, sgp4 and numpy installed. Three test-harness faults were fixed (float formatting of numpy scalars, a TLE line-2 generator, and reference-decayed states), and the WCS checks found a CAR longitude-range bug and a LATPOLE root-selection bug, both fixed in Rust.
- [ ] Wheels for manylinux/musllinux, macOS (x86_64, arm64), Windows via CI — PARTIAL: wheel and sdist workflow written in .github/workflows/wheels.yml, not yet run on a CI runner. A local sdist packages the sibling crates and the vendored target-lexicon (a direct path dependency of the bindings crate) and builds offline when extracted. Publishing is out of scope.
- [x] Jupyter example notebook (the climate data-flow example with Matplotlib) — `bindings/tpt-planetary-py/examples/climate_dataflow.ipynb`, executed with nbclient; synthetic Zarr v2 written by hand

### Benchmarks & validation
- [ ] Benchmarks vs xarray/netCDF4 (open, slice, mean, S3 streaming) — PARTIAL: open, slice and mean against netCDF4 and xarray on the same NetCDF-3 field (Windows, Python; `docs/benchmarks.md`); tpt-planetary is timed on a Zarr copy because Python cannot read NetCDF-3; no S3 streaming
- [ ] Benchmarks vs Astropy (time conversion, frame transforms, ephemeris) — PARTIAL: time conversion and frame transform benchmarks done against astropy (docs/benchmarks.md); no ephemeris comparison
- [ ] Publish reproducible benchmark methodology and results
- [ ] End-to-end test of spec section 7 data-flow example — PARTIAL: scaled-down test on synthetic data; no SPK kernel, so no kernel-based ephemeris check
- [ ] Final licence/dependency audit (`cargo deny`, SBOM, third-party notices) — PARTIAL: `THIRD-PARTY-NOTICES.md` is generated by cargo-about (`docs/notices.md`), but it lists the workspace's own crates and there is no SBOM; cargo deny passes with the MIT substitute for target-lexicon
- [ ] Final `no_std` verification of `tpt-planetary-core` — PARTIAL: core, array and astro `--no-default-features` builds pass for `thumbv7em-none-eabihf` locally; the CI job has not run

### Docs & release
- [ ] User guide, API docs (docs.rs), tutorials, migration guides (Astropy / NetCDF4 / xarray -> tpt-planetary)
- [ ] Complete `CHANGELOG.md`, version bump to 0.1.0 — PARTIAL: CHANGELOG brought up to date with the unreleased work (Added, Changed, Fixed); the version bump is a release step and is left for you (publishing and release steps are skipped as requested)
- [ ] Publish crates in dependency order: core -> array -> astro -> cli
- [ ] Publish `tpt-planetary` wheels to PyPI
- [ ] Tag release and publish GitHub release notes
- [ ] **M4: v0.1.0 released on crates.io and PyPI with published benchmarks**

---

## Phase 5: Post-Release (v0.2+)

- [x] Read-only HDF5 / NetCDF4 subset reader (superblock, B-trees, contiguous/chunked, deflate), pure Rust — DONE for the default file format: `tpt-planetary-array/src/hdf5` reads superblocks v0 to v3, object headers v1 and v2, old-style and link-message groups, dataspaces v1 and v2, contiguous, compact and chunked (version 3 layout, B-tree index) data with deflate, shuffle and fletcher32, fill values, and attributes. Checked against h5py 3.16 (HDF5 2.0) fixtures (`tests/hdf5_fixtures.rs`, `tools/gen_hdf5_fixtures.py`). Not read, and reported as unsupported: dense (fractal-heap) groups and attributes, version 4 layouts and their chunk indexes (what `libver='latest'` writes for chunked data), variable-length strings, compound and other classes, and soft or external links. netCDF-4 files that use only these features read as HDF5 datasets; their dimension scales are attributes and are not interpreted
- [ ] Write HDF5 (evaluate demand)
- [ ] GCS and Azure Blob stores — DEFERRED (ADR-004): both need TLS, which the crate does not include.
- [ ] Zarr v3 extensions (more codecs, data types) and Zarr spec updates
- [x] Parquet/Arrow-concept interop — DROPPED for licence reasons: Parquet and Arrow dependencies are Apache-2.0 only, which the policy excludes (no GPL, no Apache-only). REPLACED by a tpt-strata bridge (`tpt-planetary-array`, feature `strata`): 1-D Zarr arrays become tpt-strata columns (NaN becomes NULL), with CSV output. tpt-strata is MIT OR Apache-2.0 with zero dependencies; `cargo deny` passes with the feature enabled.
- [ ] More WCS projections (ARC, ZEA, STG, CAR, MOL, HPX, etc.) and SIP/TPV distortion — PARTIAL: ARC, ZEA, STG, CAR, MOL and HPX are implemented in `wcs.rs`; SIP is applied, with AP/BP or Newton inversion; TPV is implemented (`-TPV` CTYPE, the 40 `PV1_k`/`PV2_k` terms in wcslib's ordering, with the pixel-scale variables and CDELT scaling of wcslib) and checked against wcslib through astropy, at 1e-9 degrees and 1e-6 pixels
- [ ] DE441 and other ephemerides; SPICE kernel (SPK/CK/PCK) support — PARTIAL: SPK types 2 and 3 are read; binary PCK type-2 orientation is read (`pck::orientation_matrix`, J2000 and ECLIPJ2000 reference frames, ITRF93), matching SPICE's pxform to 4e-15 over 28 epochs (`tests/validation/test_pck_spice.py`). CK (attitude) kernels are not implemented
- [ ] Orbital mechanics: SGP4/TLE propagation, Keplerian elements, orbit determination helpers — PARTIAL: `orbit.rs` has Keplerian elements, Kepler's equation, TLE parsing, the SGP4 model with the SDP4 deep-space terms (`orbit/sdp4.rs`: lunar-solar periodics and one-day and half-day resonance, matching the sgp4 package to 1.2e-9 km over 226 states; the resonance state is not carried between calls, as in that package), Gibbs velocity from three positions (`gibbs_velocity`, exact for two-body orbits) and Herrick-Gibbs velocity for closely spaced positions (`herrick_gibbs_velocity`, error falls by about 16 per halving of the spacing). Full orbit determination is not implemented.
- [ ] FITS image I/O (pure Rust) — PARTIAL: `fits.rs` reads and writes primary and IMAGE HDUs (BITPIX 8 to -64, BZERO/BSCALE). Binary tables (`XTENSION = 'BINTABLE'`) are read by `Hdu::table` for the fixed-width forms L, B, I, J, K, E, D and A, with TSCAL/TZERO and TNULL, checked against astropy (`tests/fits_table.rs`, `tools/gen_fits_table_fixtures.py`). Tables are written by `write_bintable` for the same forms (round trip and astropy checked). Not yet: ASCII tables, variable-length columns (P/Q), bit and complex columns, and tile compression; the reader is `no_std`-capable, the file functions need `std`
- [ ] GPU-accelerated compute (if/when tpt-simd supports)
- [ ] Distributed execution / Dask-like scheduler
- [ ] Ongoing: dependency license re-audits, MSRV policy, security advisories, community issue triage

---

## Open questions / risks to track
- [ ] Confirm `object_store` license and transitive deps; fall back to custom client if any are Apache-2.0-ONLY (top-level licence confirmed; transitive sweep still open)
- [x] Confirm pure-Rust zstd *encoder* quality (ruzstd) is acceptable; otherwise decide write-side policy — DECIDED (ADR-004): ruzstd's fastest mode is the write side; the reader verifies checksums.
- [x] Confirm `tpt-math`/`tpt-simd` are `no_std`-compatible (answer so far: `tpt-math` is **not**; `tpt-simd` untested) — DECIDED (ADR-004): `tpt-math` is not used; `tpt-simd-vector` is used behind the `simd` feature (it is `no_std`).
- [x] Decide handling and redistribution of IERS EOP data and JPL DE440 files (size, licence/terms) — DECIDED (ADR-004): not redistributed; user-supplied.
- [ ] 12-month solo timeline is aggressive; re-plan at end of each phase
- [x] Leap-second table checked against IERS `leap-seconds.list` (all 28 entries match); `valid_until` set to 2027-06-28 from that file's expiry line. Re-check before each release, because after expiry UTC conversions return `LeapSecondTableExpired`.
- [ ] Verify the Earth-Sun coefficients and the TDB series beyond three terms against primary sources (IERS, IAU, CODATA, Fairhead & Bretagnon). All were written from memory and are unvalidated. (The 28 TAI-UTC entries are now checked against the IERS list.) — PARTIAL: the Earth-Sun helper agrees with `erfa.epv00` to 9.99e-5 AU over 1900-2100 (docs/validation.md), but its coefficients are not checked against a primary source, and the SPK-based Earth-Sun distance is not covered. The TDB series is now the full 787-term series, taken from ERFA's `dtdb.c` and checked against pyerfa; checking its coefficients against the 1990 paper itself is still open
- [x] Check `FINALS2000A_COLUMNS` in `tpt-planetary-astro/src/eop.rs` against the IERS finals2000A readme. The column positions were written from memory. (Checked against the readme column table, retrieved 2026-11-16: all columns matched except the LOD flag at column 79, which is blank in the readme; the flag was removed and `parses_readme_layout_line` covers it. No data file was parsed.)
- [x] Rename or fix `icrs_to_ecliptic_at_epoch` in `tpt-planetary-astro/src/frames.rs`. It rotated by the epoch obliquity but kept the J2000 equinox, so it was not the mean ecliptic of date. RESOLVED: renamed to `icrs_to_ecliptic_epoch_obliquity` and documented as not the mean ecliptic of date; a mean-ecliptic-of-date transform is not implemented.
- [ ] Check the W = R2(xp) R1(yp) polar-motion sign convention in astro against a reference. — PARTIAL: sign and axis order confirmed against `erfa.pom00` with s' = 0 (W equals its transpose to double precision; ERFA documents pom00 as TIRS -> ITRS). The TIO locator s' is still omitted (about 5.9e-11 rad at 2026), so the full IAU 2000 matrix is not matched.
- [x] **Licence decision (resolved):** `cargo deny check` fails on `target-lexicon` 0.13.6 (Apache-2.0 WITH LLVM-exception, a build-time dependency of PyO3). The spec bans Apache-2.0-only licences. Options: a narrowly scoped `deny.toml` exception with a written rationale, or a different policy. Not changed without your decision. RESOLVED 2026-11-16: target-lexicon is patched with the MIT-licensed substitute at ../target-lexicon-mit; cargo deny passes. CI needs that path or a published substitute.
- [x] Library bugs fixed in this pass: `to_calendar` and `Epoch::to_civil` now return `Result` and reject |day| > 5e11 (a 1e12 limit overflowed the i32 year). Array chunk geometry uses saturating multiplies, with a regression test that an overflowing offset saturates rather than wraps. WCS longitude folds 360.0 to 0.0, with a regression test.
