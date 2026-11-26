# GitHub Action

The action installs slopvac, lints the files you choose, and reports the findings. The exit code of `slopvac lint` is the gate: the job passes or fails on it, whatever the reporters do. The action lives at the repository root, so the examples below apply as written.

Pin the action to a full commit SHA, with the version as a trailing comment. The repository publishes no moving major tag such as `v3`, and its `v*` tags are immutable. Replace the placeholder with a SHA from `git ls-remote https://github.com/slopvac/slopvac 'refs/tags/v3.0.0' 'refs/tags/v3.0.0^{}'`. Copy it from the `^{}` row when that row is present (an annotated tag), otherwise from the plain row. Dependabot and Renovate update a SHA pin and its version comment together.

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
      - uses: slopvac/slopvac@<full commit SHA of the v3.0.0 tag> # v3.0.0
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

A separate `Sync plugins` step runs before the lint. When the working directory has a `.slopvac/plugins.lock` lockfile, the step runs `slopvac plugins sync`, which installs what the lockfile pins and checks each digest. Without a lockfile, the step prints a notice and passes. The notice does not show that the project needs no plugins. A project that depends on plugins must commit `.slopvac/plugins.lock`, so the step syncs them before the lint, which runs with `--offline`.

The sync is incomplete, and fails the action, on any nonzero exit or a crash from a signal. The binary exits 2 in these cases:

- It cannot read or accept the lockfile.
- A download fails.
- A digest does not match.
- The config and the lockfile list different plugin ids (`lock-mismatch`). A configured plugin has no lock entry, or a lock entry is no longer needed.

To fix a `lock-mismatch`, run `slopvac plugins sync` outside CI. It updates `.slopvac/plugins.lock`. Commit the updated file.

The step prints the binary's error line, then `plugins sync failed (status <n>)`. The lint step does not run after a failed sync, and `fail-on-findings: false` does not soften it.

The lint steps always run with `--offline`, so they never reach the network. Installing slopvac and syncing plugins still do.

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
        uses: slopvac/slopvac@<full commit SHA of the v3.0.0 tag> # v3.0.0
        with:
          reporter: none
          sarif: false
          rdjsonl-file: findings/findings.rdjsonl
      # Findings fail the action, so the steps below must run after a failure.
      - if: ${{ !cancelled() }}
        env:
          PR_NUMBER: ${{ github.event.pull_request.number }}
          HEAD_SHA: ${{ github.event.pull_request.head.sha }}
        run: |
          mkdir -p findings
          printf '%s\n' "$PR_NUMBER" > findings/pr-number
          printf '%s\n' "$HEAD_SHA" > findings/head-sha
      - if: ${{ !cancelled() && hashFiles('findings/findings.rdjsonl') != '' }}
        uses: actions/upload-artifact@v7
        with:
          name: slopvac-findings
          path: findings
```

The second workflow runs on `workflow_run` of the first. Treat the artifact as untrusted input. This repository's `.github/workflows/reviewdog.yml` (job `comment-v3`) is a worked example, and `scripts/fork_review.py` holds the checks:

- Find the pull request from the `workflow_run` event and the API, never from the artifact. Require one open pull request whose head repository and commit are the ones the event names.
- Require the artifact's `pr-number` and `head-sha` to equal those values. A mismatch rejects the artifact.
- Accept exactly three files: `findings.rdjsonl`, `pr-number` and `head-sha`. Reject any other name, a path with `..` or a separator, a symlink, a duplicate, and a size over the caps: 1 MB for the archive, 2 MB for the report and 64 bytes for each other file. Allow only stored and deflate compression, and read each file in chunks under its own byte cap instead of trusting header sizes.
- Parse every line of the report as a reviewdog Diagnostic. Reject unknown fields, an absolute or `..` file path, and more than 2,000 diagnostics.
- Check out nothing from the pull request, and run nothing from the artifact. Pipe the validated report to `reviewdog -f=rdjsonl -reporter=github-pr-review`.

The action itself refuses `rdjsonl-file`, `sarif-file` and the Code Climate path when they resolve outside `$GITHUB_WORKSPACE` or go through a symlink.

## Inputs and outputs

See `action.yml` at the repository root for every input and output. The action has no 2.x compatibility mode. The binary version comes from `version`. Until 3.0.0 is released, set `version: 3.0.0rc1` or later. An empty value installs the latest stable release, which is still a 2.x package, and the Action then stops with an error that says so. Once 3.0.0 is released, an empty value installs it. Pin the version for reproducible runs.
