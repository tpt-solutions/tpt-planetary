# ADR-004: Scope decisions for 0.1

**Status:** Accepted (recommended options, applied without further review)

## Context

Several open items in `todo.md` need a decision before 0.1 rather than more
code. Each decision below takes the recommended option, so the work can
proceed. Any of them can be reopened.

## Decisions

1. **Reference data files are not redistributed.** EOP tables (IERS
   `finals2000A`) and JPL ephemeris kernels (DE440 and similar) are read from a
   path that the user supplies. The crate ships no data files and has no
   download helper. This avoids licence and size questions for the data.

2. **FK5 and FK4 are out of scope for 0.1.** The frames are ICRS (with the
   J2000 mean equator and equinox), Galactic, the ecliptic frames, Alt/Az and
   the CIRS/TIRS/ITRS chain. Legacy FK frames need equinox-dependent
   transforms that are not needed by the current users.

3. **No async runtime is chosen or added.** `AsyncStore` returns plain
   futures, so an application picks its own executor. The crate depends on no
   runtime.

4. **Write-side zstd uses ruzstd's fastest mode only.** Encoding is limited to
   the level that ruzstd implements, and the reader verifies content checksums.
   Stronger compression is a later change, when a better pure-Rust encoder is
   available.

5. **`tpt-math` is not used.** It is not `no_std`, and no code needs it. The
   array crate uses `tpt-simd-vector` only, behind the `simd` feature.

6. **Python bindings, version 0.1 scope.** Time scales, the frames already
   bound, WCS, and reading and reducing local Zarr arrays. Ephemeris bindings
   wait for kernel validation, and zero-copy NumPy, xarray and Dask adapters are
   deferred.

7. **Geodesics: Vincenty only.** The inverse and direct Vincenty formulae are
   provided, and they report non-convergence for nearly antipodal points.
   Karney's algorithm is deferred.

8. **TDB uses the three-term series for 0.1.** The full Fairhead and Bretagnon
   series needs coefficients that are not verified here, so the residual
   (up to about 15 microseconds in TT-TDB) is documented instead.

9. **Cloud stores beyond S3 are deferred.** GCS and Azure need TLS, which the
   crate does not include.

10. **Parquet is dropped; tpt-strata replaces it for tabular interchange.**
    Parquet and Arrow depend on Apache-2.0-only crates, which the licence
    policy excludes. The tabular bridge uses tpt-strata (MIT OR Apache-2.0,
    no external dependencies) behind the `strata` feature of
    `tpt-planetary-array`. Its only inputs are one-dimensional arrays, and its
    only output is CSV for now. The `tpt-strata-parquet` bridge crate is not
    used.

## Consequences

- The status page and `todo.md` record each decision.
- Items marked deferred are not blockers for 0.1, and each has a note on what
  would reopen it.
