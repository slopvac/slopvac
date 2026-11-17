# Release candidates

A release candidate is a build of the tree that a stable release ships. Cut one on demand, let it pass, then promote it. For the stable release workflow, see [distribution](distribution.md).

## Cut a candidate

Tag the release PR head, or `main`, with `rc/vX.Y.Z-rc.N` and push the tag. N starts at 1 and has no leading zero.

```sh
git tag rc/v3.0.0-rc.1 <commit>
git push origin rc/v3.0.0-rc.1
```

The `Publish to PyPI` workflow (`.github/workflows/publish-pypi.yml`) then runs these jobs. The same workflow publishes stable releases, as [Promote to stable](#promote-to-stable) describes.

1. `validate` parses the tag and derives the versions: Cargo `X.Y.Z-rc.N` and PyPI `X.Y.ZrcN`. The tag reaches every script through an environment variable. For a publishing run it also requires the commit to be on `main` or a `release-please--` branch, and requires `keys/slopvac-release.pub`.
2. `evals` sets the Cargo version for that CI checkout only, so no commit carries it. It then runs the offline accuracy gate (`accuracy::` tests) and the migration end-to-end tests (`migration_e2e::`). Both use `--no-tests=fail`, so a suite that does not exist blocks the candidate.
3. `wheels` calls `wheels.yml` for the maturin wheels, and `archives` builds the cargo-dist archives and checksums for the five release targets. The SBOM is a global cargo-dist artifact, so candidates leave it out.
4. `sign` signs the archives, checksums and wheels with minisign and checks each signature, as [distribution](distribution.md#signatures) describes. It runs before the PyPI upload, which cannot be undone, so a signing failure stops the candidate first.
5. `publish` waits for approval in the `pypi` environment, then uploads `X.Y.ZrcN` to PyPI by Trusted Publishing. It runs only after `evals`, `wheels`, `archives` and `sign` pass.
6. `prerelease` attests the build provenance of the archives and wheels. It creates an unpublished GitHub prerelease named after the tag, attaches the archives, checksums, wheels and `.minisig` files, and publishes it. With immutable releases on, a published release takes no new assets, so the assets go on first. Candidates get no Homebrew formula and no installers.

The Cargo workspace version, the wheel version and the PyPI version come from the one tag. cargo-dist reads `X.Y.Z-rc.N` as a prerelease, so `dist plan` marks the announcement as a prerelease.

## Install a candidate

Candidates are opt-in. Plain installs and `slopvac@latest` stay on the latest stable release, because pip and uv skip prereleases by default.

```sh
uvx --from 'slopvac==3.0.0rc1' slopvac --version
uv tool install --prerelease allow slopvac
pip install --pre slopvac
```

The first command pins one candidate. The other two take the newest release, candidates included. The GitHub prerelease also carries the archives for manual download, each with a `.minisig` file that [verifies](distribution.md#signatures) the same way as a stable release. The `slopvac` binary does not check for candidates on its own.

## Check without publishing

Run the workflow from the Actions tab with `workflow_dispatch`. Pick the branch, give a tag name that sets the version, and leave `dry-run` on. The tag need not exist. The run builds and tests, and never publishes. The stable gate accepts push runs only, so a dispatch run is not a candidate.

To publish from a dispatch, turn `dry-run` off and start the run on the candidate tag itself, with the same tag as the input. Any other ref fails in `validate`. A pushed tag is the usual path.

## Promote to stable

Release-please makes the unpublished release for the stable `vX.Y.Z` tag, as [distribution](distribution.md) describes. A candidate prerelease uses a different tag and never touches that release.

The stable tag starts two workflows: the cargo-dist release workflow, which makes the GitHub release, and `publish-pypi.yml`, which publishes the wheels as `X.Y.Z` after approval in the `pypi` environment. Each calls the provenance gate (`release-provenance.yml`, backed by `scripts/release_provenance.py`), which requires all of these:

- An `rc/vX.Y.Z-rc.N` tag exists for the same version.
- That candidate's tree equals the stable tag's tree. A squash merge of the tested release PR keeps the tree.
- A run of `publish-pypi.yml` for the push of that exact tag, at that commit, finished with success. Runs of other workflows, other events, other refs or other commits do not count.

Otherwise the stable release stops before it builds or uploads. In `publish-pypi.yml`, `provenance` is a separate job that the upload job needs, and the upload job checks again that the tag still points at the built commit. The wheels come from the tagged commit, the one the gate checked. Stable runs build no archives and sign nothing, because the cargo-dist workflow owns those. The upload does not skip an existing version, so a repeated run fails. The cargo-dist workflow does not start on candidate tags, so a candidate cannot publish through it. A stable tag below major 3 does nothing in `publish-pypi.yml`.

`tests/test_publish_pypi_workflow.py` checks that the workflow file, its trigger and its tag filter match what the gate looks up.

## Maintainer setup

The prerequisites are the PyPI Trusted Publisher for `publish-pypi.yml`, the `pypi` environment rules and an `rc/v*` tag ruleset. See [maintainer setup](maintainer-setup.md).
