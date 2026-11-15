# Semver and release plan

Status: planned. The CI job and the release checklist below are not wired up
yet. This document is the target design so that later work has a fixed
reference.

## Published crates

| Crate | Published | Notes |
| --- | --- | --- |
| `tpt-planetary-core` | yes | `no_std` foundation; `std` and `alloc` features. |
| `tpt-planetary-array` | yes | Zarr, codecs, lazy engine, NetCDF-3 reader and writer. |
| `tpt-planetary-astro` | yes | Time, frames, ephemeris, WCS. Depends on core. |
| `tpt-planetary-cli` | yes | Binary `tptplan`. Published so it can be installed with `cargo install`. |
| `bindings/tpt-planetary-py` | no | `publish = false` in its manifest. Released to PyPI as `tpt-planetary`, not crates.io. |
| `fuzz/` (`tpt-planetary-fuzz`) | no | Outside the workspace (`exclude = ["fuzz"]`). Never published. |

Publish order follows the dependency graph: `core`, then `array` and `astro`
(in either order), then `cli`. Internal dependencies in
`[workspace.dependencies]` carry both `version` and `path`, so a crates.io
publish uses the version and a local build uses the path.

Crate names are not yet reserved (see `todo.md`, Phase 0).

## Pre-1.0 policy (0.x)

All crates are at `0.1.0` and share `version.workspace = true`, so they move
together. Until 1.0.0:

- Cargo treats `0.x` versions as incompatible across minor bumps. A change
  that breaks the public API bumps the minor version: `0.1.x` to `0.2.0`.
- A patch bump (`0.1.0` to `0.1.1`) may only contain fixes, documentation,
  and additive changes that do not change any public signature.
- Items that are new but could still change may be marked `#[doc(hidden)]`
  or documented as unstable. Do not use feature flags as the only marker.
- The MSRV is 1.95 (`rust-version` in `[workspace.package]`). Raising it is a
  minor bump before 1.0 and is listed in the changelog. It is not a patch
  change.
- `tpt-planetary-core` keeps its `no_std` build (`--no-default-features`)
  under the same rule. Removing `no_std` support is a breaking change.
- Dependency upgrades that change a public type (for example a `glam` or
  `serde_json` type in a signature) count as breaking changes.

Breaking changes are recorded in `CHANGELOG.md` under a `### Changed` or
`### Removed` heading, following Keep a Changelog. Deprecations use
`#[deprecated]` for one minor release before removal.

## cargo-semver-checks in CI

[`cargo-semver-checks`](https://github.com/obi1kenobi/cargo-semver-checks)
compares the public API of the working tree with the last published version
on crates.io. It is the gate for the version bump.

Planned job, added to `.github/workflows/ci.yml` when release work starts
(not part of this change):

```yaml
semver:
  name: cargo-semver-checks
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - uses: taiki-e/install-action@v2
      with:
        tool: cargo-semver-checks
    - name: Check public API against the last release
      run: |
        for p in tpt-planetary-core tpt-planetary-array tpt-planetary-astro tpt-planetary-cli; do
          cargo semver-checks check-release --package "$p"
        done
```

Notes on the design:

- `check-release` uses the version on crates.io as the baseline. Until the
  first publish there is no baseline, so the job should skip with a clear
  message. Enable it at the first `0.1.0` publish.
- Run the check on pull requests that touch a published crate. Skip pull
  requests that touch only `bindings/`, `docs/`, or `fuzz/`.
- Pin the tool version once it is adopted. The workspace currently lists
  `cargo-semver-checks` 0.42.0 as a candidate in `docs/dependencies.md`, with
  licence `Apache-2.0 OR MIT`. It is a CI tool, not a crate dependency, so it
  does not enter `Cargo.lock`.
- A failure is a blocker only if the version bump does not cover it. The job
  should fail when the detected breaking change is larger than the bump
  allows: under 0.x, a breaking change needs a minor bump, so a patch bump
  with a breaking change fails.
- Lints that need a rustdoc JSON build use the stable toolchain. The
  `cargo-semver-checks` version determines which rustdoc JSON format is
  needed, so keep the tool and toolchain updates together.

Known limits of the tool:

- It checks the signature-level API. It does not detect behaviour changes such
  as a different rounding in a time conversion or a changed default leap-second
  table. Those are covered by the tests and the release notes.
- It does not check the `no_std` build. The `thumbv7em-none-eabihf` job in CI
  covers that.
- Features that are off by default are checked only when the check runs with
  them enabled. Run the check with default features, and again with
  `--all-features` when a feature gate changes.

## Release checklist

Run from a clean checkout of `master`.

1. Confirm CI is green on all platforms, the MSRV job, the `no_std` job, and
   `cargo deny check`.
2. Run `cargo semver-checks check-release` for each published crate, as in the
   CI job above. Record the bump the tool requires.
3. Choose the version: `0.x.y` with the bump from step 2 and the policy above.
   Update `[workspace.package] version` and the `version` fields of the
   internal path dependencies in `[workspace.dependencies]` together.
4. Move the `## [Unreleased]` entries in `CHANGELOG.md` under the new version
   and add the date. Add the MSRV in use.
5. Run `cargo publish --dry-run -p <crate>` for each crate, in dependency
   order, and check the package contents with `cargo package --list`.
6. Publish in dependency order: `tpt-planetary-core`, then `tpt-planetary-array`
   and `tpt-planetary-astro`, then `tpt-planetary-cli`. Wait for the index to
   update between steps.
7. Tag the commit `vX.Y.Z` and create the GitHub release with the changelog
   section as its body.
8. Build and upload the Python wheels for `bindings/tpt-planetary-py` to PyPI.
   This is a separate step and does not affect the crate versions.

Publishing requires a crates.io token and is done by the maintainer. Nothing
in this document or in the repository performs a publish.

## Before 1.0.0

1.0.0 is a commitment to the public API. Before it:

- Remove or mark unstable every item documented as partial in `todo.md`.
- Run `cargo semver-checks` against the 0.x release at least once with no
  pending breaking changes.
- Freeze the Zarr v3 codec set that the crate claims to support.
- Decide the fate of the `tpt-simd` dependency, which needs Rust 1.95 and
  would change the MSRV if it is pulled in (see `rust-toolchain.toml`).
