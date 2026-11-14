#![allow(missing_docs)]
//! Regressions found in review. Each test goes through the public parse functions.

use slopvac_model::Span;
use slopvac_parse::{BlockKind, Document, TokenKind, parse_markdown, parse_mdx};

fn whole(source: &str) -> Span {
    Span::new(0, source.len()).unwrap()
}

fn md(source: &str) -> Document {
    parse_markdown(source, whole(source)).unwrap()
}

fn mdx(source: &str) -> Document {
    parse_mdx(source, whole(source)).unwrap()
}

fn slice(source: &str, span: Span) -> &str {
    &source[span.start()..span.end()]
}

fn prose(doc: &Document) -> Vec<&str> {
    doc.blocks
        .iter()
        .filter_map(|b| b.projection.as_ref())
        .map(slopvac_parse::Projection::text)
        .collect()
}

fn admonitions(doc: &Document) -> usize {
    doc.blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Admonition(_)))
        .count()
}

fn code_bodies(doc: &Document) -> Vec<Span> {
    doc.blocks
        .iter()
        .filter_map(|b| match &b.kind {
            BlockKind::CodeBlock { body, .. } => Some(*body),
            _ => None,
        })
        .collect()
}

#[test]
fn js_comments_in_mdx_expressions_may_hold_quotes_and_braces() {
    let cases = [
        "a {/* don't */} b\n",
        "a {/* } */} b\n",
        "a {\n// it's\n} b\n",
        "a {/* \" ` */ 'x'} b\n",
    ];
    for source in cases {
        assert!(parse_mdx(source, whole(source)).is_ok(), "{source:?}");
    }
}

#[test]
fn a_deeply_braced_mdx_expression_is_checked_in_linear_time() {
    let n = 40_000;
    let source = format!("{{{}{}}}\n", "{x".repeat(n), "}".repeat(n));
    let start = std::time::Instant::now();
    // Too many closing braces in one expression is an error, not a slow parse.
    assert!(parse_mdx(&source, whole(&source)).is_err());
    assert!(start.elapsed().as_secs() < 5, "took {:?}", start.elapsed());
}

#[test]
fn many_sibling_braces_in_one_expression_are_fine() {
    let source = format!("a {{{}}} b\n", "{x}".repeat(200));
    assert!(parse_mdx(&source, whole(&source)).is_ok());
}

#[test]
fn a_comment_then_a_soft_break_still_separates_words() {
    let doc = md("one <!--c-->\ntwo\n\nthree<!--c-->\nfour\n");
    assert_eq!(prose(&doc), ["one two", "three four"]);
}

#[test]
fn a_trailing_comment_leaves_no_trailing_wrap_or_blank() {
    let doc = md("word\n<!--c-->\n\n# Head <!--c-->\n");
    assert_eq!(prose(&doc), ["word", "Head"]);
}

#[test]
fn a_hard_break_after_only_a_comment_adds_no_space() {
    let doc = md("<!--c-->  \nword\n");
    assert_eq!(prose(&doc), ["word"]);
}

#[test]
fn a_standalone_string_expression_is_prose() {
    let source = "{'Rendered text here.'}\n\nNext.\n";
    let doc = mdx(source);
    let block = &doc.blocks[0];
    assert_eq!(block.kind, BlockKind::Paragraph);
    assert_eq!(slice(source, block.span), "{'Rendered text here.'}");
    let proj = block.projection.as_ref().unwrap();
    assert_eq!(proj.text(), "Rendered text here.");
    assert_eq!(proj.tokens()[0].kind, TokenKind::Opaque);
}

#[test]
fn a_standalone_expression_that_is_not_a_string_renders_nothing() {
    assert_eq!(mdx("{props.children}\n").blocks, []);
}

#[test]
fn long_runs_of_ampersands_parse_in_linear_time() {
    let source = format!("{}\n", "&".repeat(600_000));
    let start = std::time::Instant::now();
    let (a, b) = (mdx(&source), md(&source));
    assert!(start.elapsed().as_secs() < 5, "took {:?}", start.elapsed());
    assert_eq!(prose(&a)[0].len(), 600_000);
    assert_eq!(prose(&b)[0].len(), 600_000);
}

