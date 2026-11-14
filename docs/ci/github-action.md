# GitHub Action

The action installs slopvac, lints the files you choose, and reports the findings. The exit code of `slopvac lint` is the gate: the job passes or fails on it, whatever the reporters do. Until the 3.0 cutover the action lives in `action-v3/`; at release it moves to the repository root and the examples below apply as written.

```yaml
permissions:
  contents: read
  pull-requests: write # reviewdog review comments
  security-events: write # code scanning upload

jobs:
  slopvac:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v7
        with:
          fetch-depth: 0
      - uses: srobroek/slopvac@v3
        with:
          paths: README.md docs/
          changed-files-only: true
```

## Reports

| Report | Where it goes |
| --- | --- |
| reviewdog diagnostics (`rdjsonl`) | A pull request review with suggestions, annotations, or a check run, as the `reporter` input selects. |
| SARIF | Code scanning, with the category set by `sarif-category` (default `slopvac`). Set `sarif: false` to skip it. |
| Code Climate | The `slopvac-codeclimate` workflow artifact. Set `codeclimate: false` to skip it. |

The `reporter` input takes `auto`, `review`, `annotations` or `none`. With `auto`, the action tries a review with suggestions on a pull request from the same repository that Dependabot did not open. The action cannot see the token's permissions, so if reviewdog fails to post the review, for example on a read-only token, it logs a warning and posts annotations instead. In every other pull request it uses annotations, which need no write permission. On pushes it uses a check run. A reviewdog install or post failure only warns; the slopvac exit code stays the gate.

When the project has a `.slopvac/packs.lock` lockfile, a separate `Sync packs` step runs `slopvac plugins sync` before the lint. A failed sync fails the action.

The lint steps run with `--offline`, so they never reach the network. Installing slopvac still does. Set `offline: false` to allow network access during the lint.

Set `fail-on-findings: false` to report without failing the job. That softens only exit 1. An incomplete check (exit 2), any other exit status and a crash from a signal always fail the job.

## Pull requests from forks

On a fork pull request the token is read-only, so the action falls back to annotations. To get review comments on forks, split the job in two. The first workflow lints with no write access and uploads the findings. The second runs from the default branch with write access, and never checks out the pull request.

```yaml
# .github/workflows/slopvac.yml
on: pull_request
permissions:
  contents: read
jobs:
  lint:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v7
      - id: slopvac
        uses: srobroek/slopvac@v3
        with:
          reporter: none
          sarif: false
          rdjsonl-file: findings/findings.rdjsonl
      # Findings fail the action, so the steps below must run after a failure.
      - if: ${{ !cancelled() }}
        env:
          PR_NUMBER: ${{ github.event.pull_request.number }}
        run: |
          mkdir -p findings
          printf '%s\n' "$PR_NUMBER" > findings/pr-number
      - if: ${{ !cancelled() && hashFiles('findings/findings.rdjsonl') != '' }}
        uses: actions/upload-artifact@v7
        with:
          name: slopvac-findings
          path: findings
```

The second workflow runs on `workflow_run` of the first. Treat the artifact as untrusted input: accept only the expected file names and a pull request number made of digits, confirm that the pull request is open and that its head commit is the one the run tested, and only then pipe `findings.rdjsonl` to `reviewdog -f=rdjsonl -reporter=github-pr-review`. This repository's `.github/workflows/reviewdog.yml` is a worked example of those checks.

## Inputs and outputs

See `action-v3/action.yml` for every input and output. The binary version comes from `version`. Pin it for reproducible runs, or leave it empty for the latest stable release.
