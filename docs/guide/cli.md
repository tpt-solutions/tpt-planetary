# Command line (`tptplan`)

`tptplan` is built from `tpt-planetary-cli`:

```sh
cargo build -p tpt-planetary-cli
./target/debug/tptplan --help
```

## Global options

- `-v`, `--verbose` (repeatable, `-vv`): diagnostic output on stderr.
- `--format table|json`: output format for commands that print data. The
  default is `table`.

## Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success. |
| 1 | Runtime error: I/O, invalid data, or a failed computation. |
| 2 | Usage error: bad arguments or an invalid value. clap also uses 2 for its own argument errors. |
| 3 | Not implemented in this build. |

Each not-implemented path prints a message that names what is missing, and
exits with 3. The examples below include those paths so that the codes can be
checked.

## Subcommands

### `tptplan array inspect <URI>`

Shows the shape, dtype, chunk shape and attributes of a Zarr array. Only a
**local** directory is supported. `s3://` and `https://` locations exit with 3.
A path that does not exist exits with 1. A directory without Zarr metadata
exits with 1.

### `tptplan array math <EXPRESSION>`: not implemented

Exits with 3. There is no expression parser in the CLI. The lazy API in
`tpt-planetary-array` is available from Rust.

### `tptplan array convert <INPUT> <OUTPUT>`: not implemented

Exits with 3. Zarr to NetCDF-3 conversion has no CLI wiring yet, although the
NetCDF-3 reader and writer exist in the array crate.

### `tptplan astro convert <VALUE> --from <SCALE> --to <SCALE>`

Converts an ISO 8601 timestamp between the time scales `utc`, `tai` and `tt`.
Timestamps are `YYYY-MM-DDTHH:MM:SS[.fraction]` with no time-zone designator.
Frames, such as `ra,dec`, are not supported and exit with 3.

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

```sh
tptplan astro convert not-a-time --from utc --to tt
```

```text
tptplan: invalid timestamp 'not-a-time': expected YYYY-MM-DDTHH:MM:SS[.fraction] with no time-zone designator
```

That last command exits with 2. The three examples above were run in this
workspace. The UTC-to-TT offset is 69.184 s on 2025-06-15 (37 s of TAI-UTC plus
32.184 s), and the leap-second table expiry applies, see
[Time and frames](time-and-frames.md).

### `tptplan ephemeris query <BODY> <EPOCH> --kernel <PATH>`

Queries the position of a body at an epoch, interpreted as TDB. `BODY` is a
NAIF ID or one of `sun`, `earth`, `moon`, `mars`, `jupiter`, `saturn`. The
observer defaults to `earth`. The output frame is ICRS only.

- Without `--kernel`, the command exits with 3.
- A kernel path that cannot be read exits with 1.
- An unknown body name exits with 2.

No kernel file is bundled and none is downloaded. A successful query has not
been validated against JPL Horizons. See
[Status and limitations](status-and-limitations.md).

### `tptplan completions <SHELL>`

Writes shell completion scripts to standard output.

### `tptplan man [DIR]`

Writes the man pages (`*.1`) into `DIR`, which defaults to the current
directory.

## Tests

`tpt-planetary-cli/tests/cli.rs` runs the binary and checks the exit codes
listed above. `tpt-planetary-cli/tests/dataflow.rs` tests the library
pipeline, not the binary.
