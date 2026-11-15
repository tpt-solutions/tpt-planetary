# Contributing to tpt-planetary

Contributions are accepted as **issues only**. Please do not open pull requests.

- **Bug reports:** open an issue with the crate name, the version or commit, the
  toolchain (`rustc --version`), and steps to reproduce.
- **Feature requests and design questions:** open an issue describing the use
  case. Check `spec.txt` and `todo.md` first, since many planned features are
  already listed there.
- **New dependencies:** propose them in an issue. Each dependency must pass the
  licence policy in `deny.toml` and the audit rules in `docs/dependencies.md`.
  The policy bans C FFI wrappers.
