# Plan: retire the Python implementation

Bead `slopvac-8l4.16.4`. This is a plan and a draft pull request. It lands only when the lead decides. Nothing in this pull request removes code.

## Preconditions

All of these must hold before the removal pull request merges:

1. The golden suite (`slopvac-8l4.2.7`) is closed and its fixtures live outside `packages/slopvac-lint`.
2. The Rust binary passes the golden suite in CI on the exact head of the removal branch.
3. The release candidate pipeline (`slopvac-8l4.13.4`) has published a candidate that built from the Rust workspace, and `uvx slopvac` runs the binary wheel.
4. The lead has tagged the last Python commit as `py-2.x-final`. Only the lead pushes tags.
5. `slopvac migrate` (`slopvac-8l4.16.1`) is closed with no open `stub` bead under `slopvac-8l4.16`.

## What stays

- The golden suite and its recorded Python outputs. They are the lasting parity test.
- The Python plugin SDK. Plugins speak JSON-RPC over stdio, and the SDK is how Python authors write them.
- Rule data and rule YAML, once the rule converter has moved them to the 3.0 pack layout. Before that, the Python loader is the only reader.
- `schemas/`, `docs/contract/`, and `docs/design/`.

## What goes

| Path | Action |
|---|---|
| `packages/slopvac-lint/src/slopvac/` | Delete. The tag `py-2.x-final` keeps the history. |
| `packages/slopvac-lint/tests/` | Move the golden-suite parts to `tests/golden/`. Delete the rest. |
| `packages/slopvac-lint/vale/` | Delete. Vale is removed. |
| `packages/slopvac-lint/pyproject.toml`, `uv.lock` | Replace with a `pyproject.toml` that builds the maturin binary wheel. |
| `scripts/install_vale.py`, `scripts/action_entry.py` | Delete `install_vale.py`. Port `action_entry.py` to the Rust action. |
| `pyproject.toml` (workspace) | Drop the `slopvac` workspace member. Keep the dev group for the plugin SDK and golden-suite tooling. |
| `tests/test_action_entry.py`, `tests/test_reporting_contract.py`, `tests/test_rc_version.py` | Port or delete with their scripts. |

## Files that name the package path

These reference `packages/slopvac-lint` and need an edit in the removal pull request. The list comes from a search of the repository:

- `.github/ci-paths.yml`, `.github/dependabot.yml`
- `.github/workflows/`: `markdown-audit.yml`, `cli-e2e.yml`, `lint.yml`, `test.yml`, `publish-lint.yml`, `action-test.yml`
- `release-please-config.json`, `action.yml`, `_typos.toml`, `.pre-commit-config.yaml`
- `AGENTS.md`, `CONTRIBUTING.md`, `README.md`
- `scripts/rc_version.py`, `scripts/perf/perf.py`, `scripts/perf/scenarios.toml`

The workflow files belong to the Rust code-quality epic (`slopvac-8l4.14`). Ask its owner for the workflow edits, or agree on a split before the pull request opens.

## Own configuration

The repository's `slopvac.toml` is a 2.x file. The removal pull request runs `slopvac migrate --write` on it, then reviews the diff. It also moves the repository's `slopvac-allow` comments and steering blocks. The result must lint the repository's docs with exit 0. The `[[overrides]]` blocks need a look, because 3.0 matches globs from the config's directory.

## CONTRIBUTING

Rewrite the development setup for Rust: `mise install`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo nextest run`. Keep the prose-check and rule-authoring sections. Point the rule-authoring section at the 3.0 rule schema and the golden suite instead of the Python ruleset tests.

## Order of work

1. Move the golden suite and SDK to their final paths. Merge on its own.
2. Port the helper scripts that the workflows call, and the action entry point.
3. Switch the workflows, the dependency bot and release-please to the Rust workspace.
4. Migrate the repository's own config and docs with `slopvac migrate`.
5. Delete `packages/slopvac-lint/src`, `vale/`, and the Python tests.
6. Update CONTRIBUTING and README, then rebuild the generated references.

Steps 1 to 3 can land before the cutover, because each leaves the Python path working. Steps 4 to 6 land together after the preconditions hold.

## Verification

- `cargo nextest run` and the golden suite pass.
- No file outside `docs/design/` and `CHANGELOG.md` names `packages/slopvac-lint`. Check with `git grep`.
- `slopvac lint` on the repository's docs exits 0 using the migrated config.
- A release candidate builds from the branch and installs through `uvx`.

## Risks

- The `maturin` wheel build and the PyPI publish workflow must move together. A gap leaves `uvx slopvac` without a release.
- Dependabot ecosystem entries for `pip` change. Keep them for the SDK only.
- Removing the Python tests removes the only running check of the 2.x rule semantics. The golden suite must carry every case first.
