//! The `schemas/v3/conformance/plugin-protocol` corpus, one test per file, run through the
//! host's own decode and strict checks: `strict::parse`, `check_envelope`, the
//! `InitializeResult` and `PluginDeclaration` types, `parse_error_object` and the `hooks` result
//! parsers.
//!
//! Every `valid/` file must be accepted. Every `invalid/` file must be rejected, and the
//! rejection must name the field that `invalid/cases.toml` gives as the instance location.
//! Host errors carry no JSON Pointer, so the test checks that the diagnostic text contains the
//! field name at the pointer (see [`needle`]).
//!
//! Two kinds of file, as in `docs/contract/README.md`:
//! - `<Def>--<name>.json` is an instance of `#/$defs/<Def>`. [`DEFS`] maps each `<Def>` to the
//!   host entry that decodes it.
//! - Any other file is one whole JSON-RPC message, read as a frame body without framing.
//!
//! Two tables name files that the host does not decode, each by exact file name:
//! - [`HOST_BUILT`]: messages the host builds and sends. It never reads them, so it has nothing
//!   to accept or reject. The schema corpus run (`tests/contract/test_schemas.py`) covers them.
//! - [`PENDING_BINARY`]: binary-extractor files. The host has no binary extractor yet.
//!
//! The inventory tests check that every corpus file is decoded or listed in exactly one table,
//! and that every listed name exists. Every file outside the tables must behave as
//! `invalid/cases.toml` says. No case is marked expected-fail.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: a panic is a failed test"
)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;
use serde_json::Value;

use crate::PluginError;
use crate::hooks::{
    Finding, Span, match_entries, parse_completions, parse_edits, parse_error_object, parse_recipe,
    parse_rendered, parse_score, parse_segments, parse_tags,
};
use crate::host::{Envelope, check_envelope};
use crate::protocol::{InitializeResult, PluginDeclaration};
use crate::strict;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3/conformance/plugin-protocol")
}

fn cases() -> toml::Table {
    toml::from_str(&fs::read_to_string(corpus().join("invalid/cases.toml")).expect("cases.toml"))
        .expect("cases.toml parses")
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .expect("file name")
        .to_string_lossy()
        .into_owned()
}

/// The corpus directory (`valid` or `invalid`) that holds `path`.
fn dir_name(path: &Path) -> String {
    file_name(path.parent().expect("parent dir"))
}

/// How the host decodes one kind of instance.
#[derive(Clone, Copy, Debug)]
enum Decoder {
    /// `serde_json::from_value::<InitializeResult>`, as the handshake does.
    InitializeResult,
    /// `serde_json::from_value::<PluginDeclaration>`, as the manifest reader does.
    PluginDeclaration,
    /// `match_entries` on `document`, then `parse_error_object` or `parse_tags` per entry.
    TagResult,
    /// `match_entries` on `document`, then `parse_error_object` or `parse_segments`.
    ExtractResult,
    /// `match_entries` on `document`, then `parse_error_object` or `parse_edits`.
    FixResult,
    /// `match_entries` on `document`, then `parse_error_object` or `parse_score`.
    ScoreResult,
    /// `parse_completions`.
    CompleteResult,
    /// `parse_recipe`.
    RecipeResult,
    /// `parse_rendered`.
    RenderResult,
    /// The host builds this kind. Its files sit in [`HOST_BUILT`].
    HostBuilt,
    /// The host has no type for this kind yet. Its files sit in [`PENDING_BINARY`].
    Pending,
}

/// Each `#/$defs/<Def>` in the corpus and the host entry that decodes it.
const DEFS: &[(&str, Decoder)] = &[
    ("InitializeResult", Decoder::InitializeResult),
    ("PluginDeclaration", Decoder::PluginDeclaration),
    ("TagResult", Decoder::TagResult),
    ("ExtractResult", Decoder::ExtractResult),
    ("FixResult", Decoder::FixResult),
    ("ScoreResult", Decoder::ScoreResult),
    ("CompleteResult", Decoder::CompleteResult),
    ("RecipeResult", Decoder::RecipeResult),
    ("RenderResult", Decoder::RenderResult),
    // The host sends `initialize` params from a `json!` literal and never reads them back.
    ("InitializeParams", Decoder::HostBuilt),
    // The host has no binary-extractor hook yet.
    ("BinaryExtractResult", Decoder::Pending),
];

