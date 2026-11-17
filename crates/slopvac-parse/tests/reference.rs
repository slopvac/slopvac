//! Integration tests for reference.
//! Parity with the Python projection on the documents in `fixtures/reference.json`.
//!
//! The fixture comes from `slopvac.projection.project`. See `fixtures/divergences.md` for
//! what the two read differently.

use serde_json::Value;
use slopvac_model::Span;
use slopvac_parse::{parse_markdown, parse_mdx};

/// Words of the projected prose and the source text each one maps to.
#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn words(source: &str, mdx: bool) -> Vec<(String, String)> {
    let body = Span::new(0, source.len()).unwrap();
    let doc = if mdx {
        parse_mdx(source, body)
    } else {
        parse_markdown(source, body)
    }
    .unwrap();
    let mut out = Vec::new();
    for projection in doc.blocks.iter().filter_map(|b| b.projection.as_ref()) {
        let text = projection.text().replace('\0', " ");
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i].1.is_whitespace() {
                i += 1;
                continue;
            }
            let start = i;
            while i < chars.len() && !chars[i].1.is_whitespace() {
                i += 1;
            }
            let word: String = chars[start..i].iter().map(|(_, c)| c).collect();
            let span = projection.span_for_range(start, i);
            out.push((word, source[span.start()..span.end()].to_owned()));
        }
    }
    out
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn cases() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/reference.json")).unwrap()
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn pairs(case: &Value) -> Vec<(String, String)> {
    case["words"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| {
            (
                w["text"].as_str().unwrap().to_owned(),
                w["src"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn markdown_words_and_source_text_match_python() {
    for case in cases() {
        let source = case["source"].as_str().unwrap();
        assert_eq!(words(source, false), pairs(&case), "{}", case["id"]);
    }
}

#[test]
fn mdx_reads_plain_markdown_like_markdown() {
    // The same documents, minus those with constructs MDX reads differently: `<` starts
    // a JSX tag and a comment is a syntax error.
    for case in cases() {
        let source = case["source"].as_str().unwrap();
        if source.contains('<') && !source.contains("&lt;") || case["id"] == "indented_code" {
            continue;
        }
        assert_eq!(words(source, true), pairs(&case), "{}", case["id"]);
    }
}
