# Binary container inputs

Status: approved, 2026-11-16. Decision 27 of the [architecture record](0001-slopvac-3-architecture.md) records the choice. Beads epic `slopvac-8l4.20` tracks the work.

Slopvac 3.0 lints text inside Office and OpenDocument archives and inside PNG images that embed a diagram source. The design here is not a contract. The files under `docs/contract/` own the wire formats, the position rules and the threat model. Where this document and a contract file differ, the contract file wins, and the owner of the contract bead fixes this document.

## Scope

Supported formats:

| Family | Variants | Text that is linted |
|---|---|---|
| OOXML Word | `.docx`, `.docm`, `.dotx` | Body, comments, footnotes, endnotes, headers and footers, text boxes |
| OOXML PowerPoint | `.pptx`, `.pptm` | Slides, speaker notes, comments |
| OOXML Excel | `.xlsx`, `.xlsm` | Text cells, comments, headers and footers (see "xlsx") |
| OpenDocument | `.odt`, `.odp`, `.ods` | Body, annotations, `text:note`, master-page headers and footers |
| PNG with a diagram source | `.drawio.png`, `.excalidraw.png`, and PNG files named by an explicit include | draw.io, Excalidraw and PlantUML sources |

Macro and template variants (`docm`, `xlsm`, `pptm`, `dotx` and similar) are included and read-only. Slopvac skips macros and leaves these files unchanged.

Out of scope for 3.0:

- EPUB and PDF. The docs list both as unsupported.
- Mermaid, tldraw and Inkscape images. They embed no source in a PNG.
- Encrypted files. They produce an incomplete notice and exit 2.
- Nested archives and embedded objects.
- Binary input on stdin. The CLI rejects it with a usage error.

## Engine entry point

The engine gains a second entry point beside the text input:

- `BinaryInput { path, bytes, format }`, marked `#[non_exhaustive]`.
- `Engine::lint_binary(BinaryInput)`.
- `Engine::with_binary_extractors(...)`, which registers extractors the same way `with_diagram_checker` registers diagram checkers.
- `EngineError::Unsupported` for a format with no registered extractor.

`Input.source` stays `&str`. The engine lints each extracted segment as text through the normal pipeline, so rules, contexts, suppression and fixes keep one code path.

Detection reads the signature first: the PNG magic, or `PK\x03\x04` for a ZIP. A ZIP that holds `[Content_Types].xml` is OOXML. A ZIP whose first entry is a stored `mimetype` is ODF. The extension then picks the variant. If the signature and the extension disagree, the run is incomplete and exits 2.

## Extraction contract

One extraction contract serves two adapters. A native adapter calls first-party extractors in process. A wire adapter calls third-party extractors through the plugin host. Both adapters use one transport-independent validator.

A new plugin hook kind, `binary-extractor/extract`, carries the request. Section 9 of the plugin protocol allows additive hook kinds. The bytes travel inline as base64 instead of as a path. Inline bytes need no shared filesystem, leave no time-of-check gap, and let the host cache by content hash.

Each extractor returns a list of segments:

```text
Segment { part, locator, span, text, context, role }
```

- `part` names the container entry, for example `word/document.xml`.
- `locator` names the unit inside the part. Examples: `word/document.xml#w:p[@w14:paraId=...]`, `ppt/slides/slide3.xml#p:sp[@id=5]/a:p[2]`, `xl/worksheets/sheet1.xml#B4`, `content.xml#text:p[8]`, and `drawio/<page>/<role>[<id>]` for diagrams.
- `span` is the physical byte range of the segment in the container. It is provenance only.
- `text` is UTF-8 prose. The extractor emits it already decoded.
- `context` and `role` carry the same tags as text extractors.

The validator enforces, for both adapters:

- eligibility of the format and the segment fields;
- segment identity and order. Identities are unique and stable, and the order is document order;
- these distinct outcomes: valid-empty, no-source, unsupported, malformed and over-budget. A malformed payload in a named format is incomplete and exits 2. It is never a clean empty result;
- stable error codes;
- the limits in the "Limits" section.

Conformance tests also run the first-party extractors through the wire adapter, so a third party can test against the same cases.

The common `Span` in the plugin protocol schema is closed. The hook defines its own open local span type. Fields must not hide under `x-` keys.

### Transport limits

