# Lint report schema

`slopvac lint --format json` writes one report that validates against
[`schemas/v3/lint-report.schema.json`](../../schemas/v3/lint-report.schema.json). The
bead is slopvac-8l4.2.2. This page states what the schema cannot: where each 2.x key
went, and the rules a consumer relies on. Field descriptions live in the schema.

## Read a report

- `schema_version` is 3. Lint JSON jumps from 1 to 3, so no report carries 2.
- Consumers must branch on `notices[].code` and `unchecked[].code`, never on the message
  text. A code keeps its meaning for the whole 3.x line. Codes are kebab-case.
- `slopvac_version` names the tool build. It does not signal a schema change.
- The schema is strict, because it is the producer contract. Slopvac writes only reports
  that validate, and its own tests check that. A consumer must ignore unknown fields, as
  [the field policy](policies.md) requires, so a later 3.x release can add keys. Keys that
  start with `x-` are reserved for extensions and have no meaning.
- Minor releases add keys and codes only. A removal or a type change waits for 4.0.
- `line`, `column`, `end_line` and `end_column` are the display position. `span` is the
  byte range, from [`Span`](../../schemas/v3/common.schema.json#/$defs/Span). A finding
  with no source range, such as a document-wide metric, omits `span`, `end_line` and
  `end_column` together.
- `fix.edits` hold [`Edit`](../../schemas/v3/common.schema.json#/$defs/Edit) values.
  Their byte ranges address the original text and never overlap.
- `gate` is the verdict for one document under its profile. Exit codes still come from
  the run: 0 passes, 1 means that a gate failed, 2 means that the run was incomplete.

## New in v3

These keys have no 2.x counterpart.

- Root: `notices`, replacing `notes`.
- Document: `genre` with its `reasons`, `contexts_seen`, `config_sources`
  ([`ConfigSource`](../../schemas/v3/common.schema.json#/$defs/ConfigSource)) and `gate`.
- Finding: `pack`, `span`, `end_line`, `contexts`, `fix` and `provenance`
  ([`RuleProvenance`](../../schemas/v3/common.schema.json#/$defs/RuleProvenance)).

`contexts` names the text context a match sits in. Comment text in a source file carries
the tag `code-comment`, singular like `prose` and `code-span`. The input mode
`code-comments` is a different thing: it selects the input surface (`--mode`, and a rule's
`applies_to.modes` in [`rule.schema.json`](../../schemas/v3/rule.schema.json)). The mode keeps
its plural spelling because it is a CLI and config value. The two spellings differ on
purpose, and no code maps one to the other.

## 2.x key dispositions

A 2.x key is `preserve`d, `rename`d or `remove`d. 3.0 has no deprecation window and no
compatibility mode, so a renamed or removed key is gone at release. Run
`tests/contract/test_lint_report_2x_keys.py` to check that this table matches the 2.x
models in `slopvac.report` and `slopvac.model`.

| 2.x key | Fate | v3 |
| --- | --- | --- |
| `LintReport.schema_version` | preserve | The value moves from 1 to 3. A reader that pins 1 rejects a v3 report, which is intended. |
| `LintReport.version` | rename | Becomes `slopvac_version`, so no one reads it as a schema version. |
| `LintReport.summary` | preserve | Unchanged. |
| `LintReport.documents` | preserve | Unchanged. |
| `LintReport.notes` | rename | Becomes `notices`. Each entry changes from a string to a Notice with a stable `code`. The key is always present, where 2.x omitted `notes` when empty. |
| `RunSummary.documents` | preserve | Unchanged. |
| `RunSummary.words` | preserve | Unchanged. |
| `RunSummary.findings` | preserve | Unchanged. |
| `RunSummary.errors` | preserve | Unchanged. |
| `RunSummary.warnings` | preserve | Unchanged. |
| `RunSummary.suggestions` | preserve | Unchanged. |
| `RunSummary.per_100_words` | preserve | Unchanged. |
| `RunSummary.score` | preserve | Unchanged. |
| `RunSummary.passed` | preserve | Unchanged. |
| `RunSummary.categories` | preserve | Unchanged. |
| `RunSummary.ai_register` | preserve | Unchanged. |
| `RunSummary.prose` | preserve | Unchanged. |
| `CategorySummary.category` | preserve | Unchanged. |
| `CategorySummary.findings` | preserve | Unchanged. |
| `CategorySummary.errors` | preserve | Unchanged. |
| `CategorySummary.warnings` | preserve | Unchanged. |
| `CategorySummary.suggestions` | preserve | Unchanged. |
| `CategorySummary.per_100_words` | preserve | Unchanged. |
| `CategorySummary.score` | preserve | Unchanged. |
| `DocumentScore.path` | preserve | Unchanged. |
| `DocumentScore.profile` | preserve | Unchanged. |
| `DocumentScore.words` | preserve | Unchanged. |
| `DocumentScore.sentences` | preserve | Unchanged. |
| `DocumentScore.paragraphs` | preserve | Unchanged. |
| `DocumentScore.findings` | preserve | Unchanged. |
| `DocumentScore.categories` | preserve | Unchanged. |
| `DocumentScore.total_findings` | preserve | Unchanged. |
| `DocumentScore.errors` | preserve | Unchanged. |
| `DocumentScore.warnings` | preserve | Unchanged. |
| `DocumentScore.suggestions` | preserve | Unchanged. |
| `DocumentScore.per_100_words` | preserve | Unchanged. |
| `DocumentScore.score` | preserve | Unchanged. |
| `DocumentScore.ai_register` | preserve | Unchanged. |
| `DocumentScore.prose` | preserve | Unchanged. |
| `DocumentScore.passed` | rename | Becomes `gate.passed`. |
| `DocumentScore.failure_reasons` | rename | Becomes `gate.reasons`. A failed gate lists at least one reason. A passed gate lists none. |
| `DocumentScore.unchecked` | preserve | Same key. Entries change from strings to Notices with a stable `code`. |
| `CategoryScore.category` | preserve | Unchanged. `budget` stays nullable. |
| `CategoryScore.findings` | preserve | Unchanged. `budget` stays nullable. |
| `CategoryScore.errors` | preserve | Unchanged. `budget` stays nullable. |
| `CategoryScore.warnings` | preserve | Unchanged. `budget` stays nullable. |
| `CategoryScore.suggestions` | preserve | Unchanged. `budget` stays nullable. |
| `CategoryScore.per_100_words` | preserve | Unchanged. `budget` stays nullable. |
| `CategoryScore.gating_per_100_words` | preserve | Unchanged. `budget` stays nullable. |
| `CategoryScore.budget` | preserve | Unchanged. `budget` stays nullable. |
| `CategoryScore.score` | preserve | Unchanged. `budget` stays nullable. |
| `CategoryScore.over_budget` | preserve | Unchanged. `budget` stays nullable. |
| `Finding.path` | remove | Redundant with the owning document's `path`. A consumer that needs it reads the parent document. |
| `Finding.line` | preserve | Same keys. `column` and `end_column` now follow the position rules of slopvac-8l4.2.11. `end_column` is present only with `span`. |
| `Finding.column` | preserve | Same keys. `column` and `end_column` now follow the position rules of slopvac-8l4.2.11. `end_column` is present only with `span`. |
| `Finding.end_column` | preserve | Same keys. `column` and `end_column` now follow the position rules of slopvac-8l4.2.11. `end_column` is present only with `span`. |
| `Finding.rule_id` | preserve | Same key. The value uses the form `<pack>:<id>`, such as `core:orwell.not-un`. 2.x emitted `<category>.<id>`. |
| `Finding.category` | preserve | Unchanged. |
| `Finding.severity` | preserve | Unchanged. |
| `Finding.message` | preserve | Unchanged. |
| `Finding.ai_signal` | preserve | Unchanged. |
| `Finding.matched_text` | preserve | Unchanged. |
| `Finding.replacement` | rename | Becomes `fix.edits`, a list of byte-range edits instead of one replacement string. |
| `Finding.ste_ref` | remove | Rule metadata, not finding data. `slopvac explain` prints it, and `provenance` names the pack and source. |
| `Finding.orwell_ref` | remove | Rule metadata, not finding data. `slopvac explain` prints it, and `provenance` names the pack and source. |
| `AiRegisterSummary.findings` | preserve | Unchanged. |
| `AiRegisterSummary.errors` | preserve | Unchanged. |
| `AiRegisterSummary.warnings` | preserve | Unchanged. |
| `AiRegisterSummary.per_100_words` | preserve | Unchanged. |

## Consumers to migrate

These 2.x readers and writers use the old keys. The 3.0 output beads own the move:
slopvac-8l4.12.2 for the formats and slopvac-8l4.13.6 for the action.

- `action_entry.py` reads the JSON report. Owner: slopvac-8l4.13.6.
- The SARIF builder in `report.py`. Owner: slopvac-8l4.12.2.
- `html.py`. Owner: slopvac-8l4.12.2.
- The annotation output in `pipeline.py`. Owner: slopvac-8l4.12.2.

## Conformance

`schemas/v3/conformance/lint-report/` holds the corpus. `valid/multi-document.json` is a
realistic three-document report. Each file under `invalid/` fails at the pointer named in
`cases.toml`. The corpus is the snapshot: schemas generated from the Rust types must
accept and reject the same files.
