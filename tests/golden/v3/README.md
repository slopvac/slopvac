# Golden suite v3

This suite fixes what a slopvac 3.0 binary must reproduce. Three modes share one set of inputs and one projection of the lint report.

| Mode | Expectation | Source of truth |
| --- | --- | --- |
| `parity` | The binary matches the Python 2.x reference. | `parity/<profile>/<input>.json`, recorded from the reference pinned in `pin.toml`. |
| `preserve` | The binary, given the generated `core:legacy-<profile>` style, matches the same files. | The `parity` files. The profile mapping makes the two equal ([profiles.md](../../../docs/contract/profiles.md)). |
| `policy` | The binary under the 3.0 default policy matches the `parity` files plus reviewed deltas. | `parity/` and `policy/deltas.toml`. |

## Inputs

`inputs.toml` lists every input with its SHA-256. A test fails when a file changes.

- `rule-examples`: one document for each category, holding the `bad` and `good` examples of its rules, one paragraph each. `golden.py inputs` regenerates them.
- `stacklok`: the permissions page before and after the rewrite (`slopvac-8l4.2.8`).
- `issues`: the regression samples for issues 252 to 257 (`slopvac-8l4.2.8`).

A corpus tier from the eval corpus (`slopvac-8l4.15.1`) is not part of the suite yet. A swap bead tracks it.

## Projection

The 2.x reference and a 3.0 binary both reduce their lint JSON to one projection (`project_report` in `scripts/contract/golden.py`). It holds the exit code and, for each document, the counts, score, gate, category scores, unchecked notes and findings. A finding holds the rule id (`core:<category>.<id>`), category, severity, line, column, end position, byte span, matched text, message, AI signal and the replacement.

Keys that only 3.0 produces are outside the projection, because the reference cannot ground them: genre, contexts, config sources, provenance and pack. Their tests belong to the beads that add them.

2.x masks code spans and folds whitespace in `matched_text`. For those findings the end position cannot be proved from the source, so the golden keeps line and column and sets `end_line`, `end_column` and `span` to `null`. The check ignores the end position of such a finding in the candidate.

## Pin

`pin.toml` records the reference: the package version, a hash of its source and rule data, and the Vale version. The pin test fails when the reference changes. A fix to rule data lands in the Python package first, and then the golden is recorded again.

## Approved changes

A golden file changes only with an approved reason. Record the reason in the pull request that changes the file, name the rules and inputs it touches, and get a maintainer's approval before merge. A mechanical re-record, such as a pin update after a rule fix, lists the changed findings in the pull request.

Policy deltas are different: each entry in `policy/deltas.toml` needs `reason` and `approved_by`, and a test rejects an entry without them.

## Run

```sh
python scripts/contract/golden.py record --reference "<2.x slopvac>"
python scripts/contract/golden.py check  --candidate "<3.0 slopvac>" --mode parity
python scripts/contract/golden.py check  --candidate "<3.0 slopvac>" --mode preserve
python scripts/contract/golden.py check  --candidate "<3.0 slopvac>" --mode policy
python scripts/contract/golden.py diff   --a "<binary>" --b "<binary>" --mode parity
```

`diff` compares two binaries with each other, with no golden file. A command is a program prefix. The runner appends `lint <files> --config <config> --profile <profile> --format json`.
