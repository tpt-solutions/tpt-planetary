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

## Comparisons not done yet

The `todo.md` items for benchmarks against Astropy, xarray and netCDF4 are
open. To do them properly they need:

- **Reference implementations on the same machine.** Astropy (time and frame
  transforms), xarray with netCDF4 or h5netcdf (open, slice, mean), and the
  Zarr reference (`zarr-python`) for the chunk read. Record the exact versions
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