The inline base64 encoding sets a hard ceiling. A 12 MiB file encodes to exactly 16 MiB, which equals the default `maxMessageBytes` before any JSON. So admissibility is computed on the full serialized request against the negotiated limit, not on the file size. Replies are bounded the same way, because extracted text plus JSON escaping can exceed a frame. 3.0 has no chunking. A file that does not fit in one message is incomplete for plugin extractors. The 30 second hook timeout applies, and the reply counts against the run budget.

## Segment-local positions

Matching, contexts, exclusions and fingerprints use segment-local coordinates. Each segment is a small text document with its own line 1, column 1. The container byte range stays on the segment as provenance and is never used for matching.

This replaces an earlier rule that put every finding on line 1, with the column set to the container byte offset plus one and the span set to the whole entry. That rule made fingerprints depend on archive layout and made columns meaningless.

Consequences:

- Renderers print the locator plus a local `line:col`. `locate.rs` (`SourceIndex`) and the SARIF UTF-16 conversion change.
- SARIF results add `logicalLocations` that carry the locator.
- Code Climate fingerprints (`codeclimate.rs`) hash the locator and local range, not a container offset.
- Skip and exclusion logic (`skip.rs`) works in segment coordinates.
- `docs/contract/positions.md` gets a "Binary hosts" section and test vectors. The position contract owner writes them.

## Finding.origin

A finding from a binary host carries an optional `origin` structure:

```text
origin { locator, label, segment_id, local_range }
```

- `locator` is the segment locator above.
- `label` is a short human name, such as `Sheet1!B4` or `slide 3, notes`.
- `segment_id` is the stable segment identity from the validator.
- `local_range` is the range inside the segment text.

Origin strings come from file content, so they are untrusted. The contract defines escaping, uniqueness, a maximum length and stability across runs. Every renderer sanitises control characters before printing them. The lint report schema (`lint-report.schema.json`), SARIF and the fingerprints change together.

Text findings have no `origin`. Consumers must treat it as optional.

## Fix policy

`--fix` leaves binary hosts unchanged. A finding in a binary host reports its replacement text, but the fixer skips the file with `SkipReason::BinaryHost`. Rewriting a container would risk losing formatting, revision history, macros and signatures. A later release can add per-format writers.

## Format rules

### Tracked changes

Extraction follows the accepted view of a document:

- Exclude `w:del`, `w:delText`, `w:moveFrom` and `instrText`.
- Include `w:ins`, `w:moveTo` and field results.
- Handle deleted paragraph marks and deleted table rows.
- Exclude the ODF `text:tracked-changes` history.

A moved sentence must appear once in the output.

### xlsx

xlsx uses a cell-occurrence model. Each text cell becomes one segment, addressed by sheet and cell reference such as `Sheet1!B4`. Comments, headers and footers are separate segments.

The user decided these cases on 2026-11-16. Hidden sheets are skipped, because a reader does not see them. Formulas and their cached results are not linted.

The design also fixes these cases:

- `sharedStrings.xml` is never linted as standalone prose. A shared string is linted once per cell that references it.
- The number of occurrences is budgeted before allocation, because one shared string can fan out to millions of cells.

### PNG

PNG extraction reads the chunks that known tools write:

- draw.io: a `tEXt` chunk with keyword `mxfile`, whose value is URL-encoded XML. Inner pages are plain `mxGraphModel` or base64 of raw-deflated, URL-encoded XML.
- Excalidraw: a `tEXt` chunk with keyword `application/vnd.excalidraw+json`. The value is JSON with an `encoded` Latin-1 string, zlib-deflated when `compressed` is true.
- PlantUML: an `iTXt` chunk with keyword `plantuml`. Before about October 2018 PlantUML used `zTXt`, so both are read. Whether the value is raw source or PlantUML-encoded text is unverified and must be checked against a real sample fixture before implementation.

The walker checks CRCs and returns a fallible result with the five outcomes of the validator. The current draw.io code enables the DTD and swallows decode errors. Both change: the DTD is off and a malformed payload is incomplete.

## Routing

### CLI

Discovery matches `.docx` and the other container extensions by name. For PNG, discovery lints `.drawio.png`, `.excalidraw.png` and files an explicit include names. A bare `*.png` is not discovered, because sniffing every image would open files that rarely hold a source. Binary stdin is rejected.

### LSP

The LSP works on `&str` documents. A recognised binary URI gets one stable document-level diagnostic that says the file is unsupported in the editor, with no findings and no code actions. It is not a log line. The existing incomplete-document path in the analysis code already publishes diagnostics of that kind. The LSP diagnostic can land independently of the rest.

## Limits

First-party extractors enforce these limits. A breach makes the run incomplete and exits 2.

