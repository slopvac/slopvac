# Distribution

The `slopvac` binary ships through cargo-dist (`dist`). The configuration lives in `dist-workspace.toml`, and `dist generate` writes `.github/workflows/release.yml` from it. Do not edit the workflow by hand. Regenerate it with `scripts/dist_generate.sh`, never with a bare `dist generate`.

## What a release contains

- Archives for five targets:
  - macOS arm64 and x86_64.
  - Linux arm64 and x86_64 (glibc).
  - Linux x86_64 (musl).
- A shell installer and a Homebrew formula pushed to `slopvac/homebrew-tap`.
- GitHub build attestations, checksums, a CycloneDX SBOM, and binaries built with `cargo-auditable`.
- A minisign signature (`<file>.minisig`) for every published file: archives, installers, the Homebrew formula, checksum files, the SBOM and `dist-manifest.json`.

## Windows

3.0 ships no native Windows build. Windows users run the Linux build under WSL. Inside a WSL distribution, any of these installs work:

- the shell installer.
- Homebrew on Linux.
- `uvx slopvac`.

Native Windows is a 3.x feature (epic `slopvac-8l4.18.8`). The release builds no Windows target, PowerShell installer or Windows wheel, and the release gate needs no Windows item.

## How a release flows

Release-please owns the version, the tag, and the release notes. dist owns the artifacts. Neither creates the other's objects:

1. A maintainer merges the release PR. Release-please creates the `vX.Y.Z` tag and an unpublished GitHub release for it (`"draft": true` and `"force-tag-creation": true` in `release-please-config.json`). Without `force-tag-creation`, GitHub creates no tag until a release is published.
2. The tag push starts `release.yml`. The `route` job reads the major version. From major 3 on, the dist pipeline runs; earlier tags take the 2.x path below.
3. The pipeline builds the artifacts and, in parallel, runs the provenance gate and the tag ruleset check (`release-provenance.yml`).
4. The `host` job signs every artifact with minisign and checks each signature. It then uploads the artifacts and signatures to the unpublished release. Right before it publishes, it checks again that the tag still points at the workflow commit. Then it publishes the release (`create-release = false` in `dist-workspace.toml`, so dist edits the existing release instead of creating one).
5. Last, the Homebrew job mints a short-lived token from the release App, pushes the formula to the tap with it, and the smoke test installs the result.

The release-please token matters. A tag that the default `GITHUB_TOKEN` creates does not start `release.yml`, because GitHub runs no workflow for events that token causes. The release-please workflow uses the GitHub App token when `RELEASE_APP_CLIENT_ID` and `RELEASE_APP_PRIVATE_KEY` are set, and falls back to `GITHUB_TOKEN` otherwise. With the fallback, the release stays unpublished: nothing runs to publish it. Configure the App before relying on this flow.

### Releases before 3.0

A tag with major version below 3 does not go through dist. The `publish-legacy-release` job (the only other job with `contents: write`) publishes the release that release-please made, so a 2.x release ends as it did when release-please published it directly. Right before it publishes, the job runs the same tag protection as the dist path: the ruleset check, then `verify-tag` (the remote tag must still equal the workflow commit). It also runs `release_provenance.py check-draft`, which reads the release read-only and fails unless it is unpublished and its author is `github-actions[bot]` or `<slug>[bot]`, where the repository variable `RELEASE_APP_SLUG` holds the release-please App's slug. When release-please runs with the App token, its releases are authored by `<slug>[bot]`, so `RELEASE_APP_SLUG` is required: without it the legacy publish job stops at `check-draft` and the 2.x release stays unpublished. The candidate gate does not apply to 2.x. The 2.x PyPI flow (`just release X.Y.Z`, then `publish-lint.yml`) is unchanged. The `route` job fails on a tag it cannot parse instead of skipping the checks.

## Signatures

Releases carry minisign signatures next to the GitHub build attestations. The release key has the id `D9914BDB4570AC6D` and no password. Its public half is `keys/slopvac-release.pub` in this repository.

Verify a download:

```sh
minisign -Vm slopvac-x86_64-unknown-linux-gnu.tar.gz -p keys/slopvac-release.pub
```

minisign finds `slopvac-x86_64-unknown-linux-gnu.tar.gz.minisig` next to the file. A changed file, or a signature from another key, fails with a non-zero exit. For many files at once, run `python3 scripts/release_signing.py verify <files>`.

Each signature also carries a trusted comment that names the file and the release, for example `slopvac-release file:slopvac-x86_64-unknown-linux-gnu.tar.gz version:3.1.0`. The release version has no `v`, and a candidate keeps its suffix, as in `3.1.0-rc.2`. Minisign prints the comment after it verifies the signature, and the signature covers it. `slopvac self update` requires the comment to match the archive and version it requested, so a signed archive from another release or target does not install. It also installs stable releases only, so a signed candidate does not install either.

