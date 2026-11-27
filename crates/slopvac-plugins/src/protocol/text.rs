//! The descriptions of the protocol schema, one constant per definition.
//!
//! Each constant is the `description` of the definition of the same name in
//! `schemas/v3/plugin-protocol.schema.json`. A type takes its constant through
//! `#[schemars(description = ...)]`, so the published text lives next to the code that the
//! schema describes.

/// Description of `RequestId`.
pub(crate) const REQUEST_ID: &str = "Request id. Integers from 1 to 2^53-1, unique among the requests of one connection and never reused after completion or cancellation. Strings, null, fractions and larger values are invalid in v1.";

/// Description of `HookKind`.
pub(crate) const HOOK_KIND: &str = "Hook point name. The v1 kinds are tagger, extractor, binary-extractor, reporter, fixer, scorer, provider and recipe. A reader ignores a kind it does not know, so the schema fixes only the form.";

/// Description of `HookId`.
pub(crate) const HOOK_ID: &str = "Plugin-local hook identifier. The host qualifies it with the plugin name. Each hook kind has its own id space.";

/// Description of `FeatureName`.
pub(crate) const FEATURE_NAME: &str = "Optional capability name. The v1 registry holds one name, document-path. A name with the prefix x- is a private extension and a reader ignores it unless it understands it.";

/// Description of `ProtocolVersion`.
pub(crate) const PROTOCOL_VERSION: &str = "Protocol version. Same value space as ApiVersion in common.schema.json.";

/// Description of `Name`.
pub(crate) const NAME: &str = "Program name and version of the host or the plugin. For logs only; never branch on it.";

/// Description of `ErrorObject`.
pub(crate) const ERROR_OBJECT: &str = "JSON-RPC error. code follows docs/contract/plugin-protocol.md, section Error codes. data.slopvac_code is the stable kebab-case string code that policies.md section 3.5 requires. A code from -32099 to -32000 is slopvac-defined.";

/// Description of `ErrorResponse`.
pub(crate) const ERROR_RESPONSE: &str = "Response that carries an error. id is null only when the sender could not read the request id, for example after a parse error.";

/// Description of `SuccessResponse`.
pub(crate) const SUCCESS_RESPONSE: &str = "Response that carries a result. The shape of result depends on the method of the request with the same id.";

/// Description of `Response`.
pub(crate) const RESPONSE: &str = "Success or error response.";

/// Description of `PluginDocument`.
pub(crate) const PLUGIN_DOCUMENT: &str = "One document in a batch. text is the full UTF-8 source. Every span in a result is a byte range into this text. path is present only when the plugin requested the document-path feature and the host granted it. It is relative to the project root, or <stdin>, or <external>/<file name>. It is never absolute.";

/// Description of `Documents`.
pub(crate) const DOCUMENTS: &str = "Batch of documents for one request. The host splits a batch to stay under the limits agreed in initialize. Ids are unique within the array.";

/// Description of `PluginFinding`.
pub(crate) const PLUGIN_FINDING: &str = "A finding as passed to fixers and scorers. Spans index the text of the document that carries the finding.";

/// Description of `DocumentResult`.
pub(crate) const DOCUMENT_RESULT: &str = "Fields shared by every per-document result. document echoes a Document id. error marks only this document as failed; the host reports the run as incomplete (exit 2) and keeps the other documents.";

/// Description of `InitializeParams`.
pub(crate) const INITIALIZE_PARAMS: &str = "First request, from host to plugin. protocolVersion is the highest version the host speaks and minProtocolVersion the lowest (default 1). hooks lists the kinds the host can call. features lists optional capabilities the host offers. offline is true when sticky offline mode is on. limits are the host's ceilings.";

/// Description of `HookDecl`.
pub(crate) const HOOK_DECL: &str = "One hook a plugin implements. The host calls only declared hooks. A binary-extractor hook must list the formats it reads, at least one and without duplicates. For other kinds formats is an unknown field and a reader ignores it.";

/// Description of `InitializeResult`.
pub(crate) const INITIALIZE_RESULT: &str = "Reply to initialize. protocolVersion is the negotiated version: the highest version both sides support. It must lie in the host's range. capabilities repeat the pre-spawn declaration: the host compares them with the manifest and rejects the reply, with plugin-declaration-mismatch, when requiresNetwork differs, a hook is absent from the manifest, or a required feature is absent from it. features.required names features the plugin cannot work without; the host stops the load if it lacks one. limits.maxMessageBytes, maxBatchDocuments and maxInFlight may lower the host ceilings but not raise them; a higher value is plugin-output-invalid.";

/// Description of `CancelParams`.
pub(crate) const CANCEL_PARAMS: &str = "Parameters of $/cancelRequest. id names a request the sender no longer wants answered.";

