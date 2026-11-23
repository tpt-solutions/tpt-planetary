# tpt-planetary-cli (`tptplan`)

`tptplan` is the command-line front end for [tpt-planetary](https://github.com/tpt-solutions/tpt-planetary).
It inspects and converts Zarr and NetCDF-3 arrays, evaluates arithmetic over
them, converts timestamps and sky coordinates between time scales and frames,
and queries planetary ephemerides from SPK kernels.

**Status:** version 0.1.0, not yet published to crates.io. The binary is
`tptplan`. Several subcommands are partial, and some return exit code 3 (not
implemented) on purpose. The table in [Exit codes](#exit-codes) says which.

## Contents

- [Installation](#installation)
- [Global options](#global-options)
- [Commands](#commands)
- [Exit codes](#exit-codes)
- [Environment variables](#environment-variables)
- [Limitations](#limitations)
- [Development](#development)
- [Changelog](CHANGELOG.md)
- [Licence](#licence)

## Installation

From a checkout of the repository:

```sh
cargo install --path tpt-planetary-cli
```

Or build it in place:

```sh
cargo build -p tpt-planetary-cli
./target/debug/tptplan --help
```

Once published, `cargo install tpt-planetary-cli` installs `tptplan` from
crates.io. The minimum supported Rust version is 1.95.

## Global options

| Option | Effect |
| --- | --- |
| `-v`, `--verbose` | Diagnostic output on stderr. Repeat for more: `-vv`. |
| `--format table\|json` | Output format for commands that print data. The default is `table`. |
| `-h`, `--help` | Help for `tptplan` or any subcommand. |
| `-V`, `--version` | The version of the binary. |

## Commands

### `tptplan array inspect <LOCATION>`

Shows the shape, dtype, chunk shape and attributes of a Zarr v2 or v3 array.

`LOCATION` can be a local directory, an `http://` URL, or an `s3://bucket/key`
URL. `s3://` needs `TPT_S3_ENDPOINT` (see [Environment variables](#environment-variables)).
`https://` and other schemes exit with 3, because no TLS is built in. A path
without Zarr metadata exits with 1.

```sh
tptplan array inspect run1/tas.zarr
tptplan --format json array inspect http://127.0.0.1:8000/run1/tas.zarr
```

### `tptplan array math <EXPRESSION> --input NAME=PATH ... --output PATH`

Evaluates an arithmetic expression over named Zarr arrays. The arrays are
combined lazily and written chunk by chunk to a Zarr output, so the result is
not held in memory as a whole. The output is a float64 Zarr array
(`--zarr-format 2|3`, default 2). It must not exist yet.

Grammar:

```text
expression := term (('+' | '-') term)*
term       := unary (('*' | '/') unary)*
unary      := '-' unary | primary
primary    := NUMBER | NAME | '(' expression ')'
```

- `NUMBER` is a number with an optional fraction and exponent, such as `2`,
  `0.5` or `1e-3`.
- `NAME` is a letter or `_`, then letters, digits or `_`. Each name needs an
  `--input NAME=PATH` argument pointing at a local Zarr array directory.
- Unary minus binds tighter than `*` and `/`, so `-a * b` is `(-a) * b`.
- Operands broadcast with NumPy rules. Numeric constants broadcast to the shape
  of the other operand.
- Division follows IEEE rules. Fill-valued and NaN inputs give NaN in the output.

```sh
tptplan array math "(a - b) / 2 + 1" \
    --input a=run1/tas --input b=run2/tas \
    --output diff.zarr
```

An output path ending in `.nc` writes a NetCDF-3 file instead, with one float64
variable named `result` and dimensions `dim0`, `dim1` and so on.

A syntax error exits with 2. A missing or unreadable input exits with 1.

### `tptplan array index <INDEX> --input PATH --lat-start DEG --lat-step DEG --lon-start DEG --lon-step DEG`

Computes a climate index of a local Zarr field of shape `[time, lat, lon]` with
monthly time steps. `INDEX` is `nino34` (the monthly SST anomaly averaged over
5°S–5°N, 190°E–240°E) or `nino34-relative` (that minus the 20°S–20°N mean). The
anomaly is taken against the calendar-month climatology of the record. The grid
is regular and ascending: give the first latitude and longitude and positive
steps. Output is a table or, with `--format json`, a JSON object with one value
per month.

```sh
tptplan --format json array index nino34 --input sst --lat-start -87.5 --lat-step 2.5 --lon-start 0 --lon-step 2.5
```

### `tptplan array convert <INPUT> <OUTPUT>`

Converts between a NetCDF-3 file and a Zarr directory. The direction comes from
the input: a `.nc` file is read as NetCDF-3, and a Zarr directory is read as Zarr.
The output format comes from the output path: a `.nc` path writes NetCDF-3, and
anything else writes Zarr. The output must not exist yet.

- A NetCDF-3 file becomes a Zarr group, with one array per variable.
- A Zarr array becomes a NetCDF-3 variable named after the directory, without
  the `.zarr` suffix. Dimension names are kept. Unnamed axes become
  `dim{axis}_{length}`.
- The `_FillValue` attribute and other text or numeric attributes are kept.
- Supported types are int8, int16, int32, float32 and float64. Unsigned and
  64-bit integers, booleans and text are rejected with exit 1.
- Time variables stay as numbers. CF time decoding is not applied by this command.
  The library does decode CF time units; see the
  [array crate](../tpt-planetary-array/README.md).
- NetCDF-4 (HDF5) files are not supported.

```sh
tptplan array convert run1/tas.zarr tas.nc
tptplan array convert tas.nc tas_zarr
```

### `tptplan astro convert <VALUE> --from <SCALE> --to <SCALE>`

Converts a timestamp between the time scales `utc`, `tai`, `tt` and `tdb`, or converts
coordinates between the frames `icrs`, `galactic` and `ecliptic` (mean J2000).

Timestamps are ISO 8601 in the form `YYYY-MM-DDTHH:MM:SS[.fraction]`, with no
time-zone designator. Other scales, such as `tcb`, and other frames exit with 3.
The ecliptic frames `ecliptic-date` and `true-ecliptic-date` take `--epoch`, a TT
timestamp.

```sh
tptplan astro convert 2025-06-15T12:00:00 --from utc --to tt
```

```text
2025-06-15T12:01:09.184 TT
jd1 2460842
jd2 0.0008007407407407707
```

```sh
tptplan --format json astro convert 2025-06-15T12:00:00 --from utc --to tai
```

```text
{"scale": "TAI", "timestamp": "2025-06-15T12:00:37", "jd1": 2460842, "jd2": 0.0004282407407407707}
```

On 2025-06-15 the UTC to TT offset is 69.184 s: 37 s of TAI minus UTC, plus
32.184 s. The leap-second table has an expiry date, after which UTC conversions
fail. See [Limitations](#limitations).

An invalid timestamp exits with 2:

```sh
tptplan astro convert not-a-time --from utc --to tt
```

```text
tptplan: invalid timestamp 'not-a-time': expected YYYY-MM-DDTHH:MM:SS[.fraction] with no time-zone designator
```

### `tptplan ephemeris query <BODY> <EPOCH> --kernel <PATH>`

Queries the position of a body at an epoch, interpreted as TDB, from an SPK
kernel.

- `BODY` is a NAIF ID, or one of `sun` (10), `earth` (399), `moon` (301), `mars`
  (barycentre 4), `jupiter` (barycentre 5) or `saturn` (barycentre 6). No other
  names are resolved.
- `--observer` defaults to `earth`, using the same names or NAIF IDs.
- The output frame is ICRS only.

```sh
tptplan ephemeris query jupiter 2017-01-01T00:00:00 --kernel de440s.bsp
```

No kernel is bundled and none is downloaded. Without `--kernel`, the command
exits with 3. A kernel path that cannot be read exits with 1. An unknown body
name exits with 2.

`--apparent` adds the Sun's light deflection and the annual aberration (the
apparent direction, as Horizons gives it, in ICRF axes). `--astrometric` prints the astrometric right ascension and declination instead:
the light-time corrected direction from the observer, without stellar
aberration, as JPL Horizons gives it. With DE440s, Jupiter and Saturn agree with
Horizons to 0.015 arcsec at the epochs checked (see
`bindings/tpt-planetary-py/tests/validation/test_ephemeris_horizons.py`).

```sh
tptplan ephemeris query jupiter 2017-01-01T00:00:00 --kernel de440s.bsp --astrometric
```

### `tptplan completions <SHELL>`

Writes a shell completion script to standard output, for `bash`, `zsh`, `fish`,
`powershell` or `elvish`.

```sh
tptplan completions bash > tptplan.bash
```

### `tptplan man [DIR]`

Writes the man pages (`*.1`) into `DIR`, which defaults to the current directory.

```sh
tptplan man ./man
```

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success. |
| 1 | Runtime error: I/O, invalid data, or a failed computation. |
| 2 | Usage error: bad arguments, an invalid value, or a syntax error. clap also uses 2 for its own argument errors. |
| 3 | Not implemented in this build. The message names what is missing. |

## Environment variables

| Variable | Used by | Meaning |
| --- | --- | --- |
| `TPT_S3_ENDPOINT` | `array inspect` with `s3://` | The S3 endpoint over plain HTTP, for example `http://127.0.0.1:9000`. Required for `s3://` locations. |
| `AWS_*` | `array inspect` with `s3://` | Credentials, read when set. |
| `AWS_REGION`, `AWS_DEFAULT_REGION` | `array inspect` with `s3://` | The signing region. Defaults to `us-east-1`. |

## Limitations

- **No TLS.** `https://` locations exit with 3. Use an `http://` endpoint or a
  local path.
- **Partial conversions.** `array convert` supports the five types above only.
  `array math` writes float64 only.
- **Time scales and frames.** Only `utc`, `tai`, `tt`, `icrs`, `galactic` and
  `ecliptic` (mean J2000) are converted. Others exit with 3.
- **Ephemerides.** Kernels are not bundled. Queries are not validated against JPL
  Horizons. The output is ICRS only.
- **Leap seconds.** The embedded table is valid until 2027-06-28. UTC conversions
  after that date fail until the table is updated.

## Development

```sh
cargo test -p tpt-planetary-cli                         # all CLI tests
cargo test -p tpt-planetary-cli --test cli              # exit codes and output
cargo test -p tpt-planetary-cli --test dataflow         # end-to-end pipeline
```

Tests in `tests/`:

- `cli.rs`: the binary's output and exit codes.
- `array_math.rs`, `array_convert.rs`: the array commands on synthetic data.
- `remote_inspect.rs`: `array inspect` over `http://` and `s3://`, against a
  local test server.
- `dataflow.rs`: a scaled-down version of the specification's section 7 pipeline,
  tested through the libraries rather than the binary.

## Licence

Dual-licensed under either of

- MIT licence ([LICENSE-MIT](https://github.com/tpt-solutions/tpt-planetary/blob/master/LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](https://github.com/tpt-solutions/tpt-planetary/blob/master/LICENSE-APACHE))

at your option. Unless you state otherwise, any contribution you submit is
dual-licensed in the same way.
