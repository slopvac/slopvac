# Contributing

## Development setup

Install the pinned development tools and project dependencies from the repository root:

```sh
mise install
uv sync --extra dev
```

`mise.toml` pins Python, uv, Vale, and other development tools. The project supports Python 3.11 or later. Vale 3.18 or later enables complete linting. Run `mise install` to install pinned tools.

Run the test suite from the repository root:

```sh
mise exec -- uv run pytest
```

Run it through `mise exec --` so `vale` resolves to the version pinned in `mise.toml`. Tests marked `vale` run only when a `vale` is on `PATH`. If the `vale` on `PATH` is not the pin (for example a Homebrew install), the run stops with a usage error. CI installs the same pin.

The root `pyproject.toml` configures pytest to discover `packages/slopvac-lint/tests` and `tests`. CI runs the full suite with `python -m pytest` from the repository root. For documentation changes, CI runs the Slopvac prose gate and Markdown checks.

## Rust workspace

The 3.0 engine is a Cargo workspace under `crates/`. It sits beside the Python package, which stays the reference implementation. `rust-toolchain.toml` pins the toolchain, and `mise install` adds cargo-nextest. Run these from the repository root:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace --no-tests=pass
```

A new crate needs a `Cargo.lock` entry in `extra-files` in `release-please-config.json`, or the release PR does not bump its version. After you add or remove a crate, run `just sync-release-please` (or `python3 scripts/sync_release_please_crates.py`) to rewrite those entries. The quick-lint CI job runs the same script with `--check` and fails on drift.

## Prose checks

Run Slopvac's repository prose gate on the files you changed:

```sh
uvx --from ./packages/slopvac-lint slopvac <files>
```

Exit 2 means the check is incomplete, not clean. If the command exits 2, install Vale 3.18 or later and rerun it. Fix findings before you open a pull request.

### Review comments on pull requests

CI also posts prose and shellcheck findings as review comments on added lines. The lint jobs in `lint.yml` run with read-only permissions and upload their findings as `reviewdog-*` artifacts. Then `reviewdog.yml` runs on `workflow_run` from the default branch, with `pull-requests: write`. It runs without any pull request code, so it works the same for pull requests from forks.

That workflow treats the artifacts as untrusted. It accepts a pull request number only when that PR is open and its head commit is the one the CI run tested. `workflow_run` workflows load from the default branch, so changes to `reviewdog.yml` take effect only after merge. The pass or fail result of the lint jobs does not depend on this path.

Requirements for every Rust change, and the checklist reviewers use, are in [`docs/engineering.md`](docs/engineering.md).

Rust test conventions are in [`docs/testing-rust.md`](docs/testing-rust.md). Install the snapshot reviewer with `cargo binstall cargo-insta` (or `cargo install cargo-insta --locked`) before you change a snapshot.

## Rust checks

Rust CI runs rustfmt, clippy, rustdoc, nextest and doctests. A supply-chain job runs `cargo deny` for licences, bans and sources, `cargo shear` for unused dependencies, `cargo sort` for manifest order and `typos`. Advisories run nightly and report on pull requests without blocking them. `mise install` pins the tools. `prek run --all-files` runs the same checks locally.

Licences: `deny.toml` allows permissive licences only. The CC BY-SA 4.0 part-of-speech model covers the model file, so add a per-crate exception for the crate that embeds it. Never add CC BY-SA to the global allow list.

## Rules

Rule definitions live in [`packages/slopvac-lint/src/slopvac/rules/`](packages/slopvac-lint/src/slopvac/rules/). Each YAML file defines a category and its rules. Add examples of text the rule should flag and text it should leave alone.

Run the ruleset tests after changing examples:

```sh
mise exec -- uv run pytest packages/slopvac-lint/tests/test_ruleset.py
```

These tests check that each lexical rule's `bad` example matches its pattern. A `good` example must not match. The tests check that each lexical rule has an example. For substitutions, the tests check that each example resolves to a replacement.

## Stable releases and release candidates

Merge the release PR. Then run `just release X.Y.Z` to start publishing from
the matching `vX.Y.Z` tag. The workflow waits for approval in the `pypi`
environment before it publishes to PyPI.

Release candidates build from each update to the release PR branch. They also
wait for `pypi` approval. They get no tag or changelog entry.

Plain installs and `slopvac@latest` stay on the latest stable release. The package README lists how to opt in to a candidate.

## Contributor license agreement

Contributions to slopvac need a signed [Contributor License Agreement](CLA.md) (CLA). It is the Individual Contributor Non-Exclusive License Agreement from the Contributor Agreements 1.2 templates (Harmony), with the traditional patent license. You keep the copyright in your contribution. You grant Sjors Robroek a non-exclusive license to use, sublicense and distribute it, and a patent license.

In return, Sjors Robroek takes on two duties:

- License your contribution under licenses that the Free Software Foundation classifies as free and the Open Source Initiative approves.
- Offer material under a free and open license too, when it also has a license that is not free or not open.

The project stays under Apache-2.0. The laws of the Netherlands govern the agreement.

### Sign the CLA

An unsigned pull request fails the `CLA` check. The check names each person who must sign. To sign, post a comment on the pull request. The comment holds exactly this text and nothing else:

```text
I have read the slopvac CLA version harmony-icla-1.2-opt1-v2 and I hereby sign it.
```

Blank lines around the text don't matter. Extra words, a quote or a code block invalidate the comment.

You sign once for each version of the agreement. The signature counts on every pull request that you open. A new version needs a new signature.

If you edit or delete the comment, the signature ends. After that, the check fails on your open pull requests.

### Who must sign

These people must sign:

- The person who opens the pull request.
- The author of each commit.
- The committer of each commit.
- Each co-author in a `Co-authored-by` trailer that uses a GitHub `noreply` address.

Only these people can sign on the pull request. GitHub supplies their identities from its account data. Names and emails in a commit prove nothing.

The email of each author and committer must link to a GitHub account. If an email has no link, the check fails and names the commit. Add that email to your GitHub account settings. Or rewrite the commit with an email that GitHub links to you.

The check can't verify a `Co-authored-by` email that is not a `noreply` address. It lists that email as a warning and skips it.

GitHub lists at most 250 commits for a pull request. A pull request with 250 commits or more fails, because the check can't see all of them. Split it.

### Contributions from your employer

If you created the contribution as part of your job, your employer must approve the agreement first. The "How to use this Contributor Agreement" section of the CLA says so. The project has no entity agreement yet. If your employer needs one, open an issue.

### Bots

These accounts don't sign:

- `dependabot[bot]`
- `renovate[bot]`
- `github-actions[bot]`
- `sjorsr-release-bot[bot]`, the release bot
- `srobroek`, the maintainer
- `web-flow`, the committer GitHub sets on commits made in the web interface

The list gives each account name in full and has no wildcard.

### Which check decides

The CLA GitHub App posts the `CLA` check as a check run. A workflow job can't supply it, because a pull request can edit the workflows that it triggers.

On a pull request, the check covers that pull request. It runs again after these events:

- A push to the pull request.
- Reopening the pull request.
- A change to a signature.

Every open pull request with the same head commit gets the same result. This includes pull requests from forks.

In the merge queue, the check covers the whole merge group. It passes when each pull request in the group has a passing `CLA` check run for the current agreement version. It fails when anything is unclear. Examples are an unreadable queue and a pull request with no check run.

Maintainers merge a pull request only after the `CLA` check passes.

## Commits and pull requests

Use [Conventional Commits](https://www.conventionalcommits.org/). Release-please reads the commit message to prepare releases. Examples: `feat(rules): add a rule`, `fix: correct a rule`, and `docs: clarify setup`.

Open a pull request with a concise description and a list of checks you ran. CI must pass before merge. Maintainers squash-merge pull requests.

## Report a problem or propose a rule

Choose an issue form at [New issue](https://github.com/slopvac/slopvac/issues/new/choose):

The forms cover these categories:

- Bug: covers crashes, tracebacks, wrong exit codes, wrong or unstable output, or config, setup, init, Vale, cache, or prime defects.
- False positive: covers text that a rule flags as slop incorrectly.
- Missed slop: covers slop that no finding reports but an existing rule should catch. Use this form if unsure.
- Mis-tagged: covers a finding on slop with the wrong rule, severity, or span/location.
- Wrong fix: covers a correct match with a wrong message, replacement, or fix guidance. Use this form if `--fix` changes meaning or produces broken text.
- New rule: covers a writing pattern that no existing rule detects.
- Rule improvement: covers whole-rule changes to accuracy, examples, exceptions, judgment, tier, severity, genre advice, wording, or docs.

Before submitting an agent report, run `slopvac prime report`. Get the user's approval for the exact post.

Do not include paths, file names, or private data.
Do not include user names, host names, repository names, or organisation names.
Paste sample text into the report. A local file path is not enough.
Prefer a synthetic sample over text from private or proprietary sources.

Record the agent product name. Record the active model exactly as the harness reports it in its system prompt, context, or `/model` selection. Do not use model self-description or config defaults. Write N/A when the active model is unavailable.
Get the harness version from its version command. Write N/A when unavailable.
Do not infer these fields from environment variables.