/// The result type of the request that a `response-*.json` file answers. The method comes from
/// the file name. `response-shutdown.json` answers `null`.
const RESPONSES: &[(&str, Option<Decoder>)] = &[
    ("response-initialize.json", Some(Decoder::InitializeResult)),
    ("response-shutdown.json", None),
    // Answers a `tagger/tag` request (id 2 in `request-tag.json`).
    (
        "response-unknown-result-fields.json",
        Some(Decoder::TagResult),
    ),
];

/// Host-built, not host-decoded: messages from the host to a plugin. The host builds and sends
/// them (`Plugin::call`, the handshake, `$/cancelRequest`) and has no reader for them, and the
/// outgoing types get no `Deserialize`. Each entry is `(directory, file name)`.
const HOST_BUILT: &[(&str, &str)] = &[
    ("valid", "InitializeParams--minimal-range.json"),
    ("valid", "InitializeParams--unknown-fields-ignored.json"),
    ("valid", "notification-cancel.json"),
    ("valid", "notification-exit.json"),
    ("valid", "notification-initialized-empty-params.json"),
    ("valid", "notification-initialized.json"),
    ("valid", "request-binary-extract-empty-file.json"),
    ("valid", "request-binary-extract-one-padding.json"),
    ("valid", "request-binary-extract-ppsm.json"),
    ("valid", "request-binary-extract-ppsx.json"),
    ("valid", "request-binary-extract.json"),
    ("valid", "request-complete.json"),
    ("valid", "request-extract.json"),
    ("valid", "request-fix.json"),
    ("valid", "request-initialize.json"),
    ("valid", "request-max-id.json"),
    ("valid", "request-recipe.json"),
    ("valid", "request-render.json"),
    ("valid", "request-score.json"),
    ("valid", "request-shutdown.json"),
    ("valid", "request-tag.json"),
    ("valid", "request-unknown-fields.json"),
    ("valid", "request-unknown-method.json"),
    ("invalid", "cancel-with-request-id.json"),
    ("invalid", "cancel-without-id.json"),
    ("invalid", "complete-unknown-task.json"),
    ("invalid", "complete-without-model.json"),
    ("invalid", "fix-bad-severity.json"),
    ("invalid", "fix-without-findings.json"),
    ("invalid", "initialize-feature-trailing-newline.json"),
    ("invalid", "initialize-feature-uppercase.json"),
    ("invalid", "initialize-missing-limits.json"),
    ("invalid", "initialize-missing-params.json"),
    ("invalid", "initialize-small-max-message.json"),
    ("invalid", "initialize-version-zero.json"),
    ("invalid", "render-report-wrong-version.json"),
    ("invalid", "tag-absolute-path.json"),
    ("invalid", "tag-drive-path.json"),
    ("invalid", "tag-duplicate-document.json"),
    ("invalid", "tag-empty-batch.json"),
    ("invalid", "tag-hook-id-trailing-newline.json"),
    ("invalid", "tag-hook-id-uppercase.json"),
    ("invalid", "tag-parent-path.json"),
    ("invalid", "tag-path-trailing-newline.json"),
];

