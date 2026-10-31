# Releasing

Releases are cut by pushing a version tag on `main`. The `Release` workflow then:

1. Checks the tag is on `main` and matches the version in `Cargo.toml`
2. Builds binaries for Linux (x86_64 gnu/musl, aarch64), macOS (x86_64, Apple Silicon) and Windows
3. Creates a GitHub release with the archives attached
4. Publishes the crate to crates.io (only after every build has succeeded)

## One-time setup

Create a crates.io API token with the `publish-new` and `publish-update` scopes at
<https://crates.io/settings/tokens> and add it to the repository as the
`CARGO_REGISTRY_TOKEN` secret:

```bash
gh secret set CARGO_REGISTRY_TOKEN
```

Optionally add `CODECOV_TOKEN` for coverage uploads.

## Creating a release

1. Update `version` in `Cargo.toml` and run `cargo check` so `Cargo.lock` picks it up
2. Commit and push to `main`, and wait for CI to pass
3. Tag and push:

```bash
git tag v0.1.0
git push origin v0.1.0
```

A crates.io version can't be overwritten, only yanked, so if the workflow fails
after publishing, bump the version and tag again.

## Toolchain

The Rust version is pinned in `rust-toolchain.toml`. The workflows install the
same version through their `RUST_VERSION` variable, so update all three
(and `rust-version` in `Cargo.toml`) together.

## Manual build

```bash
cargo build --release
```

Binary will be at `target/release/git-whoami`.
