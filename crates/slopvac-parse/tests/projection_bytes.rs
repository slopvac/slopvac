#![allow(missing_docs)]
//! `Projection::span_for_bytes` takes a range in bytes of the projected text, which is what a
//! regex match reports, and returns the source span.

use slopvac_model::Span;
use slopvac_parse::parse_html;

/// The projected text of the only block, and the source span for a byte range of it.
fn span_of(source: &str, word: &str) -> (usize, usize, Span) {
    let doc = parse_html(source, Span::new(0, source.len()).unwrap()).unwrap();
    let projection = doc.blocks[0].projection.as_ref().unwrap();
    let start = projection.text().find(word).unwrap();
    let end = start + word.len();
    (start, end, projection.span_for_bytes(start, end))
}

#[test]
fn byte_offsets_after_multibyte_characters_map_to_the_word() {
    // In the projected text "über — naïve 😀 text" the word starts at byte 22: ü is 2 bytes,
    // the dash 3, ï 2 and the emoji 4. It is code point 17. The `<p>` is 3 source bytes.
    let source = "<p>über — naïve 😀 text</p>";
    let (start, end, span) = span_of(source, "text");
    assert_eq!((start, end), (22, 26));
    assert_eq!(span, Span::new(25, 29).unwrap());
    assert_eq!(&source[span.start()..span.end()], "text");
}

#[test]
fn byte_offsets_after_an_entity_map_to_the_word() {
    // "xéy word": the entity decodes to 2 bytes and is 8 source bytes, so the word starts at
    // byte 5 of the projected text and byte 14 of the source.
    let source = "<p>x&eacute;y word</p>";
    let (start, end, span) = span_of(source, "word");
    assert_eq!((start, end), (5, 9));
    assert_eq!(span, Span::new(14, 18).unwrap());
    assert_eq!(&source[span.start()..span.end()], "word");
}

#[test]
fn a_range_that_covers_an_entity_widens_to_all_of_it() {
    let source = "<p>x&eacute;y word</p>";
    // Bytes 1..3 of "xéy word" are the decoded é.
    let doc = parse_html(source, Span::new(0, source.len()).unwrap()).unwrap();
    let span = doc.blocks[0]
        .projection
        .as_ref()
        .unwrap()
        .span_for_bytes(1, 3);
    assert_eq!(&source[span.start()..span.end()], "&eacute;");
}
