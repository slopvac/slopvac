# Maintainer setup for candidates

The stable release prerequisites live in [distribution](distribution.md#maintainer-setup): the Homebrew tap and token, the release-please App, `RULESET_READ_TOKEN`, the `v*` tag ruleset and immutable releases. This page lists only what [release candidates](candidates.md) add. A maintainer does these by hand, because no workflow or script in this repository changes repository, PyPI or GitHub settings.

## PyPI Trusted Publishing for candidates

PyPI matches the publishing workflow by file name and environment. Candidates upload from a new workflow, so register it for the `slopvac` project at <https://pypi.org/manage/project/slopvac/settings/publishing/> before the first candidate:

| Field | Value |
| --- | --- |
| Owner | `srobroek` |
| Repository name | `slopvac` |
| Workflow name | `candidate.yml` |
| Environment name | `pypi` |

Stable releases keep the registration of `publish-lint.yml` that already exists. If a later change publishes stable wheels from another workflow, register that file the same way.

## The `pypi` environment

In Settings, Environments, `pypi`:

- Add required reviewers (at least one maintainer who did not cut the tag).
- Under deployment branches and tags, allow the `rc/v*` and `v*` tag patterns.
- Add no secrets. Trusted Publishing needs none.

## Protect candidate tags

Create a second ruleset that targets `refs/tags/rc/v*` with the same settings as the `v*` ruleset in [distribution](distribution.md#protect-version-tags): active, with the update, deletion and non-fast-forward rules, and a bypass list that is empty or admin-only.

No workflow checks this ruleset. `scripts/release_ruleset.py` accepts only plain `vX.Y.Z` tags. The stable gate does not rely on it: it needs a passed `candidate.yml` push run for the candidate tag at its current commit, so a moved tag has no passed run until its run passes. The ruleset keeps the tag that testers installed from fixed.

## Check the setup

Run the `Release candidate` workflow by hand (`workflow_dispatch`) on a branch with `dry-run` on. It exercises the build and tests and publishes nothing. The first real upload happens only when someone pushes an `rc/vX.Y.Z-rc.N` tag and approves the `pypi` environment.