/// Description of `Tag`.
pub(crate) const TAG: &str = "A context tag: kind, span, confidence and evidence, as the plan defines for taggers.";

/// Description of `TagParams`.
pub(crate) const TAG_PARAMS: &str = "tagger/tag. Run the named tagger over each document.";

/// Description of `TagResult`.
pub(crate) const TAG_RESULT: &str = "One entry per requested document. Tags may overlap and may arrive in any order.";

/// Description of `Segment`.
pub(crate) const SEGMENT: &str = "A region of prose that the extractor hands to the rules. span is the source range it came from. text is the converted text when it differs from the source bytes in span; a finding in converted text maps to the whole span. context labels the region, for example prose or code.";

/// Description of `ExtractParams`.
pub(crate) const EXTRACT_PARAMS: &str = "extractor/extract. Split each document into segments.";

/// Description of `ExtractResult`.
pub(crate) const EXTRACT_RESULT: &str = "One entry per requested document. Segments are in source order and do not overlap.";

/// Description of `BinaryFormat`.
pub(crate) const BINARY_FORMAT: &str = "Format id of a binary document, from the 3.0 list in docs/contract/plugin-protocol.md section 5.5.1. The enum is extensible (x-slopvac-extensible): a later release may add ids without a new protocol version. A plugin or reader that receives an id it does not know declines that document with binary-input-declined, reason unknown-format, and does not fail the request. Manifest validation applies the list strictly: the host refuses a manifest that declares an id outside its own list.";

/// Description of `BinaryDocument`.
pub(crate) const BINARY_DOCUMENT: &str = "One binary document in a batch. bytes is the whole file in standard base64 (RFC 4648 section 4) with padding and no line breaks; byte_length is the decoded length. format is the id the host detected. path follows PluginDocument.path and the document-path feature. Every BinarySpan in the result indexes the decoded bytes.";

/// Description of `BinarySpan`.
pub(crate) const BINARY_SPAN: &str = "Physical provenance of a segment: a half-open range [start, end) of byte offsets into the decoded bytes of the document, with start at most end and end at most byte_length. The host checks the bounds. It has no character-boundary rule, and unlike common Span it is an open object (plugin-protocol.md section 9). Matching and rendered positions never use it; they use offsets into the segment text.";

/// Description of `BinarySegment`.
pub(crate) const BINARY_SEGMENT: &str = "A region of prose that a binary extractor hands to the rules. part names the container member, locator the place inside it; together they identify the segment and are unique within one document result. Both hold 1 to 1024 characters with no control or bidirectional-control characters. The 1024-character bound is a fixed host cap: the host reports a longer value as extract-budget, and the pattern bound is a backstop for other readers. span is physical provenance only. text is the extracted UTF-8 prose that rules match; findings use offsets into it. context is a context tag. role is the structural role, from an extensible set (plugin-protocol.md section 5.5.3).";

/// Description of `BinaryUnsupported`.
pub(crate) const BINARY_UNSUPPORTED: &str = "Content that a binary extractor skipped inside a document it reads, such as an embedded object or a part type it does not handle. part names the container member and locator, when present, the place inside it; both follow the BinarySegment rules. media_type is the content type the container declares for it, when it declares one. The host reports each entry as an info notice extract-unsupported in the root notices of the lint report, and the run still exits 0 (plugin-protocol.md section 5.5.4). Entries count against the segments budget.";

/// Description of `BinaryBudget`.
pub(crate) const BINARY_BUDGET: &str = "The budget an extract-budget error names in data.budget. This def is the one list of allowed values; docs/contract/plugin-protocol.md section 5.5.5 gives each value its limit, which an extract-budget error carries in data.limit in the unit of that table. The enum is extensible (x-slopvac-extensible): a reader keeps a value it does not know.";

/// Description of `BinaryDeclineReason`.
pub(crate) const BINARY_DECLINE_REASON: &str = "Why a binary extractor declines a whole document with binary-input-declined (-32008), carried in data.reason. unknown-format: the format id is not one the extractor knows (plugin-protocol.md section 5.5.1). variant-unsupported: the format is one it lists, but this variant is one it does not handle. The enum is closed: any other value is a protocol violation (plugin-output-invalid). The host renders its own description for each reason and never shows the plugin's string. A new reason is a contract change.";

/// Description of `BinaryExtractParams`.
pub(crate) const BINARY_EXTRACT_PARAMS: &str = "binary-extractor/extract. Extract the prose of each binary document. The host admits a request by a length preflight before it reads or encodes the file, holds it against the in-flight budget, and sends it only when the exact serialized size fits the negotiated maxMessageBytes (plugin-protocol.md section 5.5.6).";

