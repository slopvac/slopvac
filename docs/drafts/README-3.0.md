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

slopvac is one native binary. It needs no Python, Vale, or network access to lint.

## Install

| Channel | Command |
| --- | --- |
| uv (default) | `uv tool install slopvac`, or run once with `uvx slopvac` |
| Homebrew | `brew install srobroek/tap/slopvac` |
| Release archive or installer | Download from the slopvac [GitHub releases](https://github.com/srobroek/slopvac/releases) |

`uvx slopvac@latest` forces the latest stable release. Release archives carry minisign signatures
and GitHub attestations.

`slopvac self update` updates an install that the slopvac installer made. For any other install, it
prints the upgrade command for that channel.

## Quick start

```sh
slopvac init
slopvac lint README.md
```

`init` creates `slopvac.toml` when absent and keeps an existing file unless you pass `--force`. A
passing run exits 0. Findings above a threshold exit 1. An incomplete check exits 2, and it is never
a pass.

```sh
slopvac lint README.md --format json --out slopvac-report.json
slopvac lint README.md --fix
```

`--fix` applies deterministic replacements. Review the diff before you keep them.

Output formats are `text`, `json`, `sarif`, `rdjsonl`, `codeclimate`, `junit`, `checkstyle`,
`markdown`, and `html`. The language server reports the same findings to editors. Each finding links
to the page for its rule.

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

A genre sets the profile for a document. `slopvac genre <file>` prints the detected genre. The
`normal` profile is the default. Use `strict` for reference material and `relaxed` for informal text.
Styles are ordered presets such as `ste` and `google`. The last style listed wins when two conflict.

## Configure a project

Settings live in `slopvac.toml`. The nearest config to each file applies. Run this to see where each
value comes from:

```sh
slopvac lint --explain-config README.md
```

It prints the layer, the file, and the line for each value, and what the value overrode. Unknown keys
in a user file are errors. Keys that start with `x-` are ignored.

Terms live in `.slopvac/terms.toml`. Each term needs only `term`.

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
workflow: lint first, then the judgment checklist for the genre, then a check of factual claims
against the source. The agent's own model runs the review. slopvac prints the workflow and does not
call a model. Set `[guidance] level` in `slopvac.toml` to change the detail of each topic. A
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
    profile: normal
```

The action posts review comments with reviewdog (`reporter: auto`) or uploads SARIF. See
[action.yml](action.yml) for the inputs and outputs.

## Packs and plugins

A data pack holds rules, styles, genres, terms, and declarative taggers. It contains no code. A code
plugin speaks JSON-RPC 2.0 over stdio and can add taggers, extractors, reporters, fixers, scorers,
and model providers.

Packs are OCI artifacts on GHCR or any registry. Release archives, local folders, and git commits
also work. `.slopvac/packs.lock` records the version, source, and digest of each pack. A minisign
signature covers each published pack.

slopvac refuses to load a code plugin until you trust it. The refusal prints the command:

```sh
slopvac plugins trust <name>
```

Trust is recorded per content hash. Agents and CI never trust on your behalf. `pack new`, `validate`,
`test`, and `publish` build and release your own packs.

## Upgrade from 2.x

3.0 has no 2.x compatibility mode. It rejects a 2.x config with exit 2 and points to
`slopvac migrate`. Removed flags, options, and hooks fail with an error.

```sh
uvx slopvac@latest prime migrate
slopvac migrate
```

`migrate` previews the changes, then rewrites the config with comments kept. Vale is no longer used.
See the [migration guide](docs/migrate-2-to-3.md).

## Limits and evaluation

A passing run means the selected checks stayed within the configured thresholds. It does not
establish factual correctness. AI-signal labels describe rule evidence and do not change the gate.

## Inspired by Vale

slopvac began as a set of Vale styles, and Vale shaped its design. The rule test format and the
style-and-package model come from [Vale](https://vale.sh). Thank you to the Vale authors and
contributors. slopvac 3.0 no longer runs or requires Vale.

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