When `slopvac self update` refuses or fails, it exits 2, leaves the installed binary unchanged and reports one of these codes:

| Code | When |
| --- | --- |
| `self-update-unmanaged` | The binary was not installed by the slopvac shell installer. The message names the upgrade command for a known install method. Cargo-style installs and unknown ones get the releases page, because slopvac is not published on crates.io. |
| `offline-network-needed` | Offline mode is on. Self update needs the network. |
| `update-check-failed` | The latest-release lookup failed. |
| `update-prerelease-refused` | The latest release is a candidate (`X.Y.Z-rc.N`). Self update installs stable releases only. |
| `update-verification-failed` | The signature does not parse, does not verify against the embedded key, or names another file or version. |
| `update-in-progress` | Another `self update` held the update lock for the whole 60-second wait. Run it again when the other update finishes. |
| `update-lock-unsafe` | The lock file `slopvac.update.lock` beside the binary is a symbolic link, has more than one hard link, belongs to another user, or has a mode looser than `0600`. Nothing is written through it. Remove the file after you check why it is there, then run the update again. |
| `update-interrupted` | An earlier update died part-way, and the installed binary matches neither the build it replaced nor the build it was installing, or the update record is unreadable. The update refuses to install over it. The message first tries the update lock. If no update holds it, the message says to remove `slopvac.update.lock` beside the binary after you check the installed `slopvac`, and to run the update again. If another update holds it, the message says to wait for that update to finish, and never to remove the file. |
| `update-version-unknown` | No update record vouches for the installed binary, the binary is not the one that is running the update, and it did not report a version when run with `--offline --version --json` (it failed, printed something else, or took longer than 10 seconds). The update refuses to install over a binary of unknown version. Check that the installed `slopvac` runs, then run the update again. |
| `self-update-failed` | The download of the archive or its `.minisig` failed (a missing signature lands here), or extraction or replacement failed, or the platform has no self-update support. |

If the latest release is not newer than the running binary, `slopvac self update` reports that it is up to date and exits 0.

Before it replaces the binary, the update takes the lock `slopvac.update.lock` beside it and works out which version is installed there. The running version counts only while the file on disk still has the same SHA-256 as the running binary, so another installer that replaced the file since the process started cannot be overwritten with an older release. The update uses the highest version that the file's fingerprint vouches for: a matching record in the lock file, or the running version. When nothing vouches for the file, the update runs it with `--offline --version --json` and reads the version from the output. It refuses when that fails.

How the workflows sign:

- `scripts/install_minisign.sh` installs a pinned minisign release and checks its sha256 before it runs anything.
- `scripts/release_signing.py sign` writes the secret from `MINISIGN_SECRET_KEY` to a mode 600 file under `RUNNER_TEMP`, signs every file in the artifact directory with the trusted comment for the version in `--version`, deletes the key file, and verifies each signature against `keys/slopvac-release.pub`. Only the signing step sees the secret. The step never prompts, so an encrypted key fails the job.
- Both workflows pass the version from the validated tag through an environment variable. The `host` job signs in `release.yml`, before it uploads to the unpublished release. In `candidate.yml` the `sign` job signs the archives, checksums and wheels before the PyPI upload, and the `prerelease` job attaches the signatures. A published release holds no unsigned asset.
- The step fails closed. Without the public key file or the secret, the job stops before it uploads. `candidate.yml` also checks for the key in `validate`. Candidate wheels on PyPI carry no `.minisig`; the signed copies are on the GitHub prerelease. A stable release has no wheels on GitHub, so its wheels have no minisign signature.

