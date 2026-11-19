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

A separate workflow, `tag-roots.yml`, runs after pushes to main and weekly, and fails when any remote tag points at a commit outside this repository's history. To catch a stray tag before it leaves your machine, run `prek install -t pre-push`. The hook then refuses any push while a local tag has a different root commit, and it lists those tags by full ref name. Remove each listed tag by hand with `git update-ref --no-deref -d <ref>`, using the full ref name as listed, which works for symbolic tags and for names that start with a hyphen.

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

Contributors sign through [CLA Assistant](https://cla-assistant.io). On your first pull request, the CLA Assistant bot posts a comment with a link. Sign at that link with your GitHub account.

You sign once for each version of the agreement. If the text changes, CLA Assistant asks you to sign again.

The `license/cla` check covers one account for each commit:

- The linked GitHub account of the commit author.
- The linked GitHub account of the committer, when the author has no linked account.

Each such account must sign. `Co-authored-by` trailers are not read, so a co-author named only in a trailer is not checked.

The text that you sign is in the [slopvac CLA Gist](https://gist.github.com/srobroek/5374c748726c6ec555de263ba96c15b4). [`CLA.md`](CLA.md) in this repository is the same text.

### Contributions from your employer

If you created the contribution as part of your job, your employer must approve the agreement first. The "How to use this Contributor Agreement" section of the CLA says so. The project has no entity agreement yet. If your employer needs one, open an issue.

### Bots

Bot accounts that the maintainer allowlists in CLA Assistant don't sign.

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
