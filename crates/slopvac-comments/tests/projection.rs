//! Every character of a fragment maps back to its own bytes in the source.

mod common;

use common::{FIXTURES, fragments};
use slopvac_comments::{Fragment, Language, TokenKind};

/// Extra sources that stress offsets: line endings, multi-byte text, BOM, banners.
const EDGE_SOURCES: &[(Language, &str)] = &[
    (Language::Rust, "// a\r\n// b\r\n\r\n// c\r\n"),
    (Language::Python, "# a\r# b\r\r# c\r"),
    (Language::Rust, "// héllo → wörld 😀\n// ünï\n"),
    (Language::Rust, "\u{feff}// bom\n// next\n"),
    (Language::C, "/*\r\n * one\r\n * two\r\n */\r\n"),
    (Language::C, "/*** 😀 ***\n **  x\n ***/\n"),
    (
        Language::Python,
        "def f():\n    \"\"\"\n    Ünï 😀\n      more\n    \"\"\"\n",
    ),
    (Language::Lua, "--[==[ ünï\n  more ]==]\n"),
];

fn check(language: Language, source: &str) {
    let mut last_end = 0;
    for fragment in fragments(language, source) {
        check_fragment(source, &fragment, language);
        let span = fragment.comment_span();
        assert!(
            span.end() <= source.len(),
            "{language:?}: comment span past end"
        );
        assert!(
            source.is_char_boundary(span.start()) && source.is_char_boundary(span.end()),
            "{language:?}: comment span splits a code point"
        );
        let text = fragment.text_span();
        assert!(text.start() >= span.start() && text.end() <= span.end());
        assert!(
            text.start() >= last_end || span.start() < last_end,
            "fragments out of order"
        );
        last_end = text.end();
    }
}

fn check_fragment(source: &str, fragment: &Fragment, language: Language) {
    let projection = fragment.projection();
    let text = projection.text();
    for (index, ch) in text.chars().enumerate() {
        let span = projection.span_for_range(index, index + 1);
        let wrap = projection
            .tokens()
            .iter()
            .any(|t| t.kind == TokenKind::Wrap && t.text == ch.to_string() && span.is_empty());
        if wrap {
            continue;
        }
        let mapped = source
            .get(span.start()..span.end())
            .unwrap_or_else(|| panic!("{language:?}: span {span:?} is not on code points"));
        assert_eq!(
            mapped,
            ch.to_string(),
            "{language:?}: character {index} ({ch:?}) of {text:?} maps to {mapped:?}"
        );
    }
}

#[test]
fn every_fixture_character_maps_to_its_source_bytes() {
    for fixture in FIXTURES {
        check(fixture.language, fixture.source);
    }
}

#[test]
fn every_edge_source_character_maps_to_its_source_bytes() {
    for (language, source) in EDGE_SOURCES {
        check(*language, source);
    }
}

#[test]
fn a_range_over_a_wrap_spans_the_line_terminator() {
    // "// ab\r\n// cd\r\n": text "ab cd" sits at bytes 3..5 and 10..12.
    let source = "// ab\r\n// cd\r\n";
    let fragments = fragments(Language::Rust, source);
    let projection = fragments[0].projection();
    assert_eq!(projection.text(), "ab cd");
    let across = projection.span_for_range(1, 4);
    assert_eq!((across.start(), across.end()), (4, 11));
    assert_eq!(&source[across.start()..across.end()], "b\r\n// c");
}

#[test]
fn a_range_over_a_wrap_alone_is_empty_at_the_end_of_the_previous_line() {
    let source = "// ab\n// cd\n";
    let fragments = fragments(Language::Rust, source);
    let span = fragments[0].projection().span_for_range(2, 3);
    assert_eq!((span.start(), span.end()), (5, 5));
}

#[test]
fn the_text_span_runs_from_the_first_to_the_last_character() {
    let source = "x();\n    // one\n    // two\n";
    let fragment = &fragments(Language::Rust, source)[0];
    let text = fragment.text_span();
    assert_eq!(&source[text.start()..text.end()], "one\n    // two");
    let comment = fragment.comment_span();
    assert_eq!(
        &source[comment.start()..comment.end()],
        "// one\n    // two"
    );
}

#[test]
fn byte_offsets_after_a_bom_count_the_bom() {
    let source = "\u{feff}// hi\n";
    let fragment = &fragments(Language::Rust, source)[0];
    let span = fragment.projection().span_for_range(0, 2);
    // BOM is bytes 0..3, "// " is 3..6, "hi" is 6..8.
    assert_eq!((span.start(), span.end()), (6, 8));
}

#[test]
fn a_docstring_maps_past_its_quotes_and_indentation() {
    let source = "def f():\n    \"\"\"One\n    two\n    \"\"\"\n";
    let fragment = &fragments(Language::Python, source)[0];
    assert_eq!(fragment.text(), "One two");
    let span = fragment.projection().span_for_range(0, 3);
    assert_eq!(&source[span.start()..span.end()], "One");
    assert_eq!((span.start(), span.end()), (16, 19));
}
