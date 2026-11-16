# Vendored: target-lexicon (MIT substitute)

This directory is a copy of the MIT-licensed, API-compatible replacement for
`target-lexicon` 0.13.6, taken from the local repository
`target-lexicon-mit` at commit `297f442` ("Initial MIT-licensed
target-lexicon replacement").

It replaces the upstream crate, which is licensed Apache-2.0 WITH
LLVM-exception. The workspace uses it through `[patch.crates-io]` in the root
`Cargo.toml`. It is a build-time dependency of `pyo3-build-config` only.

Local changes: none. Update by re-copying from the source commit, and record
the new commit here.

Formatting only: rustfmt was applied to src/lib.rs so that `cargo fmt --check` passes. No code changed.