/// `STUB(slopvac-agx)`: binary-extractor files that wait for the host's binary-extractor
/// adapter (plugin-protocol.md section 5.5). The swap removes this table and decodes the files
/// through the real types. Each entry is `(directory, file name)`.
const PENDING_BINARY: &[(&str, &str)] = &[
    ("valid", "BinaryExtractResult--budget-error.json"),
    ("valid", "BinaryExtractResult--declined.json"),
    ("valid", "BinaryExtractResult--encrypted.json"),
    ("valid", "BinaryExtractResult--extracted.json"),
    ("valid", "BinaryExtractResult--locator-1024.json"),
    (
        "valid",
        "BinaryExtractResult--open-span-and-unknown-role.json",
    ),
    ("valid", "BinaryExtractResult--outcomes.json"),
    ("valid", "BinaryExtractResult--unsupported-parts.json"),
    ("invalid", "binary-extract-bytes-line-break.json"),
    ("invalid", "binary-extract-bytes-trailing-lf.json"),
    ("invalid", "binary-extract-bytes-unpadded.json"),
    ("invalid", "binary-extract-bytes-urlsafe.json"),
    ("invalid", "binary-extract-empty-batch.json"),
    ("invalid", "binary-extract-format-trailing-lf.json"),
    ("invalid", "binary-extract-format-uppercase.json"),
    ("invalid", "binary-extract-negative-byte-length.json"),
    ("invalid", "binary-extract-path-trailing-dotdot.json"),
    ("invalid", "binary-extract-path-trailing-lf.json"),
    ("invalid", "binary-extract-text-document.json"),
    ("invalid", "binary-extract-without-byte-length.json"),
    ("invalid", "binary-extract-without-format.json"),
    ("invalid", "BinaryExtractResult--budget-as-outcome.json"),
    (
        "invalid",
        "BinaryExtractResult--budget-without-budget-name.json",
    ),
    ("invalid", "BinaryExtractResult--budget-without-limit.json"),
    (
        "invalid",
        "BinaryExtractResult--declined-unknown-reason.json",
    ),
    (
        "invalid",
        "BinaryExtractResult--declined-without-reason.json",
    ),
    ("invalid", "BinaryExtractResult--empty-with-segments.json"),
    ("invalid", "BinaryExtractResult--error-with-segments.json"),
    (
        "invalid",
        "BinaryExtractResult--error-with-unsupported.json",
    ),
    ("invalid", "BinaryExtractResult--extracted-no-segments.json"),
    (
        "invalid",
        "BinaryExtractResult--extracted-without-segments.json",
    ),
    (
        "invalid",
        "BinaryExtractResult--locator-1024-trailing-lf.json",
    ),
    (
        "invalid",
        "BinaryExtractResult--locator-control-character.json",
    ),
    ("invalid", "BinaryExtractResult--locator-too-long.json"),
    ("invalid", "BinaryExtractResult--negative-span.json"),
    (
        "invalid",
        "BinaryExtractResult--no-source-with-unsupported.json",
    ),
    ("invalid", "BinaryExtractResult--outcome-and-error.json"),
    ("invalid", "BinaryExtractResult--part-bidi-override.json"),
    ("invalid", "BinaryExtractResult--part-too-long.json"),
    ("invalid", "BinaryExtractResult--part-trailing-lf.json"),
    ("invalid", "BinaryExtractResult--role-trailing-lf.json"),
    ("invalid", "BinaryExtractResult--segment-empty-text.json"),
    ("invalid", "BinaryExtractResult--segment-without-part.json"),
    ("invalid", "BinaryExtractResult--segment-without-role.json"),
    ("invalid", "BinaryExtractResult--unsupported-as-error.json"),
    (
        "invalid",
        "BinaryExtractResult--unsupported-locator-trailing-lf.json",
    ),
    (
        "invalid",
        "BinaryExtractResult--unsupported-part-trailing-lf.json",
    ),
    (
        "invalid",
        "BinaryExtractResult--unsupported-without-part.json",
    ),
    ("invalid", "BinaryExtractResult--without-outcome.json"),
    (
        "invalid",
        "InitializeResult--binary-extractor-without-formats.json",
    ),
    (
        "invalid",
        "PluginDeclaration--binary-extractor-duplicate-formats.json",
    ),
    (
        "invalid",
        "PluginDeclaration--binary-extractor-empty-formats.json",
    ),
    (
        "invalid",
        "PluginDeclaration--binary-extractor-format-trailing-lf.json",
    ),
    (
        "invalid",
        "PluginDeclaration--binary-extractor-unknown-format.json",
    ),
    (
        "invalid",
        "PluginDeclaration--binary-extractor-without-formats.json",
    ),
];

/// Error codes of plugin-protocol.md section 10, worked out from the document's table.
const DOCUMENTED_CODES: &[(i64, &str)] = &[
    (-32700, "parse-error"),
    (-32600, "invalid-request"),
    (-32601, "method-not-found"),
    (-32602, "invalid-params"),
    (-32603, "internal-error"),
    (-32001, "protocol-version-unsupported"),
    (-32002, "required-feature-missing"),
    (-32003, "not-initialized"),
    (-32004, "message-too-large"),
    (-32005, "request-cancelled"),
    (-32006, "document-invalid"),
    (-32007, "offline"),
];

