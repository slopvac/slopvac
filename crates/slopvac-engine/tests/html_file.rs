//! A standalone HTML file, read as 2.x reads it (`_parse_html`): the visible text of each source
//! line is one paragraph, code and hidden elements give no prose, and a position is exact.

#![allow(
    missing_docs,
    clippy::assert_is_empty,
    clippy::cloned_ref_to_slice_refs
)]

mod common;

use common::{engine, matched, places, rule, tokens};
use rstest::rstest;
use serde_json::json;
use slopvac_engine::{Engine, Input, fix_document};
use slopvac_model::Document;

fn leverage() -> Engine {
    engine(&[tokens("dull", "leverage", &json!({}))])
}

fn lint(text: &str) -> Document {
    leverage().lint(&Input::html("page.html", text)).unwrap()
}

fn at(line: usize, column: usize) -> (String, usize, usize) {
    ("core:cat.dull".to_owned(), line, column)
}

#[test]
fn a_match_in_a_paragraph_is_placed_by_source_column() {
    // `<p>` is 3 bytes and `Use ` is 4, so the word starts at column 8.
    let doc = lint("<p>Use leverage now.</p>\n");
    assert_eq!(places(&doc), vec![at(1, 8)]);
}

#[test]
fn a_decoded_entity_maps_to_its_whole_source_text() {
    // `<p>A &amp; ` is 11 bytes, so the word starts at column 12.
    let doc = lint("<p>A &amp; leverage</p>\n");
    assert_eq!(places(&doc), vec![at(1, 12)]);
    assert_eq!(matched(&doc), vec!["leverage"]);
}

#[rstest]
#[case::lf("\n")]
#[case::crlf("\r\n")]
#[case::cr("\r")]
fn a_second_line_is_placed_on_line_two_whatever_ends_the_first(#[case] eol: &str) {
    let doc = lint(&format!(
        "<p>Use leverage.</p>{eol}<p>Use leverage.</p>{eol}"
    ));
    assert_eq!(places(&doc), vec![at(1, 8), at(2, 8)]);
}

#[rstest]
#[case::pre("<pre><code>leverage</code></pre>\n")]
#[case::inline_code("<p>Use <code>leverage</code> here.</p>\n")]
#[case::kbd("<p>Press <kbd>leverage</kbd> now.</p>\n")]
#[case::samp("<p>It printed <samp>leverage</samp> then.</p>\n")]
#[case::script("<script>var leverage = 1;</script>\n")]
#[case::style("<style>.leverage { color: red }</style>\n")]
#[case::head("<head><title>leverage</title></head>\n")]
#[case::comment("<!-- leverage -->\n")]
#[case::attribute("<a title=\"leverage\">link</a>\n")]
fn text_that_is_not_visible_prose_gives_no_finding(#[case] html: &str) {
    assert_eq!(lint(html).findings.len(), 0);
}

#[test]
fn each_source_line_with_text_is_one_paragraph() {
    // A `<p>` that spans two lines is two paragraphs, and so is a heading: 2.x reads lines.
    let doc = lint("<h1>Title</h1>\n<p>One.\nTwo.</p>\n\n<p>Three.</p>\n");
    assert_eq!(doc.paragraphs, 4);
}

#[test]
fn elements_on_one_line_join_into_one_paragraph_with_a_space_between() {
    let doc = lint("<table><tr><td>Alpha</td><td>beta.</td></tr></table>\n");
    assert_eq!((doc.words, doc.sentences, doc.paragraphs), (2, 1, 1));
}

#[test]
fn a_sentence_does_not_run_over_a_line_end() {
    let doc = lint("<p>First half\nsecond half.</p>\n");
    assert_eq!((doc.sentences, doc.paragraphs), (2, 2));
}

#[test]
fn a_markdown_fence_in_a_standalone_file_is_plain_text() {
    let doc = lint("```\nUse leverage\n```\n");
    assert_eq!(places(&doc), vec![at(2, 5)]);
}

#[test]
fn front_matter_is_not_stripped() {
    let doc = lint("---\ntitle: Use leverage\n---\n");
    assert_eq!(places(&doc), vec![at(2, 12)]);
}

#[test]
fn a_fix_changes_the_text_and_leaves_the_markup() {
    let engine = engine(&[rule(
        "swap",
        &json!({"kind": "substitution", "substitutions": {"leverage": "use"}}),
        &json!({}),
    )]);
    let html = "<p class=\"leverage\">Do <b>leverage</b> it.</p>\n";
    let input = Input::html("page.html", html);
    let doc = engine.lint(&input).unwrap();
    let report = fix_document(&engine, &input, &doc, None).unwrap();
    assert_eq!(report.text, "<p class=\"leverage\">Do <b>use</b> it.</p>\n");
}
