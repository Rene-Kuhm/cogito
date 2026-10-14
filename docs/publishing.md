# Publishing Guide

This document describes how to publish `cogito` to crates.io, npm, and Homebrew.
**Nothing publishes automatically.** Publishing is a deliberate human decision that
requires credentials and explicit workflow dispatch.

## Required Secrets

Before any publish job can run, set the corresponding secret in the repository
settings (`Settings` → `Secrets and variables` → `Actions`):

| Secret | Purpose | Where to get it |
|---|---|---|
| `CARGO_REGISTRY_TOKEN` | Publish to crates.io | [crates.io account settings](https://crates.io/settings/tokens) |
| `NPM_TOKEN` | Publish to npm | [npm account settings](https://www.npmjs.com/settings/TOKEN_NAME/access) |
| `HOMEBREW_TAP_TOKEN` | Push to Homebrew tap | GitHub Personal Access Token with `repo` scope |
| `HOMEBREW_TAP_REPO` | Tap repository full name (e.g. `user/homebrew-tap`) | Set as a repository variable, not a secret |

## Publish Order

`cogito-harness` depends on `cogito-harness-core`, so **core must be published
first**. If `cogito-harness-core` is not on crates.io when `cogito-harness` is
uploaded, the publish will fail. The `publish-crates` job handles this order
automatically: it publishes core, then harness.

On first publish, `cargo publish` will fail if the package already exists. The
job checks whether the package is already on crates.io and skips it if so.

## Publishing Steps

### 1. Create a GitHub Release

Push a tag to trigger the release workflow:

```bash
git tag v<version>
git push origin v<version>
```

This builds all targets, attaches the binaries and checksums to a **draft**
GitHub Release. Verify the draft release looks correct before proceeding.

### 2. Publish to crates.io

1. Ensure `CARGO_REGISTRY_TOKEN` is set.
2. In the GitHub Actions UI, navigate to the release workflow run.
3. Click **Re-run all jobs**.
4. Expand **Workflow dispatch inputs** and set `publish_crates` to `true`.
5. Click **Run workflow**.

Or trigger via CLI (requires `gh`):

```bash
gh workflow run release.yml -f publish_crates=true
```

### 3. Publish to npm

1. Ensure `NPM_TOKEN` is set.
2. Trigger the release workflow with `publish_npm: true`.
3. Verify the package appears at `https://www.npmjs.com/package/cogito-harness`.

### 4. Update Homebrew Tap

This requires a GitHub repository to serve as the Homebrew tap.

1. Set `HOMEBREW_TAP_REPO` to the full name of your tap repository
   (e.g. `renekuhm/homebrew-tap`).
2. Set `HOMEBREW_TAP_TOKEN` to a GitHub Personal Access Token with `repo` scope.
3. Trigger the release workflow with `publish_homebrew: true`.

The workflow clones the tap, renders the formula using `packaging/homebrew/render.sh`,
and commits it to the `main` branch of the tap repository.

To update manually:

```bash
# Clone your tap
git clone https://github.com/<user>/homebrew-tap.git
cd homebrew-tap

# Render the formula
../cogito/packaging/homebrew/render.sh <version> /path/to/cogito-checksums.txt > Formula/cogito.rb

# Validate syntax
ruby -c Formula/cogito.rb

# Commit and push
git add Formula/cogito.rb
git commit -m "cogito <version>"
git push
```

To install from the tap:

```bash
brew tap <user>/tap
brew install cogito
```

## Verifying Artifacts

### crates.io

```bash
cargo search cogito-harness
cargo search cogito-harness-core
```

### npm

```bash
npm view cogito-harness
```

### Homebrew

```bash
# Validate the formula without installing
brew install --build-from-source --dry-run Formula/cogito.rb

# Or install and test
brew install --HEAD cogito
cogito --version
```

### Checksums

```bash
# Verify a downloaded binary against the checksums file
sha256sum -c cogito-checksums.txt
```

## Version Synchronization

The crate version, npm package version, and GitHub release tag must all agree.
All three are derived from the GitHub ref name (stripping the leading `v`).
Update `version` in `Cargo.toml` before tagging a release.

## Unpublishing

crates.io does not support permanent unpublishing. If a crate version is
released in error, yank it:

```bash
cargo yank --version <version> -p cogito-harness-core
cargo yank --version <version> -p cogito-harness
```

Yanking makes the version uninstallable for new users but does not remove
existing copies.
