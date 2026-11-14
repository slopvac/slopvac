# Maintainer setup

These steps change repository, PyPI and GitHub settings, so a maintainer does them by hand. No workflow or script in this repository does them. For the release workflows themselves, see [distribution](distribution.md) and [release candidates](candidates.md).

## Homebrew tap

1. Create the public repository `srobroek/homebrew-tap` with a `Formula/` directory (an empty repository with a README is enough).
2. Create a fine-grained personal access token with access to that repository only and **Contents: Read and write**. Give it an expiry and put the renewal date in your calendar.
3. In `srobroek/slopvac`, add the token as the Actions secret `HOMEBREW_TAP_TOKEN` (Settings, Secrets and variables, Actions).

The cargo-dist `publish-homebrew-formula` job pushes `Formula/slopvac.rb` with that token on each stable release. Candidates skip it. Users then run `brew install srobroek/tap/slopvac`.

## PyPI Trusted Publishing for candidates

PyPI matches the publishing workflow by file name and environment. Candidates upload from a new workflow, so register it for the `slopvac` project at <https://pypi.org/manage/project/slopvac/settings/publishing/> before the first candidate:

| Field | Value |
| --- | --- |
| Owner | `srobroek` |
| Repository name | `slopvac` |
| Workflow name | `candidate.yml` |
| Environment name | `pypi` |

Stable releases keep the registration of `publish-lint.yml` that already exists. Until the flow is moved, that workflow also publishes the 2.x line. If a later change publishes stable wheels from another workflow, register that file the same way.

## The `pypi` environment

In Settings, Environments, `pypi`:

- Add required reviewers (at least one maintainer who did not cut the tag).
- Under deployment branches and tags, allow the `rc/v*` and `v*` tag patterns.
- Add no secrets. Trusted Publishing needs none.

## Protect candidate tags

The stable gate trusts the candidate tag. Extend the ruleset that [distribution](distribution.md) describes for `v*` tags to `rc/v*`, so nobody can move or delete a tag after its run passed.

## Check the setup

Run the `Release candidate` workflow by hand (`workflow_dispatch`) on a branch with `dry-run` on. It exercises the build and tests and publishes nothing. The first real upload happens only when someone pushes an `rc/vX.Y.Z-rc.N` tag and approves the `pypi` environment.
