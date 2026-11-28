//! A standalone HTML file, read as 2.x reads it (`_parse_html`): the visible text of each source
//! line is one paragraph, code and hidden elements give no prose, a position is exact, and a fix
//! never edits markup.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![allow(missing_docs, reason = "test crate: nothing public to document")]

mod common;

use common::{engine, matched, places, rule, tokens};
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Value, json};
use slopvac_engine::{Engine, EngineError, Input, fix_document};
use slopvac_model::Document;
use slopvac_parse::ParseError;

fn leverage() -> Engine {
    engine(&[tokens("dull", "leverage", &json!({}))])
}

fn lint(text: &str) -> Document {
    leverage().lint(&Input::html("page.html", text)).unwrap()
}

fn at(line: usize, column: usize) -> (String, usize, usize) {
    ("core:cat.dull".to_owned(), line, column)
}

/// The byte range of each finding.
fn spans(doc: &Document) -> Vec<(usize, usize)> {
    doc.findings
        .iter()
        .filter_map(|f| f.extent.as_ref())
        .map(|e| (e.span.start(), e.span.end()))
        .collect()
}

/// The text of `html` after a fix of every finding of a substitution of `from` to `to`, with
/// the rule fields `extra`.
fn fixed(html: &str, from: &str, to: &str, extra: &Value) -> String {
    let engine = engine(&[rule(
        "swap",
        &json!({"kind": "substitution", "substitutions": {from: to}}),
        extra,
    )]);
    let input = Input::html("page.html", html);
    let doc = engine.lint(&input).unwrap();
    fix_document(&engine, &input, &doc, None).unwrap().text
}

#[test]
fn elements_on_one_line_join_into_one_paragraph_with_a_space_between() {
    // 2.x reads the row as `Alpha beta.`: two words, one sentence, one paragraph.
    let doc = lint("<table><tr><td>Alpha</td><td>beta.</td></tr></table>\n");
    assert_eq!((doc.words, doc.sentences, doc.paragraphs), (2, 1, 1));
}

#[test]
fn a_heading_and_a_list_item_on_separate_lines_are_two_paragraphs() {
    let doc = lint("<h1>Title</h1>\n<ul>\n<li>Item one.</li>\n</ul>\n");
    assert_eq!(doc.paragraphs, 2);
}

#[test]
fn a_paragraph_over_two_lines_is_two_paragraphs() {
    let doc = lint("<p>One.\nTwo.</p>\n\n<p>Three.</p>\n");
    assert_eq!(doc.paragraphs, 3);
}

#[test]
fn a_sentence_does_not_run_over_a_line_end() {
    let doc = lint("<p>First half\nsecond half.</p>\n");
    assert_eq!((doc.sentences, doc.paragraphs), (2, 2));
}

#[rstest]
#[case::script("<script>leverage now</script>\n<p>Hello.</p>\n")]
#[case::style("<style>leverage { color: red }</style>\n<p>Hello.</p>\n")]
#[case::noscript("<noscript>leverage now</noscript>\n<p>Hello.</p>\n")]
#[case::template("<template>leverage now</template>\n<p>Hello.</p>\n")]
#[case::head("<head><title>leverage now</title></head>\n<p>Hello.</p>\n")]
#[case::pre("<pre>leverage now</pre>\n<p>Hello.</p>\n")]
fn a_hidden_element_adds_no_words(#[case] html: &str) {
    // Only `Hello.` is prose.
    let doc = lint(html);
    assert_eq!((doc.words, doc.findings.len()), (1, 0));
}

#[test]
fn inline_code_adds_no_words() {
    let doc = lint("<p>one <code>x</code> two</p>\n");
    assert_eq!(doc.words, 2);
}

#[rstest]
#[case::inline_code("<p>Use <code>leverage</code> here.</p>\n")]
#[case::kbd("<p>Press <kbd>leverage</kbd> now.</p>\n")]
#[case::samp("<p>It printed <samp>leverage</samp> then.</p>\n")]
#[case::comment("<!-- leverage -->\n")]
#[case::attribute("<a title=\"leverage\">link</a>\n")]
fn text_that_is_not_visible_prose_gives_no_finding(#[case] html: &str) {
    assert_eq!(lint(html).findings.len(), 0);
}

#[test]
fn inline_code_is_still_a_code_span() {
    // The tags are protected on their own; only the code span protects the `x` between them.
    let html = "<p>one <code>x</code> two</p>\n";
    assert_eq!(fixed(html, "x", "y", &json!({"scope": "raw"})), html);
}

#[test]
fn a_match_in_a_paragraph_is_placed_by_source_column() {
    // `<p>` is 3 bytes and `Use ` is 4, so the word starts at column 8.
    let doc = lint("<p>Use leverage now.</p>\n");
    assert_eq!(places(&doc), vec![at(1, 8)]);
}

#[test]
fn a_finding_across_an_inline_tag_spans_through_the_tag() {
    // `<p>We ` is 6 bytes. `leverage ` runs to 15, `<em>` to 19 and `synergy` to 26.
    let engine = engine(&[tokens("dull", "leverage synergy", &json!({}))]);
    let doc = engine
        .lint(&Input::html(
            "page.html",
            "<p>We leverage <em>synergy</em> today.</p>\n",
        ))
        .unwrap();
    assert_eq!(spans(&doc), vec![(6, 26)]);
}

