//! The `schemas/v3/conformance/plugin-protocol` corpus, one test per file, run through the
//! host's own decode and strict checks: `strict::parse`, the `InitializeResult` and
//! `PluginDeclaration` types, and the `hooks` result parsers the host applies to plugin output.
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
//! Where the host has no decoder for a file's kind ([`Decoder::Undecoded`]), the file only
//! goes through `strict::parse`. An `invalid/` file of such a kind is therefore accepted, and the
//! test fails for it: that gap is the finding.
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
use slopvac_plugins::PluginError;
use slopvac_plugins::hooks::{
    Finding, Span, match_entries, parse_completions, parse_edits, parse_error_object, parse_recipe,
    parse_rendered, parse_score, parse_segments, parse_tags,
};
use slopvac_plugins::protocol::{InitializeResult, PluginDeclaration};
use slopvac_plugins::strict;

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
    /// The host has no type for this kind: it builds the message or has no binary support.
    Undecoded,
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
    ("InitializeParams", Decoder::Undecoded),
    // The host has no binary-extractor hook yet.
    ("BinaryExtractResult", Decoder::Undecoded),
];

/// The result type of the request that a `response-*.json` file answers. The method comes from
/// the file name. The `Undecoded` rows are envelope-only files and replies without a result
/// type.
const RESPONSES: &[(&str, Decoder)] = &[
    ("response-initialize.json", Decoder::InitializeResult),
    // `shutdown` answers `null`.
    ("response-shutdown.json", Decoder::Undecoded),
    // Answers a `tagger/tag` request (id 2 in `request-tag.json`).
    ("response-unknown-result-fields.json", Decoder::TagResult),
    ("response-null-id-success.json", Decoder::Undecoded),
    ("response-result-and-error.json", Decoder::Undecoded),
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
    (-32008, "binary-input-declined"),
    (-32009, "extract-malformed"),
    (-32010, "extract-budget"),
    (-32011, "binary-encrypted"),
];

fn def_of(name: &str) -> Option<&str> {
    name.split_once("--").map(|(def, _)| def)
}

fn lookup(table: &[(&str, Decoder)], key: &str) -> Option<Decoder> {
    table.iter().find(|(k, _)| *k == key).map(|(_, d)| *d)
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
        Decoder::Undecoded => Ok(()),
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
/// check for the file's kind.
fn host_decode(name: &str, bytes: &[u8]) -> Result<(), String> {
    let value = strict::parse(bytes)?;
    if let Some(def) = def_of(name) {
        let decoder = lookup(DEFS, def).unwrap_or_else(|| panic!("{name}: no DEFS row for {def}"));
        return decode(decoder, value);
    }
    let Some(msg) = value.as_object() else {
        // A batch or a bare value: the host's reader checks this after `strict::parse`.
        return Ok(());
    };
    if msg.contains_key("method") {
        // Host-to-plugin request or notification. The host builds these and never reads them.
        return Ok(());
    }
    if let Some(error) = msg.get("error") {
        return parse_error_object(error)
            .map(drop)
            .map_err(|e| e.to_string());
    }
    if let Some(result) = msg.get("result") {
        let decoder = lookup(RESPONSES, name)
            .unwrap_or_else(|| panic!("{name}: no RESPONSES row for this reply"));
        return decode(decoder, result.clone());
    }
    Ok(())
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

#[test]
fn every_def_prefix_in_the_corpus_has_a_host_decoder_row() {
    let mut missing = BTreeSet::new();
    for dir in ["valid", "invalid"] {
        for entry in fs::read_dir(corpus().join(dir)).expect("corpus dir") {
            let name = entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned();
            if let Some(def) = def_of(&name)
                && lookup(DEFS, def).is_none()
            {
                missing.insert(def.to_owned());
            }
        }
    }
    assert!(missing.is_empty(), "add these to DEFS: {missing:?}");
}

#[test]
fn cases_toml_names_exactly_the_invalid_files() {
    let on_disk: BTreeSet<String> = fs::read_dir(corpus().join("invalid"))
        .expect("invalid dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|n| {
            Path::new(n)
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("json"))
        })
        .collect();
    let listed: BTreeSet<String> = cases().keys().cloned().collect();
    assert_eq!(on_disk, listed);
}
