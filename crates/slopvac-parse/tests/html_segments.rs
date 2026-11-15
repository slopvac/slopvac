#![allow(missing_docs)]
//! `parse_html_segments`: several source ranges read as one HTML stream. Expected values
//! are the source text each range covers, worked out from the input.

use slopvac_model::{SourceIndex, Span};
use slopvac_parse::{BlockKind, Document, ParseError, Projection, parse_html, parse_html_segments};

fn spans(source: &str, parts: &[&str]) -> Vec<Span> {
    let mut from = 0;
    parts
        .iter()
        .map(|part| {
            let start = from + source[from..].find(part).expect("part is in the source");
            from = start + part.len();
            Span::new(start, from).unwrap()
        })
        .collect()
}

fn parse(source: &str, parts: &[&str]) -> Document {
    parse_html_segments(source, &SourceIndex::disk(source), &spans(source, parts)).unwrap()
}

fn covered(source: &str, span: Span) -> &str {
    &source[span.start()..span.end()]
}

#[test]
fn a_paragraph_continues_across_a_stripped_prefix_and_maps_to_source() {
    let source = "> <p>one\n> two</p>\n";
    let doc = parse(source, &["<p>one\n", "two</p>"]);
    let [block] = doc.blocks.as_slice() else {
        panic!("one block, got {:?}", doc.blocks);
    };
    let projection = block.projection.as_ref().unwrap();
    assert_eq!(projection.text(), "one two");
    assert_eq!(covered(source, projection.span_for_bytes(0, 3)), "one");
    assert_eq!(covered(source, projection.span_for_bytes(4, 7)), "two");
}

#[test]
fn a_word_split_by_dropped_text_is_one_word_that_maps_to_both_pieces() {
    // The `#` stands for a container prefix the caller dropped.
    let source = "<p>ab#cd ef</p>";
    let doc = parse(source, &["<p>ab", "cd ef</p>"]);
    let projection = doc.blocks[0].projection.as_ref().unwrap();
    assert_eq!(projection.text(), "abcd ef");
    assert_eq!(covered(source, projection.span_for_bytes(0, 4)), "ab#cd");
    assert_eq!(covered(source, projection.span_for_bytes(5, 7)), "ef");
}

#[test]
fn a_pre_element_spans_the_segments_and_hides_its_lines() {
    let source = "> <pre>\n> request the data\n> </pre>\n> <p>after</p>\n";
    let doc = parse(
        source,
        &["<pre>\n", "request the data\n", "</pre>\n", "<p>after</p>"],
    );
    let prose: Vec<&str> = doc
        .blocks
        .iter()
        .filter_map(|b| b.projection.as_ref())
        .map(Projection::text)
        .collect();
    assert_eq!(prose, ["after"]);
    assert!(
        doc.blocks
            .iter()
            .any(|b| matches!(b.kind, BlockKind::CodeBlock { .. }))
    );
}

#[test]
fn a_comment_spans_the_segments_and_hides_its_lines() {
    let source = "> <!-- request\n> the data -->\n> <p>after</p>\n";
    let doc = parse(
        source,
        &["<!-- request\n", "the data -->\n", "<p>after</p>"],
    );
    let prose: Vec<&str> = doc
        .blocks
        .iter()
        .filter_map(|b| b.projection.as_ref())
        .map(Projection::text)
        .collect();
    assert_eq!(prose, ["after"]);
}

#[test]
fn one_segment_reads_like_parse_html() {
    let source = "x <table><tr><td>a &amp; b</td></tr></table> y";
    let body = Span::new(2, 44).unwrap();
    let whole = parse_html(source, body).unwrap();
    let one = parse_html_segments(source, &SourceIndex::disk(source), &[body]).unwrap();
    assert_eq!(whole, one);
}

#[test]
fn segments_that_overlap_or_run_backwards_are_refused() {
    let source = "<p>abc</p>";
    let index = SourceIndex::disk(source);
    let a = Span::new(3, 6).unwrap();
    let b = Span::new(0, 3).unwrap();
    assert_eq!(
        parse_html_segments(source, &index, &[a, b]),
        Err(ParseError::Segments)
    );
}

#[test]
fn a_segment_outside_the_source_is_refused() {
    let source = "<p>abc</p>";
    let past = Span::new(0, 99).unwrap();
    assert!(matches!(
        parse_html_segments(source, &SourceIndex::disk(source), &[past]),
        Err(ParseError::Body(_))
    ));
}
