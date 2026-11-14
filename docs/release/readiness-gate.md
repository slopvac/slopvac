# Release-readiness gate

`scripts/release_gate.py` runs the checklist for the 3.0.0 release (bead `slopvac-8l4.13.9`) and prints one row of evidence per item. A candidate is promoted to stable only when the script exits 0.

```sh
python3 scripts/release_gate.py                   # Markdown table
python3 scripts/release_gate.py --json            # machine-readable
python3 scripts/release_gate.py --evidence PATH   # other evidence file
```

| Exit | Meaning |
| --- | --- |
| 0 | Every item passed. |
| 1 | At least one item failed. |
| 2 | A tool such as `bd`, `cargo`, or `cargo-nextest` is missing, or the tracker output is empty or malformed, so an item could not run. This is never a pass. |

## Items

| Item | How it is checked |
| --- | --- |
| All `release:3.0.0` beads closed | `bd list --label release:3.0.0` returns nothing, except the ids in `allowed_open` in `scripts/release-gate.toml`. |
| Zero open `stub` beads | `bd list --label stub` returns nothing. |
| No open `soft-blocked` bead with an open swap bead | For each open `soft-blocked` bead, no open `stub` bead links to it or names it, and it names no open `stub` bead. |
| Golden suite | `golden` runner below |
| Offline accuracy gate | `accuracy` |
| Compatibility matrix preserve rows | `compat-preserve` |
| Conformance tests (Rust) | `conformance` |
| Contract tests (schemas, positions, lint report, matrix) | `conformance-contract` |
| Migration end-to-end | `migration-e2e` |
| Performance budgets (Rust) | `perf-budgets` |
| Installed binary budgets | `perf-installed-binary` |
| Every mandatory suite is configured | `release-gate.toml` lists all eight commands, each equal to its trusted argv (see below). |
| Recorded approvals | One passing record for each approval in the evidence file (see below). |

The tracker queries need `bd list --json` to print a JSON array whose records carry `id`, `status`, and `labels`. Empty output, a non-array, or a record without those keys makes the item incomplete (exit 2), never a pass.

## Pinned runners

Each item runs one trusted command. The `run` array of its entry in `scripts/release-gate.toml` must equal the argv in `TRUSTED_COMMANDS` in `scripts/release_gate.py`, word for word. The script rejects another executable, a changed filter or flag, an extra command, and a non-string id. It never runs a rejected command, and it runs every accepted command from the repository root.

| Item id | Command | State today |
| --- | --- | --- |
| `golden` | `cargo nextest run --workspace --no-tests=fail -E 'test(/^golden::/)'` | Fails until Rust tests with that prefix exist. |
| `accuracy` | the same, with `test(/^accuracy::/)` | Fails until they exist. |
| `compat-preserve` | the same, with `test(/^compat::preserve/)` | Fails until they exist. The matrix marks preserve rows `pending`. |
| `conformance` | the same, with `test(/^conformance::/)` | Fails until they exist. |
| `conformance-contract` | `python3 -m pytest -q tests/contract` | Runs today. |
| `migration-e2e` | the same nextest form, with `test(/^migration_e2e::/)` | Fails until they exist. |
| `perf-budgets` | the same nextest form, with `test(/^perf_budget::/)` | Fails until they exist. |
| `perf-installed-binary` | `python3 scripts/perf/binary_bench.py run --binary target/release/slopvac --budgets scripts/perf/binary-budgets.toml --enforce` | Fails until the release binary and the budgets file exist. |

The nextest items pass `--no-tests=fail`, so a suite with no tests fails instead of passing with zero tests. Run the script where `python3` has the dev dependencies, for example `uv run --extra dev python scripts/release_gate.py`.

## Recorded approvals

A person gives these approvals, so the script reads them from `scripts/release-evidence.toml`. The lead creates and fills that file, and it is not committed with the script. A missing file fails every item below.

| Item id | Approval | Bead |
| --- | --- | --- |
| `security-review` | Security review sign-off for the plugin host, trust store, OCI client, and extraction | `slopvac-8l4.9.11` |
| `signed-artifacts-<triple>` | Signed installed artifacts, one item for each of the six distribution targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`, `x86_64-pc-windows-msvc` | `slopvac-8l4.13.3` |
| `action-contract` | GitHub Action contract | `slopvac-8l4.13.6` |
| `hook-contract` | Published pre-commit hook contract | `slopvac-8l4.13.8` |
| `policies-doc` | `docs/contract/policies.md` approved | `slopvac-8l4.2.12` |
| `accessibility-manual-pass` | Manual keyboard and screen-reader pass that policies section 2.3 requires | `slopvac-8l4.12.2` |

Migration end-to-end results come from the `migration-e2e` suite, not from the evidence file.

Record each approval as one table:

```toml
[[item]]
id = "security-review"
status = "pass"
evidence = ["https://github.com/srobroek/slopvac/issues/123"]
approver = "alias"
date = 2026-11-14
```

An item passes only when `status` is `pass`, `evidence` holds at least one link and every link starts with `https://`, `approver` is not empty, and `date` is a TOML date. The script rejects unknown keys, unknown item ids, and duplicate items, and reports them in an `evidence-file` row.

## Test name convention

The Rust suite items select tests by module path prefix. An epic that owns a suite names its tests to match:

- `golden::` for golden corpus tests.
- `accuracy::` for the precision and recall gate.
- `compat::preserve` for compatibility matrix rows with the `preserve` disposition.
- `conformance::` for the tests described in `docs/contract/*.md`.
- `migration_e2e::` for migration end-to-end tests on fixture repositories.
- `perf_budget::` for budget measurements. These tests run the installed CLI.

Changing a prefix means changing the entry in `scripts/release-gate.toml` and `TRUSTED_COMMANDS` in `scripts/release_gate.py` together.
