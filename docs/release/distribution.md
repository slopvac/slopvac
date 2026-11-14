# Distribution

The `slopvac` binary ships through cargo-dist (`dist`). The configuration lives in `dist-workspace.toml`, and `dist generate` writes `.github/workflows/release.yml` from it. Do not edit the workflow by hand. Regenerate it with `scripts/dist_generate.sh`, never with a bare `dist generate`.

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

## Tag safety

A git tag name can hold `$(...)` or backticks, and the generated workflow pastes the tag into shell commands. `scripts/patch_release_workflow.py` rewrites the generated file after `dist generate`:

- Only tags that match `v<digits>.<digits>.<digits>` start the workflow. Candidate tags (`rc/vX.Y.Z-rc.N`) never start it, so a candidate cannot publish through this workflow.
- The first step of the plan job rejects any tag that is not exactly `vX.Y.Z`, before dist runs.
- The tag reaches every shell step through an environment variable, never through `${{ }}` inside the script.
- The workflow token is read-only. Only the host job, which creates the GitHub release, can write contents.

`dist-workspace.toml` sets `allow-dirty = ["ci"]` so dist accepts the hardened file. The script fails if a dist upgrade changes the template, and `tests/test_patch_release_workflow.py` checks the committed workflow. After changing `dist-workspace.toml` or upgrading dist, run `scripts/dist_generate.sh` and commit the result.

## Publish order and recovery

Each stable release publishes in this order: build, GitHub release with attestations, then the Homebrew formula. The formula points at the release assets, so it must come last, and a push to the tap cannot be undone by the workflow. If the formula job fails, rerun that job. If a published release is bad, delete the GitHub release and the tag, then revert the commit in `srobroek/homebrew-tap` by hand.
