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

#[rstest]
#[case::attribute_values(
    "<p class=\"delta\"><a href=\"docs/delta.md\" title='delta'>delta</a> and delta</p>\n",
    "<p class=\"delta\"><a href=\"docs/delta.md\" title='delta'>d</a> and d</p>\n"
)]
#[case::unquoted_attribute_and_src(
    "<img src=delta.png alt=delta> delta\n",
    "<img src=delta.png alt=delta> d\n"
)]
#[case::tag_name("<delta>delta</delta>\n", "<delta>d</delta>\n")]
#[case::bare_address_in_text(
    "<p>See https://example.com/delta and delta.</p>\n",
    "<p>See https://example.com/delta and d.</p>\n"
)]
fn a_raw_scope_fix_never_rewrites_a_tag_or_an_address(#[case] html: &str, #[case] want: &str) {
    let engine = engine(&[rule(
        "swap",
        &json!({"kind": "substitution", "substitutions": {"delta": "d"}}),
        &json!({"scope": "raw"}),
    )]);
    let input = Input::html("page.html", html);
    let doc = engine.lint(&input).unwrap();
    let report = fix_document(&engine, &input, &doc, None).unwrap();
    assert_eq!(report.text, want);
}

#[test]
fn markdown_syntax_in_html_text_is_not_a_destination() {
    // In a Markdown file `(delta)` after a bracketed label is a destination; here it is prose.
    let engine = engine(&[rule(
        "swap",
        &json!({"kind": "substitution", "substitutions": {"delta": "d"}}),
        &json!({"scope": "raw"}),
    )]);
    let html = "<p>[label](delta) text</p>\n";
    let input = Input::html("page.html", html);
    let doc = engine.lint(&input).unwrap();
    let report = fix_document(&engine, &input, &doc, None).unwrap();
    assert_eq!(report.text, "<p>[label](d) text</p>\n");
}

#[rstest]
#[case::inline_elements("<p>one<b>two</b><i>three</i></p>\n", 3)]
#[case::comment("<p>one<!-- note -->two</p>\n", 2)]
#[case::skipped_element("<p>one<code>x</code>two</p>\n", 2)]
#[case::skipped_element_with_spaces("<p>one <code>x</code> two</p>\n", 2)]
#[case::line_break("<p>one<br>two</p>\n", 2)]
#[case::spaced_already("<p>one <b>two</b> three</p>\n", 3)]
#[case::entity_is_not_a_boundary("<p>caf&eacute;</p>\n", 1)]
#[case::start_of_line("<p><b>one</b>two</p>\n", 2)]
fn a_tag_or_comment_between_words_separates_them_as_in_2x(
    #[case] html: &str,
    #[case] words: usize,
) {
    assert_eq!(lint(html).words, words);
}

#[test]
fn a_word_between_inline_tags_is_matched_as_a_whole_word() {
    // 2.x reads `Use leverage now.`; the match starts after `<p>Use<b>` (9 bytes).
    let doc = lint("<p>Use<b>leverage</b>now.</p>\n");
    assert_eq!(places(&doc), vec![at(1, 10)]);
    assert_eq!(matched(&doc), vec!["leverage"]);
}
