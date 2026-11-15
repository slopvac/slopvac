# Position contract

This spec defines where a finding or a fix sits in a file. It covers source-byte spans,
line and column numbers, the mapping from extracted prose back to source bytes, and the
conversion to each output format and to LSP. It owns the semantics of three shared
shapes: [`Span`](../../schemas/v3/common.schema.json#/$defs/Span),
[`LineCol`](../../schemas/v3/common.schema.json#/$defs/LineCol) and
[`Edit`](../../schemas/v3/common.schema.json#/$defs/Edit). Bead: slopvac-8l4.2.11.

The words MUST, SHOULD and MAY carry their RFC 2119 meaning.

## One coordinate system

A **span** is the only stored position. Everything else derives from a span and the
original file bytes:

- **Byte offsets** into the original file are the canonical form. Rules, parsers, fixes and
  the plugin protocol exchange spans.
- **Line and column** are display coordinates. The linter computes them from a span when
  it renders output. No component stores a column as its source of truth.

A producer that has only line and column, such as an external tool that reports a
column, MUST convert to a span before it hands the position to the rest of slopvac.

## Source model

The source is the byte sequence that the user would see on disk, or the buffer text in
LSP mode. Four rules fix how it is read.

1. **UTF-8 only.** A file that is not valid UTF-8 is an incomplete check (exit 2). The
   linter MUST NOT repair it with replacement characters. A repair changes byte lengths
   and shifts every later offset.
2. **BOM on disk.** A leading `EF BB BF` is part of the bytes, so offsets count it. It is not
   part of any line. A column on line 1 starts at 1 on the first byte after the BOM, in
   every unit. Offset 0 and offset 3 both map to line 1, column 1. SARIF requires this
   independence from the BOM
   ([SARIF 2.1.0 §3.30.2](https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/sarif-v2.1.0-errata01-os-complete.html)).
   LSP counts a received U+FEFF as a character. See the LSP section.
3. **Line terminators** are LF, CRLF and a CR that does not precede LF. CRLF is one
   terminator of two bytes. Nothing else ends a line: VT, FF, NEL (U+0085), U+2028 and
   U+2029 stay inside their line. This matches LSP's end-of-line list (`\n`, `\r\n`, `\r`,
   [LSP 3.17 text documents](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocuments))
   and CommonMark. A file that ends with a terminator has a final empty line. A position
   at the end of such a file is column 1 of the next line.
4. **No newline translation.** Offsets address the bytes as stored. A reader MUST NOT
   convert CRLF to LF before it computes offsets.

### Valid offsets

An offset is valid when all of these hold:

- It is at least 0 and at most the file length.
- It is not inside the BOM (offsets 1 and 2 of a BOM file).
- It sits on a code point boundary, so it never lands on a UTF-8 continuation byte.
- It does not fall between the CR and LF of one CRLF.

A combining mark is its own code point. An offset between a base letter and its mark is
valid. The contract does not know about grapheme clusters.

## Span

A [`Span`](../../schemas/v3/common.schema.json#/$defs/Span) is `[start, end)` over byte
offsets.

- **End is exclusive.** The byte at `end` is not part of the span.
- `start <= end`, and both are valid offsets. A span with `start == end` is empty and
  marks an insertion point or a position finding.
- A span MAY cross lines. A span that includes a terminator ends at column 1 of the next
  line.
- A span addresses source bytes. A projected range addresses extracted text, and the
  projection mapping converts one to the other.

JSON Schema cannot state `start <= end` or the boundary rules. The schema checks
integer and minimum only, so a consumer MUST validate the rest. The reference converter
and the vectors define the checks.

## Line and column

A [`LineCol`](../../schemas/v3/common.schema.json#/$defs/LineCol) has a 1-based `line`
and a 1-based `column`.

The **column** counts the code points from the start of the line to the position, plus
one. The count starts after the BOM on line 1. A tab is one column. Nothing expands tabs
or measures display width. A combining mark is one column.

The default unit is the Unicode code point because it is what a person counts in the
source, it needs no encoding choice, and it is the unit that Rust `char` iteration yields.
A format that needs another unit converts from the span. The unit table lists them.

| Unit | Counts | `a😀b`: column of `b` |
| --- | --- | --- |
| `utf8` | UTF-8 bytes | 6 |
| `utf16` | UTF-16 code units | 4 |
| `utf32` | Unicode code points | 3 |

## Projection mapping

Rules match extracted prose, not source bytes. A Markdown parser drops emphasis markers,
decodes entities and joins soft-wrapped lines, so a match offset in the prose text is not a
source offset. The projection carries the mapping back.

### Model

A projection is an ordered list of **tokens**. Each token has projected text and a source
range `[start, end)` in bytes. Projected offsets count code points of the concatenated token
text. Source ranges ascend and never overlap. Gaps between them are source the parser
dropped: markup, tags, delimiters and line terminators.

| Kind | Projected text | Source range |
| --- | --- | --- |
| `identity` | The source characters themselves | Exact. Each code point maps to its own bytes |
| `entity` | The decoded entity | The whole entity, such as `&amp;` |
| `backslash` | The escaped character | The backslash and the character |
| `opaque` | Text a syntax rule produced, such as a JSX string expression | The whole construct |
| `collapse` | One space for a whitespace run | The whole run, newlines included |
| `masked` | A placeholder for text that rules must not read, such as inline code | The whole masked construct |
| `wrap` | One synthetic space for a soft wrap | Empty, at the end of the previous real token |

All kinds except `wrap` are **real**: they have a non-empty source range. Every projected
point of a non-identity token maps to the whole token range. `wrap` is the only
**synthetic** kind. Parsers MAY add kinds in a minor release, and each new kind MUST say
whether it is real.

### From a projected range to a span

To map a projected range `[ps, pe)` to a span:

1. Drop synthetic points from both edges of the range.
2. If real points remain, the span starts at the smallest source start and ends at the
   largest source end among them. Interior synthetic points and dropped bytes fall inside
   the span.
3. If none remain, the span is empty at the source start of the point at `ps`.

An empty range follows rule 3. The point at the end of the text uses the source end of the
last token.

Three consequences follow:

- **A span covers a whole entity, escape or other multi-byte token.** A range that ends
  inside `&fjlig;` widens to the whole entity.
- **A match across a soft wrap includes the line terminator.** For `foo⏎bar` a match on
  `foo bar` spans all seven bytes, or eight with CRLF. A match that ends at the synthetic
  space ends at the real text before it.
- **Markup between matched characters falls inside the span.** A match on `a b` in
  `a <em>b</em>` spans through the tag.

### Per format

- **Markdown and MDX.** Entities, backslash escapes and MDX escaped braces, such as `\{`, are
  tokens. A JSX string expression that renders text is `opaque`. Soft wraps are `wrap`.
  Inline code is `masked`.
- **HTML.** Entities are `entity`. Tags and comments are gaps. A run of whitespace, with
  or without newlines, is one `collapse` token.
- **reStructuredText.** The core parses the document with docutils. docutils records the
  source lines of every block and keeps the source text of every inline construct, and the
  core places each node's words inside that node's own lines, never elsewhere in the file.
  Backslash escapes are `backslash`. Emphasis, strong, link and role markers are gaps. RST
  has no HTML entities. An inline literal is `masked`, and the whitespace between words is
  `collapse` when the source has it there. Positions are byte exact for every word that is
  found inside its node. When a node's words cannot be matched, its prose is kept and maps
  to the node's whole trimmed lines (`opaque`), so the position is line-precise, and the
  document carries an `info` notice. A node with no usable source lines makes the check
  incomplete. docutils 0.21 puts the line of some directive output one line late, so the
  mapper retries with one line of slack before it falls back.
- **Code comments.** The delimiters (`//`, `#`, `/*`, `*/`, a leading `*`) are gaps.
  Consecutive comment lines join with `wrap`. Comment text is not decoded.

### Fixes

An [`Edit`](../../schemas/v3/common.schema.json#/$defs/Edit) addresses source bytes. A fix
for a match inside a decoded token replaces the whole token, because the span covers it.
The `text` is source text and goes into the file verbatim. The rule or the fix generator
escapes it for the host syntax. A fix that cannot be written as source text, such as one
that needs an entity inside a code span, MUST be suppressed instead of emitted.

## Edit

An edit set belongs to one finding, or to one file for `--fix`. It applies atomically.

- Every span uses the **original** offsets. Order in the array does not shift offsets.
- Sort by `(start, end)`, and keep array order for equal pairs.
- Two edits **overlap** when one starts before the previous edit ends. A zero-width edit
  strictly inside another span overlaps it. Edits that touch at one offset do not overlap.
- Zero-width edits at one offset apply in array order.
- A zero-width edit at the start of a non-empty edit sorts first and does not overlap it.
- An overlapping set is invalid. The linter rejects the whole set and applies none of it.
- Replacement text is written verbatim. A fix MUST NOT rely on CRLF conversion, and it
  keeps the file's terminator style by writing the same bytes it replaced.
- The BOM is outside every edit that starts at or after offset 3, so it survives.

## Output formats

Every format derives from the span and the original bytes. The vectors pin each row.

| Format | Line | Column unit | End | Notes |
| --- | --- | --- | --- | --- |
| text | 1-based | code points | not shown | `path:line:col` |
| json | 1-based | code points | exclusive | `span` in bytes, plus `start` and `end` as `LineCol` |
| SARIF | 1-based | UTF-16 code units | exclusive | `startLine`, `startColumn`, `endLine`, `endColumn`, with `columnKind` set to `utf16CodeUnits` |
| rdjsonl | 1-based | UTF-8 bytes | exclusive | `range.start`, `range.end`; suggestions carry the same range |
| codeclimate | 1-based | code points | inclusive | `lines` and `positions` |
| junit | 1-based | code points | not shown | `line:col` in the failure text |
| checkstyle | 1-based | code points | not shown | `line` and `column` on `<error>` |
| markdown, html | 1-based | code points | not shown | `line:col` |
| github | 1-based | code points | not shown | `file`, `line`, `col` annotation |
| LSP | 0-based | negotiated | exclusive | see the LSP section |

An empty span renders with equal start and end wherever the format has an end.

### SARIF

[SARIF 2.1.0 §3.14.27](https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/sarif-v2.1.0-errata01-os-complete.html)
requires `columnKind` on a run that has results over text artifacts. It allows
`utf16CodeUnits` and `unicodeCodePoints`. `utf32CodeUnits` is not a legal value.

**Decision: slopvac emits `utf16CodeUnits`.** Both values are valid. A consumer that does
not support the declared kind MUST recompute columns, and an unreliable recompute is
the failure to avoid. GitHub's SARIF support page shows `columnKind` set to `utf16CodeUnits`, and JavaScript-based viewers index strings in UTF-16. Emitting the unit the main
consumers use makes the declared unit and the number agree even when a consumer ignores
`columnKind`. The cost is a conversion from the span for any line that has a supplementary
code point.

`startColumn` and `endColumn` are 1-based. `endColumn` is exclusive: the region does not
include the character at `endColumn` (§3.30.8). A region that includes a line's terminator
ends at column 1 of the next line (§3.30.2), which is what a span does. Emit `endLine` always, so a multi-line
region is not lost. A BOM never changes a column.

A SARIF `fix` replacement uses a text region with the same conversion as the result
location. An insertion has equal start and end.

### rdjsonl

The reviewdog format defines `Position.column` as a 1-based count of UTF-8 bytes, for
example the column of `b` in `a𐐀b` is 6
([reviewdog.proto](https://github.com/reviewdog/reviewdog/blob/master/proto/rdf/reviewdog.proto)).
The range end is exclusive, and an end of column 1 on the next line includes the line
terminator. A `Suggestion` carries a range and replacement text, so an `Edit` converts
by the same rule. The proto does not mention a BOM. Slopvac counts bytes from the first
byte after the BOM, as in every other unit.

### Code Climate

The [Code Climate spec](https://github.com/codeclimate/platform/blob/master/spec/analyzers/SPEC.md#locations)
has line-based and position-based locations. It says line ranges are **inclusive**, lines are
1-based, and positions refer to "specific characters". It does not name the column unit
or say whether a position end is inclusive.

**Decision:** emit both. `lines.begin` is the start line. `lines.end` is the line of the last
included character, so a span that ends at column 1 of the next line ends on the previous
line. The positions form uses code points. `positions.end` is the **last included
character**, matching the inclusive line range. For CRLF, that character is the CR. An
empty span has `end` equal to `begin`. The spec leaves the end open, so this decision needs
confirmation against a real consumer.

### JUnit

The JUnit XML schema that Jenkins ships
([junit-10.xsd](https://github.com/jenkinsci/xunit-plugin/blob/master/src/main/resources/org/jenkinsci/plugins/xunit/types/model/xsd/junit-10.xsd))
has no line or column attribute on `testcase`. Put `path:line:col` in the `failure`
message with code-point columns. Do not invent attributes.

### Checkstyle

A Checkstyle `<error>` has `line` and `column` attributes and no end
([XMLLogger](https://github.com/checkstyle/checkstyle/blob/master/src/main/java/com/puppycrawl/tools/checkstyle/XMLLogger.java)).
Emit the 1-based start with a code-point column. Checkstyle itself expands tabs by its
`tabWidth` setting. Slopvac does not. Consumers treat the column as a character index.

### GitHub annotations

The workflow command takes `file`, `line`, `col`, `endLine` and `endColumn`. The page says
`line` and `col` start at 1, and it describes `endColumn` only as the "end column number"
([GitHub workflow commands](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-commands)).
It does not name the column unit, the `endColumn` base or whether the end is inclusive. Slopvac emits
`line` and `col` only, with a code-point column, as 2.x does.

### Markdown and HTML reports

These render `line:col` for people. They use the same code-point column as text output.

## LSP

LSP 3.17 negotiates how `Position.character` counts. The client lists
`general.positionEncodings`. The server answers with `positionEncoding`. The kinds are
`utf-8`, `utf-16` and `utf-32`. UTF-16 is the default and every server MUST support it. A
missing list means `['utf-16']`
([initialize](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#initialize),
[Position](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#position)).

**Server choice.** Pick the first of `utf-8`, `utf-32`, `utf-16` that the client offers.
Otherwise answer `utf-16`. UTF-8 needs no conversion because spans are bytes. UTF-32 is
the code point unit of `LineCol`. UTF-16 is the fallback every client handles. An empty
list counts as `['utf-16']`.

`Position.line` and `Position.character` are 0-based, so convert with `line - 1` and
`character - 1` of the unit's column. A `Range` end is exclusive.

**Buffer coordinates.** LSP positions count the text the server received in `didOpen` or
`didChange`, not the file on disk. If that text starts with U+FEFF, the character is part
of line 1 and counts one column in every unit (three in `utf-8`). The disk rule that
excludes the BOM does not apply. So `Hi` after a BOM starts at character 1 in `utf-16`
and `utf-32` and at character 3 in `utf-8`. The server MUST compute LSP ranges from the
buffer text, never from disk columns. Byte offsets in a buffer also include the U+FEFF
bytes. A client that strips the BOM before it sends the buffer sends no U+FEFF, and the
server's offsets are then 3 less than the disk offsets. The server tracks which case it
holds per document.

An offset in a `TextEdit` or diagnostic converts through the negotiated encoding only. The
server MUST re-derive positions from its own copy of the document after each change
notification and MUST NOT cache them across versions.

## How 2.x works

Line numbers refer to files in `packages/slopvac-lint/src/slopvac/`.

- **No span on findings.** `Finding` has `line`, `column` and `end_column`
  (`model.py:238-254`). It has no end line and no byte offset.
- **Code point columns after newline translation.** The pipeline reads files with
  `Path.read_text(encoding="utf-8", errors="replace")` (`pipeline.py:315`). That call
  translates CRLF and CR to LF and turns invalid bytes into U+FFFD, which changes byte
  lengths. A BOM stays as U+FEFF, so line 1 columns are off by one against the 3.0 rule.
- **Projection.** `ProjectionMap` maps each projected code point to a raw byte range
  (`projection.py:32-117`). Entities and inline code are emitted with an empty raw range
  (`projection.py:317-322`, `projection.py:301-305`), so the entity's own bytes belong to no
  projected point and `slice_raw` returns nothing for it. `Block.position` turns the raw byte
  offset into line and column by decoding the prefix and counting `\n`
  (`analyze.py:409-422`).
- **Multi-line matches lose their end.** When the end falls on another line, the engine
  sets `end_column` to the end of the start line (`engine.py:878-879`).
- **SARIF declares the wrong unit.** `SarifRun.column_kind` defaults to `utf16CodeUnits`
  and also allows `utf32CodeUnits` (`report.py:249-251`). The values written are code
  points (`report.py:366-370`). `utf32CodeUnits` is not a SARIF value. The region has no
  `endLine` (`report.py:183-188`).
- **Fix offsets disagree with report offsets.** `apply_fixes` splits with
  `str.splitlines(keepends=True)` (`diff_scope.py:413-418`). That also splits on VT, FF,
  FS, GS, RS, NEL, U+2028 and U+2029, while `Block.position` counts only `\n`. A guard that
  compares the matched text (`diff_scope.py:430`) skips a mismatched fix, so the failure
  is a missed fix, not a corrupt one.
- **Vale columns pass through.** Vale findings take their column from Vale's span
  (`vale.py:296-318`) with no check against the source.
- **Formats.** 2.x has text, json, github, sarif and html. Both json and github use
  `finding.column`. rdjsonl, codeclimate, junit, checkstyle and markdown output are new in
  3.0.

### What changes in 3.0

- Findings and edits carry a `Span` in original-file bytes. Line and column derive from it.
- Files are read as bytes. Invalid UTF-8 is exit 2 and CRLF is kept.
- The BOM is outside line 1 columns.
- Decoded tokens keep their full source range.
- SARIF emits UTF-16 columns that match `columnKind`, plus `endLine`.
- Fixes use the same line table as reports, and CR and CRLF are terminators everywhere.
- Multi-line findings keep their end line.

The 2.x JSON `line` and `column` fields do not carry over. 3.0 invalidates 2.x outputs at
release, and `slopvac migrate` is the only bridge for configuration.

## Test vectors

[`schemas/v3/conformance/positions/vectors.json`](../../schemas/v3/conformance/positions/vectors.json)
holds language-neutral vectors. Documents are JSON strings with `\u` escapes, so the file
survives editors that normalize text. The sections are:

- `positions`: offset to line and column in `utf8`, `utf16` and `utf32`.
- `invalid_offsets`: offsets that MUST be rejected.
- `spans`: start and end positions per unit, and the payload of every output format except LSP.
- `invalid_spans`: spans that MUST be rejected.
- `edits` and `invalid_edits`: edit sets and their results.
- `projections`: token lists and the span each projected range maps to.
- `lsp_ranges`: LSP ranges per encoding, computed over the received buffer text. They include BOM regression vectors.
- `lsp_negotiation`: client offers and the encoding the server picks.

The documents cover ASCII, emoji, a ZWJ family sequence, supplementary-plane letters, combining marks, CJK, tabs,
CRLF, a lone CR, a BOM, characters that are not terminators, a final newline and an empty
file. The projection cases cover entities, escapes, escaped MDX, soft wraps, collapsed
whitespace, skipped tags, RST emphasis, comment lines and masked code.

The Python reference is
[`tests/contract/position_reference.py`](../../tests/contract/position_reference.py). The
test is `tests/contract/test_positions.py`. The Rust port reads the same file and MUST pass
every case. A projection vector tests the mapping algorithm and token rules. A parser's own
tests must show that it emits those tokens for real input.