fn def_of(name: &str) -> Option<&str> {
    name.split_once("--").map(|(def, _)| def)
}

fn lookup<T: Copy>(table: &[(&str, T)], key: &str) -> Option<T> {
    table.iter().find(|(k, _)| *k == key).map(|(_, d)| *d)
}

fn listed(table: &[(&str, &str)], dir: &str, name: &str) -> bool {
    table.iter().any(|(d, n)| *d == dir && *n == name)
}

/// Every `span` bound in `v`, to build a document long enough for the spans of an instance.
fn longest_span(v: &Value) -> u64 {
    match v {
        Value::Object(o) => o
            .iter()
            .map(|(k, v)| match (k.as_str(), v) {
                ("span", Value::Object(s)) => ["start", "end"]
                    .iter()
                    .filter_map(|b| s.get(*b).and_then(Value::as_u64))
                    .max()
                    .unwrap_or(0),
                _ => longest_span(v),
            })
            .max()
            .unwrap_or(0),
        Value::Array(a) => a.iter().map(longest_span).max().unwrap_or(0),
        _ => 0,
    }
}

fn sent_ids<'a>(result: &'a Value, key: &str) -> Vec<&'a str> {
    result
        .get("results")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| e.get(key).and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

/// What the host does with the entries of a per-document result: match them to the sent ids,
/// then read a per-entry error or parse the entry.
fn per_document(
    result: &Value,
    parse: &dyn Fn(&Value) -> Result<(), PluginError>,
) -> Result<(), PluginError> {
    let ids = sent_ids(result, "document");
    for entry in match_entries(result, "document", &ids)? {
        match entry.get("error") {
            Some(e) => {
                parse_error_object(e)?;
            }
            None => parse(entry)?,
        }
    }
    Ok(())
}

fn decode(decoder: Decoder, value: Value) -> Result<(), String> {
    let text = "x".repeat(usize::try_from(longest_span(&value).min(1 << 16)).unwrap());
    let outcome: Result<(), PluginError> = match decoder {
        Decoder::HostBuilt | Decoder::Pending => {
            panic!("{decoder:?} files are listed in a table and never decoded")
        }
        Decoder::InitializeResult => serde_json::from_value::<InitializeResult>(value)
            .map(drop)
            .map_err(|e| PluginError::OutputInvalid(e.to_string())),
        Decoder::PluginDeclaration => serde_json::from_value::<PluginDeclaration>(value)
            .map(drop)
            .map_err(|e| PluginError::OutputInvalid(e.to_string())),
        Decoder::TagResult => per_document(&value, &|e| parse_tags(e, &text).map(drop)),
        Decoder::ExtractResult => per_document(&value, &|e| parse_segments(e, &text).map(drop)),
        Decoder::ScoreResult => per_document(&value, &|e| parse_score(e).map(drop)),
        Decoder::FixResult => {
            // The sent findings are not in a result: give the fixer as many as the edits name,
            // each covering the whole document.
            let count = value
                .pointer("/results")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|e| e.get("edits").and_then(Value::as_array))
                .flatten()
                .filter_map(|e| e.get("finding").and_then(Value::as_u64))
                .max()
                .map_or(1, |m| m.min(64) + 1);
            let whole = Span {
                start: 0,
                end: text.len() as u64,
            };
            let findings: Vec<Finding> = (0..count)
                .map(|_| Finding {
                    rule: "core:x".into(),
                    severity: "warning".into(),
                    span: whole,
                    message: "m".into(),
                })
                .collect();
            per_document(&value, &|e| parse_edits(e, &text, &findings).map(drop))
        }
        Decoder::CompleteResult => {
            let ids = sent_ids(&value, "item");
            parse_completions(&value, &ids).map(drop)
        }
        Decoder::RecipeResult => parse_recipe(&value).map(drop),
        Decoder::RenderResult => parse_rendered(&value).map(drop),
    };
    outcome.map_err(|e| e.to_string())
}