To rotate the key, follow [Rotate the key](maintainer-setup.md#rotate-the-key). Releases already published keep verifying against the key that signed them, so keep the old public key under `keys/` with its key id in the file name, and record the change in the release notes.

## Check a change without publishing

```sh
dist plan --tag v<version>
dist generate --check
dist build --artifacts=local --tag v<version>
```

`dist plan` needs a tag whose version equals the version in `Cargo.toml`. On pull requests the `plan` job in `release.yml` runs the same check (`pr-run-mode = "plan"`), so nothing builds or publishes from a branch.

## Maintainer setup

A maintainer must do these once. No workflow does them.

- Create the `MINISIGN_SECRET_KEY` secret from the key described in [maintainer setup](maintainer-setup.md#release-signing-key) in the `release`, `candidate-signing` and `revocations` environments. No repository-level copy exists, and a test fails when a job reads a `MINISIGN_*` secret without declaring an environment. Release and candidate jobs stop without the secret or the committed public key.
- Create the `slopvac/homebrew-tap` repository and install the release GitHub App on it (Contents: read and write). The Homebrew job mints a short-lived token from the App credentials `RELEASE_APP_CLIENT_ID` and `RELEASE_APP_PRIVATE_KEY`, so no tap token secret exists.
- Configure the release-please GitHub App, as the flow section describes, and set the repository variable `RELEASE_APP_SLUG` to the App's slug. It is required: the 2.x publish job accepts only releases authored by `github-actions[bot]` or `<slug>[bot]`, and stops otherwise.
- Give the release GitHub App the repository permission Administration: Read-only, and accept the new permission on the organization installation. The tag ruleset check mints a token from the App with only that permission to read bypass actors. If minting fails, the release stops with a message that names the permission.
- Create the tag ruleset described next.
- Enable GitHub immutable releases in the repository settings. Once a release is published, its tag and assets then cannot change. The flow above fits this: assets attach before the release is published.

A pushed version tag starts the workflow, so only maintainers tag. The workflow starts only on plain `v<major>.<minor>.<patch>` tags, so it never publishes a prerelease. [Release candidates](candidates.md) run from a separate workflow on `rc/vX.Y.Z-rc.N` tags; this workflow does not handle them.

### Protect version tags

Create a repository ruleset with these settings:

- Target: tags, matching `refs/tags/v*`.
- Enforcement: active. A ruleset in `evaluate` or `disabled` mode does not count.
- Rules: restrict updates, restrict deletions, and block force pushes. In the API these are `update`, `deletion`, and `non_fast_forward`.
- Bypass list: empty, or only the repository admin role. Any other bypass actor (a write role, a team, an app, a user, or a deploy key) lets that actor move the tag, so the check rejects the ruleset whatever the bypass mode.

Without this ruleset, someone who can write tags could move a tag after the checks pass. The `verify-tag` check narrows that window, and the ruleset closes it.

`release-provenance.yml` runs `scripts/release_ruleset.py check` on every `vX.Y.Z` tag of major 3 and later, and the 2.x publish job runs it too. The script is read-only. It lists the tag rulesets (`GET /repos/{owner}/{repo}/rulesets`) and reads each one (`GET /repos/{owner}/{repo}/rulesets/{id}`). It trusts a ruleset that covers the tag only when all of these hold: the ruleset is active, `current_user_can_bypass` is `never` for the reading token, and its ID and `updated_at` match an entry in `scripts/release_ruleset_pins.json`. The active, trusted rulesets that cover the tag must together enforce the update, deletion and non-fast-forward rules. If the ruleset was edited, the check fails until a maintainer reviews it and updates the pin. When GitHub shows `bypass_actors`, the script also requires them to be the repository admin role (role id 5) or organization admins. It fails the release when the ruleset is missing, inactive, too narrow, changed since it was pinned, bypassable, or unreadable.

GitHub matches ref patterns with `fnmatch` under `File::FNM_PATHNAME` and adds the `~ALL` and `~DEFAULT_BRANCH` tokens (see [Creating rulesets for a repository](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/creating-rulesets-for-a-repository)). The check does not reimplement that matching. It counts an include only if it is `~ALL`, `refs/tags/*`, `refs/tags/**`, `refs/tags/**/*`, `refs/tags/v*`, or the exact tag ref, so use one of these. It treats an exclude as excluding the tag unless it provably cannot match.

The ruleset endpoints are documented in the GitHub REST reference for [repository rules](https://docs.github.com/en/rest/repos/rules). GitHub returns `bypass_actors` only to a caller that can write the ruleset, which the release token cannot, so the pin stands in for reading them. `updated_at` changes on every edit of a ruleset, and `current_user_can_bypass` tells the reading token whether it can bypass the ruleset. The workflow mints a token from the release App (`actions/create-github-app-token`), scoped to this repository with `permission-administration: read` and nothing else. That token is `GH_TOKEN` for this one step only. If the first release fails with a 403 or 404 although the ruleset exists, the check failed closed as designed; confirm that the App has Administration: Read-only on the installation.

To test the token and the check without a release, a maintainer dispatches `release-provenance.yml` with a `tag` input (for example `gh workflow run release-provenance.yml -f tag=v2.9.1`). That run mints the token and runs only the ruleset check against the tag. It skips the candidate checks and publishes nothing. The workflow must exist on the default branch before it can be dispatched.

## Tag safety

A git tag name can hold `$(...)` or backticks, and the generated workflow pastes the tag into shell commands. `scripts/patch_release_workflow.py` rewrites the generated file after `dist generate`:

- Only tags that match `v<digits>.<digits>.<digits>` start the workflow.
- The first step of the plan job rejects any tag that is not exactly `vX.Y.Z`, before dist runs.
- The tag reaches every shell step through an environment variable, never through `${{ }}` inside the script.
- The `custom-release-provenance` plan job runs `release-provenance.yml` and passes it only the `RELEASE_APP_PRIVATE_KEY` secret, never `secrets: inherit`. The `host` job needs it, so a failed gate cannot let a release through when dist skips the build jobs. The gate runs on every `vX.Y.Z` tag of major 3 and later. It can pass only when a passed [candidate](candidates.md) run exists.
- The gate binds to the commit. `release_provenance.py check-stable` takes the commit the workflow runs on (`$GITHUB_SHA`) and fails when the tag points anywhere else. It reads the candidate runs page by page, up to 1000 runs, and fails closed beyond that.
- The host job re-resolves the tag on the remote (`release_provenance.py verify-tag`, peeled for annotated tags) before dist runs and again right before it publishes the release. It fails unless the tag still equals `$GITHUB_SHA`. The tag and commit reach the script through environment variables only.
- The build step runs under bash, and the dist flags reach it through environment variables.
- The CycloneDX step output is read as `outputs.paths`, so the SBOM reaches the upload. dist 0.34.0 writes `output`, which is a typo.
- The stable PyPI job in `publish-lint.yml` uses the same gate for versions 3.0.0 and later only. The `build` job decides this with `release_provenance.py requires-gate`. A 2.x release keeps its flow without the gate, because 2.x candidates are untagged branch builds that no candidate tag can prove.
- The workflow token is read-only. Only the host job and the 2.x publish job can write contents.

`dist-workspace.toml` sets `allow-dirty = ["ci"]` so dist accepts the hardened file. The script fails if a dist upgrade changes the template, and `tests/test_patch_release_workflow.py` hardens dist's raw output, `tests/fixtures/release.generated.yml`, and requires the result to equal the committed workflow byte for byte. After changing `dist-workspace.toml` or upgrading dist, run `scripts/dist_generate.sh --save-raw tests/fixtures/release.generated.yml` and commit both files.

### Pinned jobs

`scripts/release_workflow_pins.json` holds a sha256 for every job in `release.yml` and for each workflow-level block (`name`, `on` and `permissions`). Generation fails when a job changes, appears, disappears or gets a new name. It also fails when one of those blocks changes. The error names the job or block. The pins exist because no pattern list can recognize every way a shell step hides a command, so every change to the workflow needs a person to read it.

When a pin fails:

1. Run `git diff .github/workflows/release.yml`. The generator writes the hardened file before it checks the pins, so the diff shows the change.
2. Read the diff for each job the error names. Treat it as you would any change to a release workflow.
3. Run `scripts/dist_generate.sh --update-pins`. This rewrites the pin file. No other command writes it.
4. Commit the workflow and the pin file together. After a dist upgrade, add the raw fixture.

## Publish order and recovery

Each release of major 3 and later publishes in this order: build, upload to the unpublished release, provenance and tag checks, publish with attestations, then the Homebrew formula. The formula points at the release assets, so it must come last, and a push to the tap cannot be undone by the workflow. If the formula job fails, rerun that job. If a published release is bad, delete the GitHub release and the tag, then revert the commit in `slopvac/homebrew-tap` by hand. With immutable releases enabled, GitHub keeps a deleted release's tag name reserved, so the fix ships as a new version.

## Test an installed release

`.github/workflows/release-smoke.yml` installs a published release and runs `slopvac --version` for each route, then compares the output with the tag:

- the archive for each of the five targets, after checking the published `.sha256` file.
- the shell installer on Linux and macOS.
- `brew install slopvac/tap/slopvac` on Linux and macOS. It runs only when the tag is the repository's latest stable release, because the tap serves only that one; for an older tag the workflow logs why it skipped. It retries for about ten minutes, because the tap updates after the release.

It starts after the `Release` workflow succeeds on a tag push. The workflow has no `release: published` trigger: the host job publishes the release with `GITHUB_TOKEN`, which raises no event, and such a trigger could start before the assets exist. To test any release again, run the workflow from the Actions tab and give a `tag` such as `v3.0.0`. The tag must be `vX.Y.Z` or `vX.Y.Z-rc.N`; `scripts/release_smoke.py` rejects anything else, and the tag reaches shell steps only through the environment. Tags of major 2 and earlier are skipped, because dist did not build them.

The workflow needs a published release, so it has not run yet. The first real release is its first test.
