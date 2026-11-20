# Third-party notices

The workspace ships `THIRD-PARTY-NOTICES.md` at the repository root. It lists
every third-party crate in the dependency graph with the licence text that the
crate supplies.

## Process

1. Change dependencies only after the licence and FFI audit in
   [dependencies.md](dependencies.md). `cargo deny check` must pass.
2. Regenerate the notices file from the repository root:

   ```sh
   cargo about generate about.hbs -o THIRD-PARTY-NOTICES.md
   ```

   `cargo-about` is installed with `cargo install cargo-about --locked`.
3. Commit the regenerated `THIRD-PARTY-NOTICES.md` with the dependency change.

Do not edit `THIRD-PARTY-NOTICES.md` by hand.

## Configuration

- `about.toml` accepts the same licences as the allow list in `deny.toml`
  (MIT, BSD-2-Clause, BSD-3-Clause, Zlib, Unicode-3.0, ISC). Dual
  `MIT OR Apache-2.0` crates pass on their MIT option. A crate whose licence
  is not accepted makes `cargo about` fail, which is the same policy `cargo deny`
  enforces.
- `about.hbs` is the template. It groups crates by licence text.
- Dev-dependencies are ignored, because they are not distributed in the
  published crates.

## Known gaps

- The workspace's own crates are included in the output. The template does not
  filter them out.
- `target-lexicon` is built from the MIT-licensed substitute in
  `../target-lexicon-mit` (see the licence decision in `todo.md`). Its notice
  refers to that substitute's repository.
- The notices file is generated, not checked in CI. A CI check that regenerates
  it and fails on a diff is not yet written.
