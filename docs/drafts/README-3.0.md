# slopvac

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/slopvac-dark-1200-q85.webp">
    <source media="(prefers-color-scheme: light)" srcset="assets/slopvac-light-1200-q85.webp">
    <img alt="Slopvac robot vacuum cleaning prose" src="assets/slopvac-light-1200-q85.webp" width="480">
  </picture>
</p>

[![CI](https://github.com/srobroek/slopvac/actions/workflows/test.yml/badge.svg?branch=main&event=push)](https://github.com/srobroek/slopvac/actions/workflows/test.yml)
[![Security](https://github.com/srobroek/slopvac/actions/workflows/security.yml/badge.svg?branch=main&event=schedule)](https://github.com/srobroek/slopvac/actions/workflows/security.yml)
[![PyPI version](https://img.shields.io/pypi/v/slopvac)](https://pypi.org/project/slopvac/)

[Install](#install) · [Quick start](#quick-start) · [What it checks](#what-it-checks) ·
[Agent setup](#agent-setup) · [CI](#ci-integration) · [Packs and plugins](#packs-and-plugins) ·
[Upgrade from 2.x](#upgrade-from-2x) · [Reference](https://srobroek.github.io/slopvac/)

`slopvac` lints prose and source comments. Its rules cover AI writing patterns and general prose
quality, including documentation discipline and technical-writing constraints. Use it on
AI-generated documentation, PR descriptions, Markdown, and comments in supported source files.

Coding agents use the CLI to inspect findings and apply fixes. The same checks run in pre-commit,
CI, and editors.

slopvac is one native binary. The core needs no Vale. Markdown, MDX, and HTML need no Python. RST
may need a Python runtime for its bundled docutils adapter. A plugin can need its own runtime.

## Install

| Channel | Command |
| --- | --- |
| uv (default) | `uv tool install slopvac`, or run once with `uvx slopvac` |
| Homebrew | `brew install srobroek/tap/slopvac` |
| Release archive or installer | Download from the slopvac [GitHub releases](https://github.com/srobroek/slopvac/releases) |

`uvx slopvac@latest` forces the latest stable release. Release archives carry minisign signatures
and GitHub attestations.

Release builds cover macOS (arm64, x86_64) and Linux (arm64, x86_64). Windows is supported through
WSL: inside a WSL distribution, use the Linux build with the shell installer, Homebrew on Linux, or
`uvx slopvac`. Native Windows is a 3.x feature.

`slopvac self update` updates an install that the slopvac installer made. For any other install, it
prints the upgrade command for that channel.

## Quick start

```sh
slopvac init
slopvac lint README.md
```

`init` creates `slopvac.toml` when absent and keeps an existing file unless you pass `--force`. A
passing run exits 0. Findings above a threshold exit 1. An incomplete check exits 2. Treat exit 2 as
a failed check, never as a pass.

```sh
slopvac lint README.md --format json --out slopvac-report.json
slopvac lint README.md --fix
```

`--fix` applies safe substitutions only and reports the other findings. Review the diff before you
keep the edits.

Output formats:

- `text`
- `json`
- `github`
- `sarif`
- `rdjsonl`
- `codeclimate`
- `junit`
- `checkstyle`
- `markdown`
- `html`

The language server reports the same findings to editors. SARIF and rdjsonl output link each rule
to its documentation page. The [documentation site](docs/site.md) lists the page layout.

## What it checks

| Coverage | Examples |
| --- | --- |
| AI writing patterns | Repeated structures, inflated register, formatting tells, and generated residue |
| General prose | Agency, word choice, scope, consistency, and promotional language |
| Documentation discipline | Internal references, status language, and project history in user documentation |
| Simplified Technical English | Sentence length, noun groups, procedural writing, and safety text |
| Orwell-derived checks | Stale figures of speech, excess words, and unclear agency |

Some rules are judgment rules. They run in review, not in lint. The writing guidance and the semantic
review apply them.

Inspect rules from the CLI:

```sh
slopvac rules
slopvac explain prose-craft.relative-date
```

## Configure a project

Settings live in `slopvac.toml`, which starts with `schema_version = 3`. The nearest config to each
file applies. The file can set:

- `styles` and `extends`, which choose the rule presets and base configs
- `packs` and `plugins`, which add data packs and code plugins
- `[thresholds]`, which set the gates that fail a run
- `[[overrides]]`, which change settings for matching paths
- `[guidance]`, which sets how much detail `prime` prints
- `[network]`, which sets offline mode and the update check

Run this to see where each value comes from:

```sh
slopvac lint --explain-config README.md
```

It prints the layer, the file, and the line for each value, and what the value overrode. An unknown
key in a user file is an error. Keys that start with `x-` are ignored.

Terms live in `.slopvac/terms.toml`. Each term needs only `term`.

## Network use

The update check is the only network use of `lint`. It runs only for `lint`, `doctor`, `prime`, and
`self update`, and it caches its result for 24 hours. Pack commands such as `plugins sync` connect
only when you run them.

These settings stop the update check:

- `--offline` or `SLOPVAC_OFFLINE=1`
- `[network] offline = true` in `slopvac.toml`
- `[network] update_check = false` in `slopvac.toml`
- `NO_UPDATE_NOTIFIER=1`, `SLOPVAC_NO_UPDATE_CHECK=1`, or `DO_NOT_TRACK=1`
- The `CI` environment variable

Offline mode is sticky. When any source turns it on, no other source turns it off.

## Agent setup

```sh
slopvac setup opencode
```

`setup <harness>` writes, checks, or removes the steering block and the skill for one harness.
`slopvac setup --list` prints the supported harnesses and their paths.

Agents get guidance on demand:

```sh
slopvac prime          # commands and topics
slopvac prime write    # before drafting prose
slopvac prime review   # at hand-off
```

`prime write` prints the genre table and the documentation contract. `prime review` prints the review
workflow. The agent lints first, answers the judgment checklist for the genre, and checks factual
claims against the source. The agent's own model runs the review. slopvac prints the workflow and
does not call a model. Set `[guidance] level` in `slopvac.toml` to change the detail of each topic. A
`.slopvac/PRIME.md` file overrides a topic.

## CI integration

### pre-commit

```yaml
repos:
  - repo: https://github.com/srobroek/slopvac
    rev: v3.0.0 # x-release-please-version
    hooks:
      - id: slopvac
```

### GitHub Actions

```yaml
- uses: actions/checkout@v7
- uses: srobroek/slopvac@v3.0.0 # x-release-please-version
  with:
    paths: README.md docs/
```

The action has no `profile` input in 3.0. Choose styles and thresholds in `slopvac.toml`, and point
the action at the file with the `config` input. It posts review comments with reviewdog by default,
or uploads SARIF. See [action.yml](action.yml) for the inputs and outputs.

## Packs and plugins

A data pack holds rules, styles, genres, terms, and declarative taggers. It contains no code. A code
plugin speaks JSON-RPC 2.0 over stdio and can add taggers, extractors, reporters, fixers, scorers,
and model providers.

Packs are OCI artifacts on GHCR or any registry. Release archives, local folders, and git commits
also work. `.slopvac/packs.lock` records the version, source, and digest of each pack.

A minisign signature covers each published OCI pack, and `plugins sync` fails with exit 2 when the
signature is missing or invalid. Local folders and git commits carry no signature. A git pin names a
commit, and the trust hash covers the content.

slopvac refuses to load a code plugin until it is trusted. The refusal prints the command:

```sh
slopvac plugins trust <name>
```

Trust is recorded per content hash. Two things record trust: `plugins trust`, and a `SLOPVAC_TRUST`
variable that a person wrote into the CI configuration. `plugins sync` and `plugins update` never
record trust. Agents never trust on your behalf. `pack new`, `validate`, `test`, and `publish` build
and release your own packs.

## Upgrade from 2.x

3.0 has no 2.x compatibility mode. It rejects a 2.x config with exit 2 and points to
`slopvac migrate`. Removed flags, options, and hooks fail with an error.

```sh
uvx slopvac@latest prime migrate
slopvac migrate
```

`migrate` previews the changes, then rewrites the config with comments kept. See the
[migration guide](docs/migrate-2-to-3.md).

## Limits

A passing run means the selected checks stayed within the configured thresholds. It does not
establish factual correctness.

## Inspired by Vale

slopvac began as a set of Vale styles, and its rule test format and style-and-package model come
from [Vale](https://vale.sh). Thanks to the Vale authors and contributors. slopvac 3.0 runs without
Vale.

For Markdown hygiene, use [rumdl](https://github.com/rvben/rumdl) next to slopvac.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) for development and pull request guidance. Use
[SECURITY.md](SECURITY.md) to report vulnerabilities in private. File bugs, rule findings, and
proposals with an [issue form](https://github.com/srobroek/slopvac/issues/new/choose). Do not post
security reports as issues.

## License

Apache-2.0. Includes rules derived from [hardikpandya/stop-slop](https://github.com/hardikpandya/stop-slop)
(MIT). The embedded part-of-speech model is licensed CC BY-SA 4.0, and that licence covers the
model file only.
