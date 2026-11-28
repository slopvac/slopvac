//! The bytes the host writes to a plugin's standard input, pinned stream by stream.
//!
//! The fake plugin run in `wire` mode appends every frame it reads to a file, header included.
//! Each test compares the whole raw capture with the expected bytes: the committed goldens in
//! `tests/fixtures/wire/` joined in order. A golden is a whole frame: the `Content-Length`
//! header, the blank line and the body. The goldens were worked out from plugin-protocol.md,
//! apart from the host code: keys sit in sorted order and the body has no spaces. The two
//! frames that depend on the request id (`shutdown` after a request) and the one request of the
//! cancel test are built here from written-out bodies.
//!
//! The first assertion of every test is that raw comparison. Nothing parses, decodes or splits
//! the capture before it, and nothing splits it after. The outcome of the call, the start and the
//! shutdown is held as a `Result` and checked after the comparison, so a plugin that fails on a
//! changed frame cannot hide the byte difference.
//!
//! A second group pins the size probe for a documents request. The probe sets how many bytes
//! a batch may add to the request, so it must equal the request with no documents exactly.
#![cfg(unix)]

#[expect(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "shared test support: a panic is a failed test"
)]
mod common;

use std::fs;
use std::path::PathBuf;

use common::*;
use rstest::rstest;
use serde_json::{Value, json};
use slopvac_plugins::hooks::{
    CompleteItem, CompleteRequest, Document, Finding, FindingsDocument, ProviderTask, Span,
};
use slopvac_plugins::{DocOutcome, HostConfig, Plugin, PluginDeclaration, PluginError};

const GOLDENS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/wire");

/// A config with a fixed host name and version, so `initialize` has one set of bytes.
fn wire_cfg(base: HostConfig) -> HostConfig {
    HostConfig {
        host_name: "slopvac-test".into(),
        host_version: "9.9.9".into(),
        ..base
    }
}

/// Why a run did not go well.
#[derive(Debug, PartialEq)]
enum Failure {
    /// The plugin did not start.
    Start(String),
    /// The call returned this error.
    Call(PluginError),
    /// The shutdown was not clean, or the test asked for an unknown hook.
    Other(String),
}

/// What one run of a fake plugin left behind.
struct Run {
    /// Every byte the host wrote to the plugin, in order.
    raw: Vec<u8>,
    /// Whether the start, the call and the clean shutdown all went well.
    outcome: Result<(), Failure>,
}

/// `len` bytes of `bytes` as hex, from `from`, never past the end.
fn hex(bytes: &[u8], from: usize, len: usize) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::new();
    for b in bytes.iter().skip(from).take(len) {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    out
}

/// The failure text of a raw comparison: both lengths, the first offset that differs and a
/// bounded hex window of 32 bytes either side of it from each side.
fn difference(captured: &[u8], expected: &[u8]) -> String {
    let at = captured
        .iter()
        .zip(expected)
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| captured.len().min(expected.len()));
    let from = at.saturating_sub(32);
    let len = 64;
    format!(
        "captured {} bytes, expected {} bytes, first difference at offset {at}\n captured [{from}..]: {}\n expected [{from}..]: {}",
        captured.len(),
        expected.len(),
        hex(captured, from, len),
        hex(expected, from, len),
    )
}

/// Compare the whole capture with the expected bytes. This is the first assertion of a test.
fn assert_raw(captured: &[u8], expected: &[u8], what: &str) {
    assert!(
        captured == expected,
        "{what}: the captured bytes differ from the golden\n{}",
        difference(captured, expected)
    );
}

