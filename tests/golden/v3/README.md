# Golden suite v3

This suite fixes what a slopvac 3.0 binary must reproduce. Three modes share one set of inputs and one projection of the lint report.

| Mode | Candidate configuration | Expectation |
| --- | --- | --- |
| `parity` | The reference-equivalent legacy configuration: the profile and the generated `core:legacy-<profile>` style, which carries the 2.x category dials, rule defaults and STE tiers. | The Python 2.x reference output, recorded in `parity/<profile>/<input>.json` and `runs/<profile>.json`. |
| `preserve` | The config that `migrate --preserve-behavior` writes for `profile = "<profile>"`. | The `parity` files. The profile mapping makes the two equal (`docs/contract/profiles.md`, added by the profile mapping PR). |
| `policy` | The 3.0 default styles for the profile. | The `parity` files plus reviewed deltas from `policy/deltas.toml`. |

A 3.0 binary takes its profile from the config, because `--profile` is removed in 3.0. The 2.x reference runs with `--flavor 2x`: an empty config and `--profile`.

## Inputs

`inputs.toml` lists every input with its SHA-256. A test fails when a file changes.

- `rule-examples`: one document for each category, holding the `bad` and `good` examples of its rules, one paragraph each. `golden.py inputs` regenerates them.
- `stacklok`: the permissions page before and after the rewrite (`slopvac-8l4.2.8`).
- `issues`: the regression samples for issues 252 to 257 (`slopvac-8l4.2.8`).
- `formats`: one sample each for MDX, HTML and reStructuredText, and code comments in Python and JavaScript. The runner stages each with its real extension, because the format comes from the extension, and runs the code files with `--mode code-comments`. The RST sample needs `rst2html` on `PATH`; the dev dependencies install docutils.

A corpus tier from the eval corpus (`slopvac-8l4.15.1`) is not part of the suite yet. A swap bead tracks it.

## Projection

The 2.x reference and a 3.0 binary both reduce their lint JSON to one projection (`project_report` in `scripts/contract/golden.py`). It holds the exit code and, for each document, the counts, score, gate, category scores, unchecked notes and findings. A document exits 2 when a check did not run, and otherwise 0 or 1 by its gate. A finding holds the rule id (`core:<category>.<id>`), category, severity, line, column, end position, byte span, matched text, message, AI signal and the replacement.

Keys that only 3.0 produces are outside the projection, because the reference cannot ground them: genre, contexts, config sources, provenance and pack. Their tests belong to the beads that add them.

2.x masks code spans and folds whitespace in `matched_text`. For those findings the end position cannot be proved from the source, so the golden keeps line and column and sets `end_line`, `end_column` and `span` to `null`. The check ignores the end position of such a finding in the candidate.

## Run records

Each run of a binary has a process exit status and a summary that the document gates do not determine. The runner groups the inputs by surface and format, runs each group once, and records both for each group in `runs/<profile>.json`. A check compares them apart from the documents, so a binary that changes only the summary score or only its exit status fails.

## Pin

`pin.toml` records the reference: the package version, the last commit that changed its sources, a hash of its source and rule data, and the Vale version. `record` refuses to run when the checkout does not match the pin. The pin test fails when the reference changes. A fix to rule data lands in the Python package first, then `golden.py pin` moves the pin, and then the golden is recorded again. The commit is compared only in a full clone, because the source hash proves the same thing.

## Approved changes

A golden file changes only with an approved reason. Record the reason in the pull request that changes the file, name the rules and inputs it touches, and get a maintainer's approval before merge. A mechanical re-record, such as a pin update after a rule fix, lists the changed findings in the pull request.

Policy deltas are different: each entry in `policy/deltas.toml` needs `reason` and `approved_by`, and a test rejects an entry without them. A delta names its finding by rule id, line and column. A delta that changes a gate also changes the expected exit status, and one that moves a run summary records it in a `run` table.

## Run

```sh
python scripts/contract/golden.py record --reference "<2.x slopvac>"   # runs with --flavor 2x
python scripts/contract/golden.py check  --candidate "<3.0 slopvac>" --mode parity
python scripts/contract/golden.py check  --candidate "<3.0 slopvac>" --mode preserve
python scripts/contract/golden.py check  --candidate "<3.0 slopvac>" --mode policy
python scripts/contract/golden.py diff   --a "<binary>" --b "<binary>" --mode parity
```

`diff` compares two binaries with each other, with no golden file. A command is a program prefix. The runner appends `lint <files> --config <config> --profile <profile> --format json`.
