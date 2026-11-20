# Lint report schema

`slopvac lint --format json` writes one report that validates against
[`schemas/v3/lint-report.schema.json`](../../schemas/v3/lint-report.schema.json). The
bead is slopvac-8l4.2.2. This page states what the schema cannot: where each 2.x key
went, and the rules a consumer relies on. Field descriptions live in the schema.

## Read a report

- `schema_version` is 3. Lint JSON jumps from 1 to 3, so no report carries 2.
- Consumers must branch on `notices[].code` and `unchecked[].code`, never on the message
  text. A code keeps its meaning for the whole 3.x line. Codes are kebab-case.
- Extractors share four codes: `extract-unsupported`, `extract-malformed`, `extract-budget`
  and `extract-output-invalid`. Unsupported content inside a supported input, such as a
  Mermaid `gitGraph` block or a PlantUML mind map, is a root `notices` entry with the code
  `extract-unsupported` and level `info`. The run stays complete and exits 0. Malformed
  content (`extract-malformed`) and content over a limit (`extract-budget`) are `unchecked`
  entries at level `warning`. They make the run incomplete and exit 2.
- An unknown diagram type in a recognized fence, such as `gitGraph` or `@startmindmap`, is
  unsupported. A broken diagram of a type slopvac reads, such as an unclosed `@startuml`
  block or Mermaid front matter that never closes, is `extract-malformed`.
- Content that slopvac skips counts toward no score and no gate. The report shows that it
  was skipped, through the `extract-unsupported` notice.
- Three cases exit 2 although the input may be valid. A whole input that the user names in a
  format slopvac does not read, such as EPUB or PDF, is an error. An encrypted or
  password-protected container is an error. A file whose name promises source but holds
  none, such as `.drawio.png`, `.excalidraw.png` or `.drawio.svg`, is `binary-no-source` for
  a binary file and `extract-malformed` with the reason `no diagram source` for a
  diagram file. A plain PNG or SVG with no source is not an error. It is info and exits 0.
