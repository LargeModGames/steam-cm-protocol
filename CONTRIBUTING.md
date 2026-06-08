# Contributing to vapour-protocol

`vapour-protocol` is the standalone Steam client protocol crate used by Vapour.

## Ground rules

- Keep PRs focused and scoped.
- Add or update tests when changing protocol parsing, event decoding, or public API behavior.
- Include captured protocol behavior in the PR notes when it explains an implementation decision.
- Update `README.md` and `CHANGELOG.md` for user-facing or public API changes.

## Local checks

Run the same checks as CI before opening a PR:

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo package --locked
```

## Coordinating with Vapour

Vapour may depend on a local checkout of this repository while protocol work is in progress. For changes that Vapour must consume immediately, keep the protocol PR small and merge/release it before switching Vapour to the new API.