/// Description of `BinaryExtractResult`.
pub(crate) const BINARY_EXTRACT_RESULT: &str = "One entry per requested document. Each entry carries exactly one of outcome or error. outcome extracted carries one or more segments in reading order; empty and no-source carry none. An entry with an outcome may list skipped content in unsupported, which the host reports as extract-unsupported notices; an entry with an error may not. extract-unsupported is never a per-document error. A document the extractor declines, an encrypted container, a malformed file and a budget breach travel as a per-document error: binary-input-declined (-32008) with data.reason, binary-encrypted (-32011), extract-malformed (-32009) and extract-budget (-32010) with data.budget and data.limit. A reader never parses these fields from the message. The reply must fit maxMessageBytes; a document whose segments do not fit gets the error message-too-large instead.";

/// Description of `RenderParams`.
pub(crate) const RENDER_PARAMS: &str = "reporter/render. report is one lint report with schema_version 3. Its shape is the lint-report schema; this schema checks only the version so that the two schemas can evolve apart. options holds reporter-specific settings from the config.";

/// Description of `RenderResult`.
pub(crate) const RENDER_RESULT: &str = "Rendered text. The host writes it to stdout or to the file the user named. The host strips terminal control characters other than newline and tab before it writes to a terminal.";

/// Description of `FixParams`.
pub(crate) const FIX_PARAMS: &str = "fixer/fix. Propose edits for the selected findings of each document. findings holds only the findings the user selected to fix. They define the envelope of what edits may touch.";

/// Description of `FixResult`.
pub(crate) const FIX_RESULT: &str = "One entry per requested document. Each edit names the selected finding that authorizes it. The host rejects every edit of a document, and reports that document as failed with plugin-output-invalid, when any edit lies outside its finding span, names no valid finding, or overlaps another edit. v1 has no approval path for broader edits. Edits apply to the text the host sent.";

/// Description of `ScoreParams`.
pub(crate) const SCORE_PARAMS: &str = "scorer/score. Compute a score for each document from its findings. The scorer may also read the text.";

/// Description of `ScoreResult`.
pub(crate) const SCORE_RESULT: &str = "One entry per requested document. score is finite and at least 0. The scale belongs to the scorer; the host applies the gate thresholds from the config.";

/// Description of `CompleteParams`.
pub(crate) const COMPLETE_PARAMS: &str = "provider/complete. Ask a model provider about each item. task names the review type. The host sends only the text the task needs. model is the model name from the config. A provider that cannot use it answers with an error.";

/// Description of `CompleteResult`.
pub(crate) const COMPLETE_RESULT: &str = "One entry per requested item. model is the model name the provider reports. The host compares it with the author model, as policies.md section 1.5 requires.";

/// Description of `RecipeParams`.
pub(crate) const RECIPE_PARAMS: &str = "recipe/plan. Plan the files that set up one agent harness. The plugin writes nothing; the host applies the plan. options holds the flags of the setup command.";

/// Description of `RecipeFile`.
pub(crate) const RECIPE_FILE: &str = "One planned file operation. path is relative to the project root, with no .. segment, no drive letter and no backslash. write creates or replaces the file. merge replaces a block that carries a slopvac marker and creates the file otherwise. remove deletes the file or the block. content is required for write and merge.";

/// Description of `RecipeResult`.
pub(crate) const RECIPE_RESULT: &str = "The plan. The host shows it, checks every path against the project root after symlink resolution, and applies it only when the command allows writes.";

/// Description of `Call`.
pub(crate) const CALL: &str = "A request or notification, dispatched on method. A request carries an id and a notification does not. An unknown method is not a schema error: a reader answers -32601 to a request and drops a notification.";

/// Description of `PluginDeclaration`.
pub(crate) const PLUGIN_DECLARATION: &str = "Pre-spawn declaration. The plugin manifest embeds it, and the manifest is part of the content that `slopvac plugins trust` hashes (plugin-manifest schema, owner slopvac-8l4.2.4). The host decides whether to spawn from this declaration, never from the initialize reply. network true allows the plugin to open connections, and the host does not spawn it in offline mode. hooks lists every hook the plugin may offer. features lists the features it may require or request. The initialize reply must stay inside the declaration.";

/// Description of `FixEdit`.
pub(crate) const FIX_EDIT: &str = "An edit with the finding that authorizes it. finding is the zero-based index into the findings array of the same document in the request. The host accepts the edit only when span lies inside the span of that finding, endpoints included. A zero-width span is an insertion and follows the same rule.";

/// Description of the `requiresNetwork` member of the `initialize` reply.
pub(crate) const REQUIRES_NETWORK: &str = "Must equal PluginDeclaration.network. It informs and cannot grant anything.";