/// The host's decode of one corpus file: the frame body through `strict::parse`, then the
/// check for the file's kind. The caller has already left out the files in the two tables.
fn host_decode(name: &str, bytes: &[u8]) -> Result<(), String> {
    let value = strict::parse(bytes)?;
    if let Some(def) = def_of(name) {
        let decoder = lookup(DEFS, def).unwrap_or_else(|| panic!("{name}: no DEFS row for {def}"));
        return decode(decoder, value);
    }
    match check_envelope(&value)? {
        // A well-formed error object is a valid reply.
        Envelope::Unsolicited
        | Envelope::Reply {
            reply: Err(PluginError::Rpc(_)),
            ..
        } => Ok(()),
        Envelope::Reply { reply: Err(e), .. } => Err(e.to_string()),
        Envelope::Reply {
            reply: Ok(result), ..
        } => {
            let decoder = RESPONSES
                .iter()
                .find(|(n, _)| *n == name)
                .unwrap_or_else(|| panic!("{name}: no RESPONSES row for this reply"))
                .1;
            decoder.map_or(Ok(()), |d| decode(d, result))
        }
    }
}

/// Files whose diagnostic cannot contain the last field of the `cases.toml` pointer, with the
/// text that pins each one. The host's errors carry no JSON Pointer. Two shapes occur:
/// - `cases.toml` points at the parent object of a missing field, and the host names the
///   missing field.
/// - The host names the offending value or variant, not the field.
const PINNED: &[(&str, &str)] = &[
    // Missing field: the pointer is the parent object.
    ("CompleteResult--without-model.json", "`model`"),
    (
        "ExtractResult--segment-without-context.json",
        "missing field `context`",
    ),
    ("FixResult--edit-without-text.json", "missing field `text`"),
    (
        "FixResult--edit-without-finding.json",
        "missing field `finding`",
    ),
    (
        "FixResult--whole-document-edit-without-finding.json",
        "missing field `finding`",
    ),
    (
        "InitializeResult--hook-without-id.json",
        "missing field `id`",
    ),
    (
        "InitializeResult--requires-network-missing.json",
        "missing field `requiresNetwork`",
    ),
    (
        "PluginDeclaration--hook-without-id.json",
        "missing field `id`",
    ),
    ("RecipeResult--write-without-content.json", "has no content"),
    (
        "TagResult--result-without-document.json",
        "no string `document`",
    ),
    ("TagResult--span-end-missing.json", "missing field `end`"),
    // The text names the value or variant, not the field.
    (
        "error-code-below-range.json",
        "integer -9007199254740992 outside",
    ),
    (
        "FixResult--negative-finding.json",
        "integer `-1`, expected u64",
    ),
    (
        "RecipeResult--unknown-action.json",
        "unknown variant `chmod`",
    ),
    (
        "TagResult--negative-span.json",
        "integer `-1`, expected u64",
    ),
];

/// The text that the host's diagnostic must contain: a pinned text, else the last field name of
/// the pointer. An empty pointer means the root or any location, with no requirement.
fn needle(name: &str, pointer: &str) -> String {
    if let Some((_, text)) = PINNED.iter().find(|(n, _)| *n == name) {
        return (*text).to_owned();
    }
    pointer
        .rsplit('/')
        .find(|s| !s.is_empty() && s.parse::<u64>().is_err())
        .unwrap_or_default()
        .to_owned()
}

#[rstest]
fn valid_file_is_accepted(
    #[files("../../schemas/v3/conformance/plugin-protocol/valid/*.json")] path: PathBuf,
) {
    let name = file_name(&path);
    if listed(HOST_BUILT, &dir_name(&path), &name) || listed(PENDING_BINARY, "valid", &name) {
        return;
    }
    let bytes = fs::read(&path).expect("read case");
    if let Err(e) = host_decode(&name, &bytes) {
        panic!("{name} should be accepted: {e}");
    }
}

#[rstest]
fn invalid_file_is_rejected_at_its_location(
    #[files("../../schemas/v3/conformance/plugin-protocol/invalid/*.json")] path: PathBuf,
) {
    let name = file_name(&path);
    if listed(HOST_BUILT, &dir_name(&path), &name) || listed(PENDING_BINARY, "invalid", &name) {
        return;
    }
    let bytes = fs::read(&path).expect("read case");
    let pointer = cases()
        .get(&name)
        .and_then(toml::Value::as_str)
        .map_or_else(
            || panic!("{name} is missing from cases.toml"),
            str::to_owned,
        );
    let Err(diagnostic) = host_decode(&name, &bytes) else {
        panic!("{name} should be rejected at `{pointer}`, but the host accepted it");
    };
    let field = needle(&name, &pointer);
    assert!(
        diagnostic.contains(&field),
        "{name}: cases.toml says `{pointer}`; the host rejected it without naming `{field}`: {diagnostic}"
    );
}

