# Release candidates

A release candidate is a build of the exact tree that a stable release will ship. Cut one on demand, let it pass, then promote it.

## Cut a candidate

Tag the release PR head, or `main`, with `rc/vX.Y.Z-rc.N` and push the tag. N starts at 1.

```sh
git tag rc/v3.0.0-rc.1 <commit>
git push origin rc/v3.0.0-rc.1
```

The `Release candidate` workflow (`.github/workflows/candidate.yml`) then:

1. Validates the tag and checks that the commit is on `main` or a release-please branch.
2. Sets the build version to `X.Y.Z-rc.N` for the CI checkout only. No commit carries it.
3. Runs the offline accuracy gate (`accuracy::` tests) and the migration end-to-end tests (`migration_e2e::`). Both use `--no-tests=fail`, so a missing suite blocks the candidate.
4. Builds wheels for the supported platforms through `wheels.yml`.
5. Waits for approval in the `pypi` environment, then publishes `X.Y.ZrcN` to PyPI.

The Cargo workspace version, the wheel version and the PyPI version come from one tag. `uvx slopvac==3.0.0rc1` installs the candidate. A plain `uvx slopvac` keeps resolving to the latest stable release, because pip and uv skip prereleases by default.

## Check without publishing

Run the workflow from the Actions tab with `workflow_dispatch`. Pick the branch, give a tag name for the version, and leave `dry-run` on. It builds and tests and never publishes.

## Promote to stable

A stable `vX.Y.Z` tag starts the cargo-dist release workflow. Its provenance gate (`release-provenance.yml`, backed by `scripts/release_provenance.py`) requires all of these:

- An `rc/vX.Y.Z-rc.N` tag exists for the same version.
- That candidate's tree equals the stable tag's tree. A squash merge of the tested release PR keeps the tree.
- A `Release candidate` run on the candidate commit finished with success.

Otherwise the stable release stops before it builds. Prerelease tags skip the gate.

## Maintainer setup

- Register `candidate.yml` and the `pypi` environment as a PyPI Trusted Publisher for `slopvac`.
- Keep required reviewers on the `pypi` environment.
- Wire `release-provenance.yml` into cargo-dist with `plan-jobs = ["./release-provenance"]` in `dist-workspace.toml`, then run `dist generate`.
