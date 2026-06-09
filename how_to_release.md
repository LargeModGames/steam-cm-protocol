# To create a release

Releases are automated via GitHub Actions using [`.github/workflows/cd.yml`](.github/workflows/cd.yml).

The workflow runs when you push a tag.

## Stable release

1. Bump the version in `Cargo.toml`.
2. Update `CHANGELOG.md`: move relevant entries from `Unreleased` under a new version header.
3. Run the local release checks:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --locked --all-targets -- -D warnings
   cargo test --locked
   cargo package --locked
   ```
4. Commit and push your changes.
5. Create an annotated tag with a message:
   ```bash
   git tag -a v0.2.0 -m "Release v0.2.0"
   ```
6. Push that specific tag:
   ```bash
   git push origin v0.2.0
   ```
7. Wait for the build on the GitHub Actions page.

Stable tags publish to crates.io using `CARGO_REGISTRY_TOKEN`.

## Pre-release

Use a SemVer pre-release tag like `v0.3.0-rc1` or `v0.3.0-beta.1`.

Pre-release tags create a GitHub pre-release and skip crates.io publishing.

## Steamie dependency update

`steamie` currently consumes this repo through local path dependencies. After a stable `steam-cm-protocol` release is published, update `steamie` to the released version when you want normal dependency resolution:

```toml
steam-cm-protocol = "0.2"
```

Keep the path dependency locally while coordinating unreleased protocol changes across both repos.