#[rstest]
fn error_response_code_is_in_its_documented_range(
    #[files("../../schemas/v3/conformance/plugin-protocol/valid/error-response-*.json")]
    path: PathBuf,
) {
    let name = file_name(&path);
    let value = strict::parse(&fs::read(&path).expect("read case")).expect("strict parse");
    let error = value.get("error").expect("error object");
    let rpc = parse_error_object(error).unwrap_or_else(|e| panic!("{name}: {e}"));
    let documented = DOCUMENTED_CODES
        .iter()
        .find(|(c, _)| *c == rpc.code)
        .map_or("internal-error", |(_, n)| *n);
    // The reported code follows the number alone. A code outside the table, such as the plugin
    // range or any other number, is `internal-error` whatever `slopvac_code` says.
    assert_eq!(rpc.code_name(), documented, "{name}: code {}", rpc.code);
    assert_eq!(
        Some(rpc.plugin_code.as_str()),
        error.pointer("/data/slopvac_code").and_then(Value::as_str),
        "{name}"
    );
}

/// Every `(directory, file name)` pair in the corpus.
fn corpus_files() -> BTreeSet<(String, String)> {
    let mut files = BTreeSet::new();
    for dir in ["valid", "invalid"] {
        for entry in fs::read_dir(corpus().join(dir)).expect("corpus dir") {
            let name = entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned();
            if Path::new(&name)
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("json"))
            {
                files.insert((dir.to_owned(), name));
            }
        }
    }
    files
}

#[test]
fn every_def_prefix_in_the_corpus_has_a_host_decoder_row() {
    let missing: BTreeSet<String> = corpus_files()
        .iter()
        .filter_map(|(_, n)| def_of(n))
        .filter(|def| lookup(DEFS, def).is_none())
        .map(str::to_owned)
        .collect();
    assert!(missing.is_empty(), "add these to DEFS: {missing:?}");
}

#[test]
fn every_corpus_file_is_decoded_or_listed_in_exactly_one_table() {
    for (dir, name) in corpus_files() {
        let tables = usize::from(listed(HOST_BUILT, &dir, &name))
            + usize::from(listed(PENDING_BINARY, &dir, &name));
        assert!(tables <= 1, "{dir}/{name} is in both tables");
        // A file of a def that the host builds or cannot read yet has to be in its table.
        if let Some(decoder) = def_of(&name).and_then(|d| lookup(DEFS, d)) {
            match decoder {
                Decoder::HostBuilt => {
                    assert!(
                        listed(HOST_BUILT, &dir, &name),
                        "{dir}/{name}: add to HOST_BUILT"
                    );
                }
                Decoder::Pending => {
                    assert!(
                        listed(PENDING_BINARY, &dir, &name),
                        "{dir}/{name}: add to PENDING_BINARY"
                    );
                }
                // A decoded def may still hold binary files that wait for the adapter.
                _ => assert!(
                    !listed(HOST_BUILT, &dir, &name),
                    "{dir}/{name} has a decoder but sits in HOST_BUILT"
                ),
            }
        }
    }
}

#[test]
fn every_name_in_a_table_exists_once() {
    let files = corpus_files();
    for (label, table) in [
        ("HOST_BUILT", HOST_BUILT),
        ("PENDING_BINARY", PENDING_BINARY),
    ] {
        let mut seen = BTreeSet::new();
        for (dir, name) in table {
            let key = ((*dir).to_owned(), (*name).to_owned());
            assert!(files.contains(&key), "{label}: {dir}/{name} does not exist");
            assert!(seen.insert(key), "{label}: {dir}/{name} is listed twice");
        }
    }
}

#[test]
fn cases_toml_names_exactly_the_invalid_files() {
    let on_disk: BTreeSet<String> = corpus_files()
        .into_iter()
        .filter(|(d, _)| d == "invalid")
        .map(|(_, n)| n)
        .collect();
    let listed: BTreeSet<String> = cases().keys().cloned().collect();
    assert_eq!(on_disk, listed);
}
