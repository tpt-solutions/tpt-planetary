# target-lexicon-mit

An MIT-licensed replacement for the `target-lexicon` crate, providing the subset of
its API that PyO3's build configuration (`pyo3-build-config`) uses: parsing and
formatting target triples and reporting the host triple.

The upstream crate is licensed `Apache-2.0 WITH LLVM-exception`. This crate is
written from scratch against the public behaviour of target triples and is
licensed `MIT` only.

## Use as a drop-in replacement

Keep the normal `target-lexicon = "0.13"` requirement and patch it in the
consuming workspace's root `Cargo.toml`:

```toml
[patch.crates-io]
target-lexicon = { git = "https://github.com/tpt-solutions/target-lexicon-mit", rev = "<commit>" }
```

For a local checkout, use `{ path = "../target-lexicon-mit" }` instead.

## Scope

- `Triple` with `architecture`, `vendor`, `operating_system`, `environment` and
  `binary_format`, plus `FromStr`, `Display` and `Triple::host()`.
- `Architecture`, `Vendor`, `OperatingSystem`, `Environment` and `BinaryFormat`
  enums. Unrecognised components are kept as `Other(String)` instead of being
  rejected, so unusual triples still round-trip.
- The `triple!` macro, for constants and tests.

It is not a full port of the upstream crate. Variants that are not listed above
are not provided. Add them as `Other` values or new variants when a consumer needs
them.

## Licence

MIT. See [LICENSE-MIT](LICENSE-MIT).
