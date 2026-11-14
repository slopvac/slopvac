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

The workflow starts only on plain `v<major>.<minor>.<patch>` tags, so it never publishes a prerelease. Release candidates come from the separate candidate workflow (`rc/vX.Y.Z-rc.N` tags), which is not part of this workflow.

### Protect version tags

Create a repository ruleset that targets `v*` tags and blocks updates, deletions, and non-fast-forward pushes. This is a repository setting, and a maintainer must apply it: no workflow can. Without it, someone who can write tags could move a tag after the checks pass. The workflow narrows that window, but a ruleset closes it.

## Tag safety

A git tag name can hold `$(...)` or backticks, and the generated workflow pastes the tag into shell commands. `scripts/patch_release_workflow.py` rewrites the generated file after `dist generate`:

- Only tags that match `v<digits>.<digits>.<digits>` start the workflow. Candidate tags (`rc/vX.Y.Z-rc.N`) never start it, so a candidate cannot publish through this workflow.
- The first step of the plan job rejects any tag that is not exactly `vX.Y.Z`, before dist runs.
- The tag reaches every shell step through an environment variable, never through `${{ }}` inside the script.
- The `custom-release-provenance` plan job runs `release-provenance.yml` without inherited secrets. The `host` job needs it, so a failed gate cannot let a release through when dist skips the build jobs. The gate runs on every `vX.Y.Z` tag. It can pass only once the candidate workflow (`rc/vX.Y.Z-rc.N` tags, a separate change) exists, because it looks for a passed candidate run.
- The gate binds to the commit. `release_provenance.py check-stable` takes the commit the workflow runs on (`$GITHUB_SHA`) and fails when the tag points anywhere else. It reads the candidate runs page by page, up to 1000 runs, and fails closed beyond that.
- The host job re-resolves the tag on the remote (`release_provenance.py verify-tag`, peeled for annotated tags) before dist runs and again right before `gh release create`. It fails unless the tag still equals `$GITHUB_SHA`. The tag and commit reach the script through environment variables only.
- The Windows build step runs under bash, so the `$TAG_FLAG` environment variable expands. Under PowerShell it expanded to nothing.
- The CycloneDX step output is read as `outputs.paths`, so the SBOM reaches the upload. dist 0.34.0 writes `output`, which is a typo.
- The stable PyPI job in `publish-lint.yml` uses the same gate for versions 3.0.0 and later only. The `build` job decides this with `release_provenance.py requires-gate`. A 2.x release keeps its flow without the gate, because 2.x candidates are untagged branch builds that no candidate tag can prove.
- The workflow token is read-only. Only the host job, which creates the GitHub release, can write contents.

`dist-workspace.toml` sets `allow-dirty = ["ci"]` so dist accepts the hardened file. The script fails if a dist upgrade changes the template, and `tests/test_patch_release_workflow.py` hardens dist's raw output, `tests/fixtures/release.generated.yml`, and requires the result to equal the committed workflow byte for byte. After changing `dist-workspace.toml` or upgrading dist, run `scripts/dist_generate.sh --save-raw tests/fixtures/release.generated.yml` and commit both files.

## Publish order and recovery

Each stable release publishes in this order: build, GitHub release with attestations, then the Homebrew formula. The formula points at the release assets, so it must come last, and a push to the tap cannot be undone by the workflow. If the formula job fails, rerun that job. If a published release is bad, delete the GitHub release and the tag, then revert the commit in `srobroek/homebrew-tap` by hand.

## Test an installed release

`.github/workflows/release-smoke.yml` installs a published release and runs `slopvac --version` for each route, then compares the output with the tag:

- the archive for each of the six targets, after checking the published `.sha256` file;
- the shell installer on Linux and macOS, and the PowerShell installer on Windows;
- `brew install srobroek/tap/slopvac` on Linux and macOS, for stable tags only. It retries for about ten minutes, because the tap updates after the release.

It starts after the `Release` workflow succeeds on a tag, because a release that `GITHUB_TOKEN` creates does not raise a `release` event. It also starts when a maintainer publishes a release by hand. To test any release again, run it from the Actions tab and give a `tag` such as `v3.0.0`. The tag must be `vX.Y.Z` or `vX.Y.Z-rc.N`; `scripts/release_smoke.py` rejects anything else, and the tag reaches shell steps only through the environment.

The workflow needs a published release, so it has not run yet. The first real release is its first test.