- `extract-output-invalid` belongs to first-party extractors only. The result of a failed
  third-party plugin is `plugin-output-invalid`.
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
  ([`RuleProvenance`](../../schemas/v3/common.schema.json#/$defs/RuleProvenance)) and
  the optional `origin` (see [Finding origin](#finding-origin)).

## Notice data

A notice may carry `data`, a JSON object whose values may be any JSON value, nested objects and arrays included. A notice with no details
omits it. The schema does not constrain the keys. Each notice code documents its own
keys, and a consumer must ignore keys it does not know.

`extract-unsupported` has these keys:

| Key | Meaning |
| --- | --- |
| `data.document` | The path of the document that holds the skipped content, as shown in the report. |
| `data.kind` | What was skipped: the format of the diagram, such as `mermaid`, `plantuml`, `excalidraw`, `drawio`, `drawio-svg` or `drawio-png`. |

`extract-budget` has these keys:

| Key | Meaning |
| --- | --- |
| `data.budget` | Name of the cap that stopped extraction. |
| `data.limit` | The integer limit, in the unit of that cap. |

`data.budget` takes one of these values:

| Value | Unit of `data.limit` |
| --- | --- |
| `segments` | Count of segments. |
| `part` | Unicode code points. |
| `locator` | Unicode code points. |
| `text` | UTF-8 bytes of extracted text. |
| `in-flight` | Bytes. |
| `frame` | Bytes, the negotiated `maxMessageBytes`. |
| `fragments` | Count of extracted fragments. |
| `retained-bytes` | Bytes of fragment locations and converted text. |
| `inflated-bytes` | Bytes of a decompressed draw.io page. |
| `xml-nodes` | Count of XML nodes in one draw.io file. |

The list of budget names can grow in a later 3.x release, so a consumer must handle a
name it does not know. The plugin protocol's error `data` for the same code uses the
same keys.

`config-budget-exceeded` has these keys:

| Key | Meaning |
| --- | --- |
| `data.budget` | Name of the config cap that the load passed: `extends-depth`, `extends-files`, `extends-bytes`, `extends-applied-bytes` or `file-blocks`. |
| `data.limit` | The integer limit, in the unit of that cap. |
| `data.path` | The config file at which the budget tripped, relative to the root, or `<external>/<file name>` outside it (policies.md, sections 1.7 and 1.9). Never an absolute path. For the `extends-*` budgets it is the file whose `extends` entry passed the limit; for `extends-depth` that is the last file accepted, because its own entry would nest too deep. For `file-blocks` it is the file that holds the blocks. |

`config-unknown-rule` is a `warning` notice. It does not make the run incomplete and does not change its exit status. A config layer named a rule id that no rule has. This covers a rule that 3.0 removed. The notice has one entry for each place that names the id. Its `path` is the config file, relative to the root, or `<external>/<file name>` outside it. The notice has these keys:

| Key | Meaning |
| --- | --- |
| `data.rule` | The id as the config wrote it, qualified or bare. |
| `data.line` | The one-based line of the key. Absent without a line. |
| `data.column` | The one-based column of the key. Absent without a column. |

A notice whose code this section does not list may still carry `data`. A consumer reads only the keys it knows and ignores the rest.

## Finding origin

A container document, such as a Word file or a draw.io diagram, is linted as a list of
extracted text segments. `origin` names the segment that holds a match. It is optional
and absent for plain text files. The bead is slopvac-8l4.2.29.

| Key | Meaning |
| --- | --- |
| `locator` | Percent-escaped address of the segment in the container. |
| `label` | Display name for people. Never an identity. |
| `segment` | Segment id, unique within the document. |
| `range` | Match range as UTF-8 byte offsets into the segment text, `[start, end)`. |

`range` uses the closed `SegmentRange` shape, not the common `Span`. A `Span` addresses
the original file bytes, and a segment range does not. `SegmentRange` takes no `x-` keys.
`start` is at most `end`, and both fall on code point boundaries of the segment text.
`matched_text` equals the segment text inside `range`.

### Positions of a finding with an origin

- `line`, `column`, `end_line` and `end_column` count in the segment text, not in the
  container. They follow the rules in [positions](positions.md) with the segment text as
  the file.
- `span` is optional. When present it is the physical byte range in the container, and
  it can be wider than the match, for example the whole entry when the container
  compresses it. A consumer must not read it as the match. The existing rule still ties
  `span` to `end_line` and `end_column`.
- A finding with an origin carries no `fix`. The schema rejects a finding that has both,
  and the `slopvac-model` type refuses to decode or serialize one. An edit addresses
  container bytes, which the linter cannot rewrite. See
  [Binary hosts](positions.md#binary-hosts).

### Locator

- A locator is opaque. A consumer compares two locators byte for byte after JSON
  decoding, and treats each as a whole string without parsing, normalizing or case-folding.
- Escaping: these characters are written as `%XX` for each of their UTF-8 bytes, with
  uppercase hex: `%`, controls (U+0000 to U+001F, U+007F to U+009F), U+2028, U+2029 and
  bidirectional overrides (U+202A to U+202E, U+2066 to U+2069). Every other character,
  including `/`, `#`, `[`, `]`, `@` and non-ASCII text, appears as is. A `%` in a locator
  is always the start of a `%XX` escape, so the schema rejects `%zz` and `%0a`.
- Length: at most 512 code points. A producer that needs more shortens the path by using
  a stable id the format provides, and never truncates.
- Uniqueness: within one document, a `locator` identifies one `segment` and a `segment`
  has one `locator`. Findings in the same segment share both and differ in `range`. Each pair of
  segments has distinct locators and distinct ids. JSON Schema cannot check this across findings, so the
  producer validator of slopvac-8l4.20.1 enforces it, and a consumer may rely on it.
- The `label` holds at most 256 code points, with the same forbidden characters but no
  escaping. A segment id matches `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}(?![\s\S])`.

### Stability

Take the same container bytes, the same extractor and the same `schema_version`. A
document then yields the same segments, with the same locators, segment ids and ranges,
on every run and platform and in any order of work.

A producer prefers an id that the format stores, such as a Word `w14:paraId`, over a
position. The locator grammar of a format is frozen for the whole 3.x line. A change
waits for 4.0.

A locator can differ between two versions of the container. An example is a paragraph
inserted before a position-based locator.

### SARIF

[positions.md](positions.md#binary-hosts) is the authority for the rendered forms. A SARIF
result for a finding with an origin gets:

- `locations[0].physicalLocation.artifactLocation.uri` for the container file, with no
  `region`. A region would claim a position in the artifact bytes, and the segment-local
  line and column are not one.
- `locations[0].logicalLocations[0].fullyQualifiedName` set to the escaped `locator`,
  and `name` set to `label`. Its `kind` is `resource` for a container part and `element`
  only for an XML element.
- `locations[0].properties["slopvac/local"]` holding the local start and end as a pair
  of `LineCol` values under `start` and `end`, plus `"unit": "code-point"` and
  `"base": 1`. Lines and columns are 1-based, and the column counts Unicode code points
  from the segment line start, as in [positions](positions.md#binary-hosts). They are not
  UTF-16 code units, whatever `columnKind` a SARIF region would use.
- `message.text` repeating the text form `path!locator:line:col`. The renderer encodes
  `%` as `%25` and `!` as `%21` in the path and in the locator. A reader takes `line` and
  `col` from the right, splits the rest at the first `!`, then decodes both parts.

The `segment` id and the byte `range` stay in the JSON report. GitHub code scanning can
show no line annotation for these alerts, because the location has no region.

### Fingerprints

A fingerprint for a finding with an origin hashes the rule id, `locator`, the start and
end of `range`, and the normalized `matched_text`. It leaves out `span` and every position in
the container. Moving a segment inside the container, or recompressing it, keeps the
fingerprint, and editing the segment text before the match changes `range` and the
fingerprint, as it does for plain text. The renderers and fingerprints are slopvac-8l4.20.12.

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
| `Finding.rule_id` | preserve | Same key. The value uses the form `<plugin>:<id>`, such as `core:orwell.not-un`. 2.x emitted `<category>.<id>`. |
| `Finding.category` | preserve | Unchanged. |
| `Finding.severity` | preserve | Unchanged. |
| `Finding.message` | preserve | Unchanged. |
| `Finding.ai_signal` | preserve | Unchanged. |
| `Finding.matched_text` | preserve | Unchanged. |
| `Finding.replacement` | rename | Becomes `fix.edits`, a list of byte-range edits instead of one replacement string. |
| `Finding.ste_ref` | remove | Rule metadata, not finding data. `slopvac explain` prints it, and `provenance` names the plugin and source. |
| `Finding.orwell_ref` | remove | Rule metadata, not finding data. `slopvac explain` prints it, and `provenance` names the plugin and source. |
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
