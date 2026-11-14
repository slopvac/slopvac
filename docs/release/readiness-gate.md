# Release-readiness gate

`scripts/release_gate.py` runs the checklist for the 3.0.0 release (bead `slopvac-8l4.13.9`) and prints one row of evidence per item. A candidate is promoted to stable only when the script exits 0.

```sh
python3 scripts/release_gate.py          # Markdown table
python3 scripts/release_gate.py --json   # machine-readable
```

| Exit | Meaning |
| --- | --- |
| 0 | Every item passed. |
| 1 | At least one item failed. |
| 2 | A tool such as `bd` or `cargo` is missing, so an item could not run. This is never a pass. |

## Items

| Item | How it is checked |
| --- | --- |
| All `release:3.0.0` beads closed | `bd list --label release:3.0.0` returns nothing, except the ids in `allowed_open` in `scripts/release-gate.toml`. |
| Zero open `stub` beads | `bd list --label stub` returns nothing. |
| No open `soft-blocked` bead with an open swap bead | For each open `soft-blocked` bead, no open `stub` bead links to it or names it, and it names no open `stub` bead. |
| Golden suite | `cargo nextest run -E 'test(/^golden::/)'` |
| Offline accuracy gate | `cargo nextest run -E 'test(/^accuracy::/)'` |
| Compatibility matrix preserve rows | `cargo nextest run -E 'test(/^compat::preserve/)'` |
| Conformance tests | `cargo nextest run -E 'test(/^conformance::/)'` |
| Migration end-to-end | `cargo nextest run -E 'test(/^migration_e2e::/)'` |
| Every mandatory suite is configured | `release-gate.toml` lists all six suites, and each nextest suite passes `--no-tests=fail`. |
| Performance budgets | `cargo nextest run -E 'test(/^perf_budget::/)'` |

The suite items pass `--no-tests=fail`. A suite that does not exist yet fails instead of passing with zero tests. The script rejects a config that drops a mandatory suite or omits that flag. It runs every command from the repository root.

## Test name convention

The five suite items select tests by module path prefix. An epic that owns a suite names its tests to match:

- `golden::` for golden corpus tests.
- `accuracy::` for the precision and recall gate.
- `compat::preserve` for compatibility matrix rows with the `preserve` disposition.
- `conformance::` for the tests described in `docs/contract/*.md`.
- `migration_e2e::` for migration end-to-end tests on fixture repositories.
- `perf_budget::` for budget measurements. These tests run the installed CLI.

To use a different prefix, edit the `run` filter of that entry in `scripts/release-gate.toml`.

## Items outside the script

These approvals need a person, so the lead records them next to the script output:

- Security review sign-off for the plugin host, trust store, OCI client, and extraction (`slopvac-8l4.9.11`).
- Signed installed artifacts on the target matrix (`slopvac-8l4.13.3`).
- Migration end-to-end results on fixture repositories (`slopvac-8l4.16.1`).
- The Action and published hook contracts (`slopvac-8l4.13.6`, `slopvac-8l4.13.8`).
