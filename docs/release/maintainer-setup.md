# Maintainer setup

These steps change repository, PyPI and GitHub settings, so a maintainer does them by hand. No workflow or script in this repository does them.

## Homebrew tap

1. Create the public repository `srobroek/homebrew-tap` with a `Formula/` directory (an empty repository with a README is enough).
2. Create a fine-grained personal access token with access to that repository only and **Contents: Read and write**. Give it an expiry and put the renewal date in your calendar.
3. In `srobroek/slopvac`, add the token as the Actions secret `HOMEBREW_TAP_TOKEN` (Settings, Secrets and variables, Actions).

The cargo-dist `publish-homebrew-formula` job pushes `Formula/slopvac.rb` with that token on each stable release. Candidates and other prereleases skip it. Users then run `brew install srobroek/tap/slopvac`.

## PyPI Trusted Publishing

PyPI matches the publishing workflow by file name and environment. Register each workflow that uploads to the `slopvac` project at <https://pypi.org/manage/project/slopvac/settings/publishing/>:

| Field | Candidates | Stable releases |
| --- | --- | --- |
| Owner | `srobroek` | `srobroek` |
| Repository name | `slopvac` | `slopvac` |
| Workflow name | `candidate.yml` | `release.yml` |
| Environment name | `pypi` | `pypi` |

The stable row applies once the cargo-dist workflow publishes wheels. The existing `publish-lint.yml` registration serves the 2.x flow and stays until cutover.

## The `pypi` environment

In Settings, Environments, `pypi`:

- Add required reviewers (at least one maintainer who did not cut the tag).
- Under deployment branches and tags, allow the `rc/v*` and `v*` tag patterns.
- Add no secrets. Trusted Publishing needs none.

## Check the setup

Run the `Release candidate` workflow by hand (`workflow_dispatch`) on a branch with `dry-run` on. It exercises the build and tests and publishes nothing. The first real upload happens only when someone pushes an `rc/vX.Y.Z-rc.N` tag and approves the `pypi` environment.
