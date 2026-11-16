# Benchmarks methodology

Status: the micro-benchmark harness exists for three crates. The comparisons
against Astropy, xarray and netCDF4 are not done. The numbers below are
single-machine smoke runs. They are not validation and not a claim about
performance relative to other libraries.

## What is measured

Harness: [divan](https://crates.io/crates/divan) 0.1.21
(`MIT OR Apache-2.0`, `rust-version = 1.80`), declared as a dev-dependency
with `default-features = false`. Each bench target sets `harness = false`.

| Crate | Bench target | Benchmarks | What it times |
| --- | --- | --- | --- |
| `tpt-planetary-core` | `core` | `jd_calendar_round_trip` | `JulianDate::from_calendar` then `to_calendar`, Gregorian. |
| | | `utc_tai_round_trip` | `Epoch<Utc>` to `Epoch<Tai>` and back, with `leap::EMBEDDED`. |
| | | `great_circle_separation` | `angular_separation` for one pair of positions. |
| `tpt-planetary-array` | `array` | `zarr_v2_read_one_chunk` | `Array::read_chunk` for one 250x250 float64 chunk from a `MemoryStore`. |
| | | `zarr_v2_read_full_f64` | `Array::read_f64` for the whole 1000x1000 array (16 chunks). |
| | | `lazy_mean_full_array` | `Expr::mean` over the whole array, chunk-streamed. The array is reopened per sample, outside the timed region. |
| | | `codec_gzip_round_trip_1mib` | `GzipCodec` (level 6) encode then decode of 1 MiB. |
| | | `codec_lz4_round_trip_1mib` | `Lz4Codec` encode then decode of 1 MiB. |
| `tpt-planetary-astro` | `astro` | `icrs_to_galactic_10k` | `SkyCoord<Icrs>::transform_to::<Galactic>` for 10,000 positions. |
| | | `wcs_tan_pixel_to_world_10k` | `Wcs::pixel_to_world` (TAN) for 10,000 pixels (100x100). |

Inputs are generated in memory and are deterministic. No input is read from
disk and no network I/O is involved. The array benchmarks write the data into
a `MemoryStore` during setup, so the read benchmarks measure decode and copy
work, not storage.

Divan's `black_box` is applied to inputs. Return values are consumed by the
harness, so the optimiser cannot remove the work.

## Machine

Recorded for the runs in this document:

- CPU: Intel Core i7-12700 (12 cores, 20 logical processors).
- RAM: about 16 GB.
- OS: Windows 11 Pro (build 26200), Git Bash shell.
- Toolchain: rustc and cargo 1.85.0, `x86_64-pc-windows-msvc`, release
  profile with `opt-level = 3` as cargo's bench profile.
- Power plan, background load and thermal state were not controlled.

Results will differ on other machines, and between runs on this one. Treat
any number as an order of magnitude.

## How to run

From the workspace root:

```sh
# Compile only (what CI should check).
cargo bench -p tpt-planetary-core --no-run
cargo bench -p tpt-planetary-array --no-run
cargo bench -p tpt-planetary-astro --no-run

# Full run for one crate. Select the bench target with --bench; arguments
# after -- go to divan, not to the libtest harness of the lib target.
cargo bench -p tpt-planetary-array --bench array

# Quick run, for a smoke check of the harness and rough numbers.
cargo bench -p tpt-planetary-array --bench array -- --sample-count 3 --sample-size 5
```

Use `--bench <name>` with the target name (`core`, `array` or `astro`). Without
it, cargo also runs the library's own test harness with the divan arguments,
which fails with `Unrecognized option`.

Quick-mode numbers from the command above are noisy. Use the default sample
count for numbers you intend to compare.

## Quick-mode results (one run, this machine)

Command: `cargo bench -p <crate> --bench <target> -- --sample-count 5 --sample-size 1000`
(core), `--sample-count 5 --sample-size 20` (astro), and
`--sample-count 3 --sample-size 5` (array). Medians are shown.

| Benchmark | Median |
| --- | --- |
| `core::great_circle_separation` | 39 ns |
| `core::jd_calendar_round_trip` | 76 ns |
| `core::utc_tai_round_trip` | 1.08 us |
| `array::zarr_v2_read_one_chunk` | 144 us |
| `array::zarr_v2_read_full_f64` | 10.4 ms |
| `array::lazy_mean_full_array` | 18.5 ms |
| `array::codec_gzip_round_trip_1mib` | 5.4 ms |
| `array::codec_lz4_round_trip_1mib` | 273 us |
| `astro::icrs_to_galactic_10k` | 390 us (about 39 ns per coordinate) |
| `astro::wcs_tan_pixel_to_world_10k` | 687 us (about 69 ns per pixel) |

These are smoke numbers from a 3 to 5 sample run on one machine. They are not
validation, they do not establish correctness, and they are not used to claim
the engine is faster or slower than any other library.

A second quick run of the core and astro benches on the same machine, while
other builds were running in parallel, gave:

| Benchmark | Median (second run) |
| --- | --- |
| `core::great_circle_separation` | 50 ns |
| `core::jd_calendar_round_trip` | 58 ns |
| `core::utc_tai_round_trip` | 1.27 us |
| `astro::icrs_to_galactic_10k` | 486 us |
| `astro::wcs_tan_pixel_to_world_10k` | 838 us |

The spread between the two runs (up to about 60 percent for the astro
benches) is the size of the noise to expect from a shared desktop. A change
needs to be well outside that band before it means anything.

## What is not validated

- Correctness of outputs. The benchmarks discard results. Correctness is
  covered by unit tests, doctests and the planned reference-vector tests
  (`todo.md`, Phase 1 and Phase 2).
- Memory use. The lazy mean is chunk-streamed, but these benches do not
  measure allocator peaks. The memory-budget item in `todo.md` is separate.
- Cold and remote I/O. No S3, HTTP or filesystem store is benchmarked. All
  array benches use `MemoryStore`.
- Parallel execution. The lazy engine is single-threaded at present.
- Other platforms. Only Windows x86_64 has been run. Linux and macOS are not
  measured.
- Scalar accuracy of the astro benchmarks. They time the transforms only.

## Comparisons still open

The Astropy frame comparison and the NetCDF and xarray comparison are done (see
the sections above and below). The following still need to be done properly:

- **Reference implementations on the same machine.** Astropy for time scales, and
  the Zarr reference (`zarr-python`) for the chunk read. Record the exact versions
  of each in the results.
- **Matched workloads.** The same array shape, dtype, chunking and
  compression. For time conversion, the same number of epochs and the same
  leap-second table. For frames, the same coordinate set and the same
  precision. Write the inputs once to disk and feed them to every library.
- **Matched precision and outputs.** Confirm that the outputs agree within a
  stated tolerance before timing. A fast result that is wrong is not a
  comparison.
- **Warm and cold cases reported separately**, with the cache state stated.
- **A harness outside this repository's dependency tree**, typically a Python
  script run from a separate environment, so that Python packages do not
  enter the Rust lockfile. The script and its environment file should be
  committed alongside the results.
- **The data files and the generator script**, so that another person can
  regenerate the inputs. Large inputs should not be committed; record their
  checksums.
- **Hardware and OS details** as in the Machine section above, taken from the
  run itself and not typed by hand.
- **Licence review** of any new Python package used (the `deny.toml` policy
  covers Rust crates only).

## Adding a benchmark

1. Put the bench in the crate's `benches/` folder and name the target in the
   crate's `Cargo.toml` as a `[[bench]]` entry with `harness = false`.
2. Use only deterministic inputs. Generate them in a setup closure
   (`bencher.with_inputs`) when generation is not the thing being measured.
3. Run `cargo bench -p <crate> --no-run` so that it compiles without running.
4. Record the new benchmark and its command in this file if its numbers are
   quoted anywhere.

## Policy for quoted numbers

- A number quoted in a README, changelog or release note must come from a
  full run (default sample count), on a named machine, with the command and
  toolchain given.
- A quick-mode number is only for checking that a change did not break
  performance by an order of magnitude.
- Do not compare numbers across machines or toolchains.

## Reference comparison (Windows, Python)

Single run on the machine above, Python 3.13, `tpt_planetary` built with
`maturin develop --release`, astropy 8.0.2 and numpy 2.5.4. Both sides convert
the same 10,000 seeded random ICRS positions in one array call. The median of
50 runs (tpt_planetary) and 20 runs (astropy) is shown. The Astropy timing covers
only `transform_to("galactic")` on a `SkyCoord` built beforehand. It is not a
claim about performance in general, and it does not include the cost of
building the `SkyCoord`.

| Implementation | Call | Median | Min |
| --- | --- | --- | --- |
| tpt_planetary | `icrs_to_galactic(ra, dec)` | 0.550 ms | 0.541 ms |
| astropy 8.0.2 | `SkyCoord.transform_to("galactic")` | 1.885 ms | 1.851 ms |

The two outputs agree to 0.025 arcsec (see `docs/validation.md`).

## Reference comparison: NetCDF and xarray (Windows, Python)

Script: `bindings/tpt-planetary-py/examples/bench_vs_netcdf.py`. It writes one
2-D float64 field of 720 x 1440 (8.3 MB, seeded normal values, mean about 15)
into a temporary directory, as two files:

- a NetCDF-3 classic file, written with netCDF4 and `format="NETCDF3_CLASSIC"`;
- a Zarr v2 directory, written by the script (uncompressed, little-endian
  `<f8`, chunks 180 x 360, 16 chunk files).

It times three operations. Each timed run starts with a fresh open. One warm-up
run is discarded, then 5 timed runs are taken and the median is reported.

- `open`: open the dataset and read shape and dtype. No data is read.
- `slice`: open, then read `[200:520, 300:900]` into NumPy (320 x 600).
- `mean`: open, then the mean over the whole field.

Before timing, the script checks that the three means agree with the in-memory
mean to 1e-9 relative.

Command (from the repository root, with the project venv):

    "<venv>/Scripts/python.exe" bindings/tpt-planetary-py/examples/bench_vs_netcdf.py --runs 5

Machine: Intel Core i7-12700 (12 cores, 20 threads, 2.1 GHz base), 15.7 GB RAM,
WD PC SN740 512 GB NVMe SSD, Windows 11 Pro 10.0.26200. Python 3.13.7, numpy
2.5.4, xarray 2026.9.0, netCDF4 1.7.5.1.post1, `tpt_planetary` 0.1.0 (editable
install in the venv). Both files live on the same SSD and are in the OS file
cache during the timed runs.

| Library (format read) | open (ms) | slice (ms) | mean (ms) |
| --- | ---: | ---: | ---: |
| netCDF4 + numpy (NetCDF-3) | 2.52 (2.41-2.58) | 4.17 (4.09-4.30) | 9.16 (8.77-9.77) |
| xarray + numpy (NetCDF-3, netCDF4 engine) | 2.95 (2.93-3.00) | 4.50 (4.48-4.52) | 8.37 (8.26-8.63) |
| tpt_planetary (Zarr v2, release build) | 0.44 (0.42-0.47) | 2.46 (2.41-3.04) | 15.40 (15.34-15.67) |

Cells are median (min-max) of the 5 timed runs.

What is and is not comparable:

- **Different file formats.** The Python bindings cannot read NetCDF-3, so
  `tpt_planetary` is timed on the Zarr v2 copy of the same values, not on the
  NetCDF-3 file. The rows measure different formats and different readers. They
  are not a like-for-like file-format comparison.
- **Build profile.** The tpt_planetary rows were re-measured after
  `maturin develop --release`. An earlier debug-build run is superseded and not
  quoted.
- **Work done per call.** The `mean` row for tpt_planetary streams all chunks
  through the lazy engine with a per-element NaN/fill check and Welford update.
  netCDF4 and xarray call the C library and then NumPy's vectorised mean.
- **Warm cache only.** Every timed run reads from the OS file cache. No cold
  run, and no remote storage, was measured.
- **Single machine, single field size.** Results are not generalised to other
  shapes, chunkings, compressors or machines.
- **What is compared.** Only open, slice and mean. No write speed, no
  compression, no CF decoding, and no xarray dask chunking.
- **Agreement is checked.** The three means agree before timing, so no timing
  is taken for a wrong result.