/// One frame from a body written out here: the header, the blank line and the body.
fn frame(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

/// The `shutdown` request the host sends after `requests` earlier requests (`initialize` counts).
fn shutdown_after(requests: u32) -> Vec<u8> {
    let id = requests + 1;
    frame(&format!(
        r#"{{"id":{id},"jsonrpc":"2.0","method":"shutdown"}}"#
    ))
}

/// The goldens, joined in order.
fn stream(names: &[&str]) -> Vec<u8> {
    names.iter().flat_map(|n| golden(n)).collect()
}

/// A whole session with one request: `initialize`, `initialized`, the request frame, the
/// `shutdown` that follows it and `exit`.
fn session(request: &[u8]) -> Vec<u8> {
    [
        golden("initialize"),
        golden("initialized"),
        request.to_vec(),
        shutdown_after(2),
        golden("exit"),
    ]
    .concat()
}

/// Start a fake plugin, run `act` on it, shut it down and return every byte the host wrote.
/// A failure of any step is kept in the outcome, not raised.
#[expect(clippy::unwrap_used, reason = "test helper: a panic is a failed test")]
fn run_plugin(
    name: &str,
    mode: &str,
    cfg: &HostConfig,
    declaration: PluginDeclaration,
    overrides: &Value,
    act: impl FnOnce(&Plugin) -> Result<(), Failure>,
) -> Run {
    let dir = scratch(name);
    let pidfile = dir.join("pids");
    let started = Plugin::start(
        &spec_with(mode, overrides, Some(&pidfile), declaration),
        cfg,
        &Allow,
    );
    let outcome = match started {
        Err(e) => Err(Failure::Start(format!("{e:?}"))),
        Ok(p) => {
            let called = act(&p);
            let down = p.shutdown();
            called.and_then(|()| {
                if down.clean() {
                    Ok(())
                } else {
                    Err(Failure::Other(format!("shutdown: {down:?}")))
                }
            })
        }
    };
    let raw = fs::read(dir.join("pids.wire")).unwrap_or_default();
    fs::remove_dir_all(dir).unwrap();
    Run { raw, outcome }
}

/// Keep a call's error, so the caller can check it after the bytes.
fn seen<T>(r: Result<T, PluginError>) -> Result<(), Failure> {
    r.map(drop).map_err(Failure::Call)
}

/// The bytes of a committed golden. A missing file fails the test.
#[expect(clippy::panic, reason = "test helper: a panic is a failed test")]
fn golden(name: &str) -> Vec<u8> {
    let path = PathBuf::from(GOLDENS).join(format!("host-{name}.frame"));
    fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn doc(id: &str, text: &str, media_type: Option<&str>, path: Option<&str>) -> Document {
    Document {
        id: id.into(),
        text: text.into(),
        media_type: media_type.map(Into::into),
        path: path.map(Into::into),
    }
}

/// Two documents; the second has text that JSON escapes and UTF-8 that it does not.
fn two_docs() -> Vec<Document> {
    vec![
        doc("d0", "text 0", Some("text/markdown"), None),
        doc("d1", "h\u{e9}llo \"q\"\n", None, None),
    ]
}

fn finding(start: u64, end: u64) -> Finding {
    Finding {
        rule: "core:x".into(),
        severity: "warning".into(),
        span: Span { start, end },
        message: "m".into(),
    }
}

fn findings_docs() -> Vec<FindingsDocument> {
    vec![
        FindingsDocument {
            document: doc("d0", "abcdef", Some("text/markdown"), None),
            findings: vec![finding(0, 2), finding(3, 4)],
        },
        FindingsDocument {
            document: doc("d1", "ghi", None, None),
            findings: vec![finding(1, 2)],
        },
    ]
}

#[test]
fn the_lifecycle_frames_match_their_goldens() {
    let run = run_plugin(
        "wire-life",
        "wire",
        &wire_cfg(cfg()),
        decl_all(),
        &offer_all(),
        |_| Ok(()),
    );
    let expected = stream(&["initialize", "initialized", "shutdown", "exit"]);
    assert_raw(&run.raw, &expected, "lifecycle");
    assert_eq!(run.outcome, Ok(()));
}

/// Ask the hook once and return what the run left behind.
fn hook_run(hook: &str) -> Run {
    run_plugin(
        &format!("wire-{hook}"),
        "wire",
        &wire_cfg(cfg()),
        decl_all(),
        &offer_all(),
        |p| match hook {
            "tagger" => seen(p.tag("main", &two_docs())),
            "extractor" => seen(p.extract("main", &two_docs())),
            "fixer" => seen(p.fix("main", &findings_docs())),
            "scorer" => seen(p.score("main", &findings_docs())),
            "reporter" => {
                let options = json!({"width": 80, "color": false});
                seen(p.render("main", &json!({"schema_version": 3}), &options))
            }
            "provider" => {
                let items = [
                    CompleteItem {
                        id: "i1".into(),
                        text: "abc".into(),
                        instruction: None,
                    },
                    CompleteItem {
                        id: "i2".into(),
                        text: "x".into(),
                        instruction: Some("fail".into()),
                    },
                ];
                seen(p.complete(
                    "main",
                    &CompleteRequest::new(ProviderTask::Rewrite, Some("m1".into()), items.to_vec()),
                ))
            }
            "recipe" => {
                let options = json!({"marker": "hi", "force": true});
                seen(p.recipe("main", "claude", &options))
            }
            other => Err(Failure::Other(format!("no such hook {other}"))),
        },
    )
}

#[rstest]
#[case::tagger("tagger")]
#[case::extractor("extractor")]
#[case::fixer("fixer")]
#[case::scorer("scorer")]
#[case::reporter("reporter")]
#[case::provider("provider")]
#[case::recipe("recipe")]
fn each_hook_request_matches_its_golden(#[case] hook: &str) {
    let run = hook_run(hook);
    assert_raw(&run.raw, &session(&golden(hook)), hook);
    assert_eq!(run.outcome, Ok(()));
}

#[test]
fn a_granted_document_path_is_sent_with_the_document() {
    let mut d = decl_all();
    d.features.optional = vec!["document-path".into()];
    let mut o = offer_all();
    o["capabilities"]["features"] = json!({"optional": ["document-path"]});
    let docs = vec![doc(
        "d0",
        "text 0",
        Some("text/markdown"),
        Some("docs/a.md"),
    )];
    let run = run_plugin("wire-path", "wire", &wire_cfg(cfg()), d, &o, |p| {
        seen(p.tag("main", &docs))
    });
    assert_raw(&run.raw, &session(&golden("tagger-path")), "tagger-path");
    assert_eq!(run.outcome, Ok(()));
}

#[test]
fn a_request_past_its_deadline_is_followed_by_a_cancel_with_its_id() {
    let run = run_plugin(
        "wire-cancel",
        "wire+cancel-ok",
        &wire_cfg(tight()),
        decl_all(),
        &offer_all(),
        |p| {
            let got = code(&p.tag("main", &docs(1)));
            if got == "plugin-timeout" {
                Ok(())
            } else {
                Err(Failure::Other(format!("code {got}")))
            }
        },
    );
    let request = frame(
        r#"{"id":2,"jsonrpc":"2.0","method":"tagger/tag","params":{"documents":[{"id":"d0","text":"text 0"}],"tagger":"main"}}"#,
    );
    // initialize, initialized, the request, the cancel, shutdown, exit
    let expected = [
        golden("initialize"),
        golden("initialized"),
        request,
        golden("cancel"),
        shutdown_after(2),
        golden("exit"),
    ]
    .concat();
    assert_raw(&run.raw, &expected, "cancel");
    assert_eq!(run.outcome, Ok(()));
}

/// Run one documents call with a plugin whose message limit is the smallest allowed. The run
/// holds the bytes the host wrote and the outcome of the call.
fn one_document(hook: &str, text: &str) -> Run {
    let d = doc("d0", text, None, None);
    let fd = || {
        vec![FindingsDocument {
            document: d.clone(),
            findings: vec![finding(0, 1)],
        }]
    };
    let mut o = offer_all();
    o["limits"] = json!({"maxMessageBytes": 65_536});
    run_plugin(
        &format!("wire-probe-{hook}-{}", text.len()),
        "wire",
        &wire_cfg(cfg()),
        decl_all(),
        &o,
        |p| match hook {
            "tagger" => first_outcome(p.tag("main", std::slice::from_ref(&d))),
            "extractor" => first_outcome(p.extract("main", std::slice::from_ref(&d))),
            "fixer" => first_outcome(p.fix("main", &fd())),
            "scorer" => first_outcome(p.score("main", &fd())),
            other => Err(Failure::Other(format!("no such hook {other}"))),
        },
    )
}

/// The outcome of the first document of a batch.
fn first_outcome<T>(out: Result<Vec<DocOutcome<T>>, PluginError>) -> Result<(), Failure> {
    match out {
        Ok(v) => match v.into_iter().next() {
            Some(first) => seen(first.outcome),
            None => Err(Failure::Other("no outcome".into())),
        },
        Err(e) => Err(Failure::Call(e)),
    }
}

/// A text of `n` bytes: one letter, then spaces, so the fake plugin's answer stays small.
fn pad(n: usize) -> String {
    format!("a{}", " ".repeat(n - 1))
}

/// The encoded size of the one document `one_document` sends, for a text of `n` bytes.
#[expect(clippy::unwrap_used, reason = "test helper: a panic is a failed test")]
fn sent_size(hook: &str, n: usize) -> usize {
    let d = doc("d0", &pad(n), None, None);
    if hook == "fixer" || hook == "scorer" {
        let fd = FindingsDocument {
            document: d,
            findings: vec![finding(0, 1)],
        };
        serde_json::to_vec(&fd).unwrap().len()
    } else {
        serde_json::to_vec(&d).unwrap().len()
    }
}

/// Each case gives the request with the largest id and no documents, which is what the batch
/// size probe must measure.
#[rstest]
#[case::tagger(
    "tagger",
    r#"{"id":9007199254740991,"jsonrpc":"2.0","method":"tagger/tag","params":{"documents":[],"tagger":"main"}}"#
)]
#[case::extractor(
    "extractor",
    r#"{"id":9007199254740991,"jsonrpc":"2.0","method":"extractor/extract","params":{"documents":[],"extractor":"main"}}"#
)]
#[case::fixer(
    "fixer",
    r#"{"id":9007199254740991,"jsonrpc":"2.0","method":"fixer/fix","params":{"documents":[],"fixer":"main"}}"#
)]
#[case::scorer(
    "scorer",
    r#"{"id":9007199254740991,"jsonrpc":"2.0","method":"scorer/score","params":{"documents":[],"scorer":"main"}}"#
)]
fn the_batch_overhead_equals_the_empty_batch_request(#[case] hook: &str, #[case] empty: &str) {
    let limit = 65_536; // the smallest message limit a plugin may report
    let fit = limit - empty.len() - sent_size(hook, 1) + 1;
    assert_eq!(
        empty.len() + sent_size(hook, fit),
        limit,
        "the document fills the limit"
    );
    let sent = one_document(hook, &pad(fit));
    // The probe counts the 16 digits of the largest id. The request goes out as id 2, which has
    // one digit, so the real body is 15 bytes under the limit.
    assert_raw(
        &sent.raw,
        &session(&golden(&format!("probe-{hook}"))),
        "the session whose request fills the limit",
    );
    assert_eq!(
        sent.outcome,
        Ok(()),
        "a document that fills the limit is sent"
    );
    let over = one_document(hook, &pad(fit + 1));
    // No request leaves for a document one byte over, so the session is the bare lifecycle.
    let expected = stream(&["initialize", "initialized", "shutdown", "exit"]);
    assert_raw(&over.raw, &expected, "the session with no request");
    assert!(
        matches!(
            over.outcome,
            Err(Failure::Call(PluginError::MessageTooLarge(_)))
        ),
        "one byte more cannot fit: {:?}",
        over.outcome
    );
}
