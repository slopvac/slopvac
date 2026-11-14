# Distribution

The `slopvac` binary ships through cargo-dist (`dist`). The configuration lives in `dist-workspace.toml`, and `dist generate` writes `.github/workflows/release.yml` from it. Do not edit the workflow by hand: `dist plan` fails when the file drifts.

## What a release contains

- Archives for macOS (arm64, x86_64), Linux (arm64 and x86_64 glibc, x86_64 musl), and Windows (x86_64).
- Shell and PowerShell installers, and a Homebrew formula pushed to `srobroek/homebrew-tap`.
- GitHub build attestations, checksums, a CycloneDX SBOM, and binaries built with `cargo-auditable`.

## Check a change without publishing

```sh
dist plan --tag v<version>
dist generate --check
dist build --artifacts=local --tag v<version>
```

`dist plan` needs a tag whose version equals the version in `Cargo.toml`. On pull requests the `plan` job in `release.yml` runs the same check (`pr-run-mode = "plan"`), so nothing builds or publishes from a branch.

## Maintainer setup

The Homebrew job reads the `HOMEBREW_TAP_TOKEN` repository secret, a token with write access to `srobroek/homebrew-tap`. A maintainer creates the tap repository and the secret. A pushed version tag starts the workflow, so only maintainers tag.

A tag with a prerelease suffix publishes a GitHub prerelease and skips the Homebrew formula.