- Input file: 256 MiB.
- ZIP:
  - at most 10,000 entries, checked before the archive opens;
  - at most 64 MiB per entry, enforced by reading `limit + 1` bytes;
  - at most 256 MiB read in total;
  - a compression ratio of at most 100:1 once an entry passes a 100 KiB grace size;
  - no nested archives;
  - only known text parts and relationships are read;
  - entries with `..`, absolute paths, duplicate names or encryption are rejected.
- XML: DTD disabled everywhere, about 1M nodes, depth 256, and caps on attributes per element and namespaces, all checked before the DOM is built.
- PNG: signature and CRC checked, at most 10,000 chunks, at most 16 MiB per text chunk, at most 32 MiB inflated in total, and chunks after `IEND` ignored.
- Extracted text: at most 32 MiB. Plugin requests and replies must fit the negotiated `maxMessageBytes`.

Bounds apply before expensive work. The extractor reads the ZIP end-of-central-directory record itself and bounds the central-directory size and entry count before `ZipArchive::new`, which allocates headers when it opens. XML limits apply before `roxmltree` runs, because it reserves vectors from raw `<` and `=` counts and its attribute check is quadratic. A per-document and an aggregate memory budget cover concurrent runs.

First-party timeouts are cooperative. An extractor checks its deadline between parts, so one part can overrun the deadline by the time the extractor needs to read it.

## Libraries

The first-party extractors use `zip` and `roxmltree` (already a workspace dependency), plus a PNG chunk walker written in the repository. The crate research found:

- `zip` 9.6 has no read-side bomb guards. It offers `validate_header_consistency`, which the extractor must call.
- `roxmltree` 0.21 supports `allow_dtd = false` and an entity-depth limit, but has no depth option.
- `quick-xml` does not expand a DTD, but gives no node limit.
- The writer-centric DOCX crates (`docx-rs`, `docx-rust`, `ooxmlsdk`, `undoc`) give no byte spans and are immature or heavy.
- `calamine` reads spreadsheets but gives no byte spans.

A new crate, `slopvac-office`, holds the Office and ODF extractors. It needs a `Cargo.lock` entry in `extra-files` in `release-please-config.json`; `just sync-release-please` writes it.

## Threat rows

Each row needs a proving fixture. The threat model owner adds them to `docs/contract/threat-model.md`.

| Threat | Control |
|---|---|
| A plugin sees the whole container, including images, revision history and embedded objects | Document it. Plugins are not sandboxed in 3.0, so the install directory is only a working directory. Binary extractors need the same trust as code plugins |
| External relationships fetch remote content, or embedded content runs | Slopvac skips external relationships and embedded content |
| Parser disagreement over a ZIP | Reject duplicate names, overlapping entries and local/central header mismatch. Call `validate_header_consistency` |
| Locator or metadata injection through names, sheet names or labels | Sanitise every origin string before it reaches a renderer |
| Decompression bombs and ratio abuse | The ZIP and PNG limits above |
| Parser memory blow-up through attributes, namespaces or depth | Bound them before building the DOM |
| Aggregate and semantic expansion through shared-string references, segment counts or finding counts | Budget the counts before allocation |
| DTD and entity expansion | `allow_dtd = false` everywhere, including `drawio.rs` |
| A malformed payload hidden as a clean empty result | The five-outcome validator. Malformed exits 2 |

## Approved tradeoffs

The user approved all five recommendations on 2026-11-16:

1. First-party extractors run in process through the native adapter, with conformance tests through the wire adapter. The alternative was bundled subprocess plugins, which cost a spawn and base64 encoding per file. In process, an extractor gives up crash and memory isolation: an abort or panic in it affects the host process. Its deadline is cooperative and checked between parts, while the host cancels and kills a subprocess plugin.
2. PNG discovery uses `.drawio.png`, `.excalidraw.png` and explicit includes. The alternative was to sniff every PNG.
3. Binary input on stdin is rejected in 3.0.
4. Macro and template variants are included and read-only.
5. xlsx stays in 3.0 with the narrow cell model. The alternative was to drop it.

## Order of work

Hard edges, in order:

1. Contract semantics, positions, `Finding.origin` and threat sign-off.
2. Shared types and the validator.
3. The engine adapter and the wire adapter.
4. First-party formats.
5. Renderers, fingerprints and CLI routing.
6. End-to-end conformance.

The LSP diagnostic lands independently. Beads `slopvac-8l4.10.10`, `.10.12` and `.10.13` are superseded only after their acceptance coverage moves to the new beads.
