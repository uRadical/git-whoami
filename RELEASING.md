# Releasing

Releases are cut by pushing a version tag on `main`. The `Release` workflow then:

1. Checks the tag is on `main` and matches the version in `Cargo.toml`
2. Builds binaries for Linux (x86_64 gnu/musl, aarch64), macOS (x86_64, Apple Silicon) and Windows
3. Creates a GitHub release with the archives attached
4. Publishes the crate to crates.io (only after every build has succeeded)

## One-time setup

The first release must be published with an API token, because crates.io only
allows trusted publishing for a crate that already exists.

1. Create a crates.io API token at <https://crates.io/settings/tokens> with the
   `publish-new` and `publish-update` scopes (restricted to `git-whoami`), and add
   it to the repository as the `CARGO_REGISTRY_TOKEN` secret:

   ```bash
   gh secret set CARGO_REGISTRY_TOKEN
   ```

2. After the first release is on crates.io, switch to trusted publishing: on
   crates.io go to the crate's **Settings → Trusted Publishing → Add → GitHub** and enter
   - Repository owner: `uRadical`
   - Repository name: `git-whoami`
   - Workflow filename: `release.yml`
   - Environment: leave empty

3. Delete the `CARGO_REGISTRY_TOKEN` secret and revoke the token on crates.io.

The publish job tries trusted publishing first and falls back to the secret, so
the workflow needs no changes for the switch.

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