#[test]
fn a_finding_on_a_decoded_entity_covers_the_whole_entity() {
    // `<p>A ` is 5 bytes, `caf` runs to 8 and `&eacute;` to 16.
    let engine = engine(&[tokens("dull", "café", &json!({}))]);
    let doc = engine
        .lint(&Input::html("page.html", "<p>A caf&eacute; here.</p>\n"))
        .unwrap();
    assert_eq!(spans(&doc), vec![(5, 16)]);
    assert_eq!(matched(&doc), vec!["café"]);
}

#[test]
fn a_decoded_entity_before_a_match_keeps_the_source_column() {
    // `<p>A &amp; ` is 11 bytes, so the word starts at column 12.
    let doc = lint("<p>A &amp; leverage</p>\n");
    assert_eq!(places(&doc), vec![at(1, 12)]);
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
    // 2.x reads `Use leverage now.`. The match starts after `<p>Use<b>` (9 bytes) and ends
    // before `</b>`: the boundary space maps to no source bytes.
    let doc = lint("<p>Use<b>leverage</b>now.</p>\n");
    assert_eq!(places(&doc), vec![at(1, 10)]);
    assert_eq!(spans(&doc), vec![(9, 17)]);
}

#[test]
fn a_prose_fix_changes_the_text_and_leaves_the_markup() {
    let html = "<p class=\"leverage\">Do <b>leverage</b> it.</p>\n";
    assert_eq!(
        fixed(html, "leverage", "use", &json!({})),
        "<p class=\"leverage\">Do <b>use</b> it.</p>\n"
    );
}

#[rstest]
#[case::quoted_attribute_values(
    "<p class=\"delta\"><a href=\"docs/delta.md\" title='delta'>delta</a> and delta</p>\n",
    "<p class=\"delta\"><a href=\"docs/delta.md\" title='delta'>d</a> and d</p>\n"
)]
#[case::unquoted_attribute_values(
    "<img src=delta.png alt=delta> delta\n",
    "<img src=delta.png alt=delta> d\n"
)]
#[case::tag_name("<delta>delta</delta>\n", "<delta>d</delta>\n")]
#[case::bare_url(
    "<p>See https://example.com/delta and delta.</p>\n",
    "<p>See https://example.com/delta and d.</p>\n"
)]
#[case::www_host(
    "<p>Visit www.delta.org or delta.</p>\n",
    "<p>Visit www.delta.org or d.</p>\n"
)]
#[case::email(
    "<p>Mail delta@example.com about delta.</p>\n",
    "<p>Mail delta@example.com about d.</p>\n"
)]
#[case::markdown_link_syntax_is_prose("<p>[label](delta) text</p>\n", "<p>[label](d) text</p>\n")]
fn a_raw_scope_fix_never_rewrites_a_tag_or_an_address(#[case] html: &str, #[case] want: &str) {
    assert_eq!(fixed(html, "delta", "d", &json!({"scope": "raw"})), want);
}

#[test]
fn a_file_over_the_html_cap_is_a_parse_error() {
    // The HTML parser takes at most 16 MiB.
    let html = "a".repeat(16 * 1024 * 1024 + 1);
    let err = leverage()
        .lint(&Input::html("page.html", &html))
        .unwrap_err();
    assert!(
        matches!(err, EngineError::Parse(ParseError::TooLarge)),
        "{err:?}"
    );
}

#[test]
fn a_file_over_the_work_budget_is_a_parse_error() {
    // Each paragraph copies its 500 enclosing quotes: 10 million units of work for about
    // 166,000 bytes, where the budget is 32 units a byte.
    let html = "<blockquote>".repeat(500) + &"<p>x</p>".repeat(20_000);
    let err = leverage()
        .lint(&Input::html("page.html", &html))
        .unwrap_err();
    assert!(
        matches!(err, EngineError::Parse(ParseError::Budget)),
        "{err:?}"
    );
}

/// Pieces of HTML the property test joins: words, markup, entities, line ends and text in
/// several byte widths.
const PIECES: [&str; 22] = [
    "leverage",
    "word",
    " ",
    ".",
    "\n",
    "\r\n",
    "\r",
    "<p>",
    "</p>",
    "<b>",
    "</b>",
    "<em>",
    "</em>",
    "<br>",
    "<!-- c -->",
    "&amp;",
    "&eacute;",
    "é",
    "日本",
    "<code>x</code>",
    "<pre>leverage</pre>",
    "<a href=\"leverage\">",
];

/// The visible text of `html`: tags dropped and the two entities in [`PIECES`] decoded.
fn visible(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&eacute;", "é")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn every_finding_span_is_inside_the_source_on_character_boundaries(
        pieces in prop::collection::vec(prop::sample::select(&PIECES[..]), 0..24),
    ) {
        let html = pieces.concat();
        let doc = lint(&html);
        for f in &doc.findings {
            let extent = f.extent.as_ref().expect("a token finding has a range");
            let (start, end) = (extent.span.start(), extent.span.end());
            prop_assert!(start <= end && end <= html.len(), "{html:?}: {start}..{end}");
            prop_assert!(
                html.is_char_boundary(start) && html.is_char_boundary(end),
                "{html:?}: {start}..{end}"
            );
            prop_assert_eq!(visible(&html[start..end]), f.matched_text.clone(), "{:?}", html);
        }
    }
}