#[test]
fn a_code_body_in_a_block_quote_excludes_the_closing_fence() {
    let cases = [
        ("> ```js\n> x\n> ```\n", "> x\n"),
        ("> > ```\n> > a\n> > b\n> > ```\n", "> > a\n> > b\n"),
        ("- item\n\n  > ```\n  > x\n  > ```\n", "  > x\n"),
    ];
    for (source, want) in cases {
        for doc in [md(source), mdx(source)] {
            let bodies = code_bodies(&doc);
            assert_eq!(bodies.len(), 1, "{source:?}");
            assert_eq!(slice(source, bodies[0]), want, "{source:?}");
        }
    }
}

#[test]
fn a_fence_in_an_admonition_body_has_an_exact_code_body() {
    let source = "!!! note\n    ```py\n    x = 1\n    ```\n";
    let doc = md(source);
    let bodies = code_bodies(&doc);
    assert_eq!(bodies.len(), 1);
    assert_eq!(slice(source, bodies[0]), "    x = 1\n");
}

#[test]
fn an_over_indented_fence_line_does_not_close_a_fence() {
    // The four-space line is code, so the marker lines after it are code too.
    let source = "```\ncode\n    ```\n::: note\ntext\n:::\n```\n";
    let doc = md(source);
    assert_eq!(admonitions(&doc), 0);
    assert_eq!(code_bodies(&doc).len(), 1);
}

#[test]
fn mdx_has_no_indented_code_so_an_indented_fence_line_closes() {
    // markdown-rs lets a closer use any indent when indented code is off.
    let source = "```\ncode\n    ```\n::: note\ntext\n:::\n";
    let doc = mdx(source);
    assert_eq!(admonitions(&doc), 1);
    assert_eq!(code_bodies(&doc).len(), 1);
}

#[test]
fn markers_inside_a_deeply_nested_fence_are_not_admonitions() {
    let source = "1. a\n   - b\n\n     ```\n     ::: note\n     text\n     :::\n     ```\n";
    assert_eq!(admonitions(&md(source)), 0);
}

#[test]
fn a_two_code_point_entity_followed_by_its_own_letters_stays_exact() {
    let source = "x&fjlig;jy\n";
    for doc in [md(source), mdx(source)] {
        let proj = doc.blocks[0].projection.as_ref().unwrap();
        assert_eq!(proj.text(), "xfjjy");
        let kinds: Vec<_> = proj.tokens().iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            [TokenKind::Identity, TokenKind::Entity, TokenKind::Identity]
        );
        // A match on the second letter of the entity covers the whole entity.
        assert_eq!(slice(source, proj.span_for_range(2, 3)), "&fjlig;");
    }
}

#[test]
fn an_unknown_or_double_encoded_entity_is_literal_or_exact() {
    let source = "a &nosuchentity; b &amp;amp; c\n";
    for doc in [md(source), mdx(source)] {
        let proj = doc.blocks[0].projection.as_ref().unwrap();
        assert_eq!(proj.text(), "a &nosuchentity; b &amp; c");
        // The decoded `&` maps to the first reference only.
        let at = proj.text().rfind("&amp;").unwrap();
        let start = proj.text()[..at].chars().count();
        assert_eq!(
            slice(source, proj.span_for_range(start, start + 1)),
            "&amp;"
        );
    }
}

#[test]
fn deeply_nested_bang_blocks_parse_in_linear_time() {
    let mut source = String::new();
    for depth in 0..3000 {
        source.push_str(&" ".repeat(4 * depth));
        source.push_str("!!! note\n");
    }
    for _ in 0..3000 {
        source.push_str(&" ".repeat(4 * 2999));
        source.push_str("text\n\n");
    }
    let start = std::time::Instant::now();
    let _ = md(&source);
    assert!(start.elapsed().as_secs() < 10, "took {:?}", start.elapsed());
}
