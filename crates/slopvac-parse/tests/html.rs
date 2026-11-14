#![allow(missing_docs)]
//! Behaviour of `parse_html` through its public interface.
//!
//! Expected values are worked out from the HTML in each test, not from the parser output.

use slopvac_model::Span;
use slopvac_parse::{Block, BlockKind, Document, Projection, TokenKind, parse_html};

fn parse(source: &str) -> Document {
    parse_html(source, Span::new(0, source.len()).unwrap()).unwrap()
}

/// The projected text of every block that has prose.
fn texts(doc: &Document) -> Vec<&str> {
    doc.blocks
        .iter()
        .filter_map(|b| b.projection.as_ref())
        .map(Projection::text)
        .collect()
}

/// The source text a projected range of block `index` maps to.
fn source_of<'a>(source: &'a str, block: &Block, start: usize, end: usize) -> &'a str {
    let span = block
        .projection
        .as_ref()
        .unwrap()
        .span_for_range(start, end);
    &source[span.start()..span.end()]
}

fn kinds(block: &Block) -> Vec<TokenKind> {
    block
        .projection
        .as_ref()
        .unwrap()
        .tokens()
        .iter()
        .map(|t| t.kind)
        .collect()
}

#[test]
fn paragraph_text_and_block_span() {
    let source = "<p>Hello world</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["Hello world"]);
    assert_eq!(doc.blocks[0].kind, BlockKind::Paragraph);
    // The block covers the text only: bytes 3..14.
    assert_eq!(doc.blocks[0].span, Span::new(3, 14).unwrap());
    assert_eq!(source_of(source, &doc.blocks[0], 6, 11), "world");
}

#[test]
fn offsets_count_multibyte_characters_in_bytes() {
    // "é" is 2 bytes and "日" is 3, so "naïve" ends at byte 3 + 6 and "日本" follows.
    let source = "<p>naïve 日本</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["naïve 日本"]);
    assert_eq!(source_of(source, &doc.blocks[0], 6, 8), "日本");
    assert_eq!(doc.blocks[0].span, Span::new(3, 3 + 6 + 1 + 6).unwrap());
}

#[test]
fn entity_decodes_and_maps_to_the_whole_reference() {
    let source = "<p>AT&amp;T rocks</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["AT&T rocks"]);
    let block = &doc.blocks[0];
    assert_eq!(
        kinds(block)[..3],
        [TokenKind::Identity, TokenKind::Entity, TokenKind::Identity]
    );
    // The ampersand is projected point 2; it maps to all five bytes of `&amp;`.
    assert_eq!(source_of(source, block, 2, 3), "&amp;");
    // A range that touches only the T after it does not widen.
    assert_eq!(source_of(source, block, 3, 4), "T");
}

#[test]
fn numeric_and_named_references_decode() {
    let source = "<p>a&#169;b&#x1F600;c&eacute;d</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["a©b😀céd"]);
    let block = &doc.blocks[0];
    assert_eq!(source_of(source, block, 1, 2), "&#169;");
    assert_eq!(source_of(source, block, 3, 4), "&#x1F600;");
    assert_eq!(source_of(source, block, 5, 6), "&eacute;");
}

#[test]
fn malformed_references_follow_the_html_spec() {
    // `&bogus;` is not a reference: it stays literal. `&amp` without a semicolon still
    // decodes. `&#xD800;` is a surrogate and becomes U+FFFD.
    let source = "<p>x&bogus; y&amp z&#xD800;</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["x&bogus; y& z\u{fffd}"]);
    let block = &doc.blocks[0];
    assert_eq!(source_of(source, block, 0, 8), "x&bogus;");
    assert_eq!(source_of(source, block, 10, 11), "&amp");
    assert_eq!(source_of(source, block, 13, 14), "&#xD800;");
}

#[test]
fn a_reference_that_decodes_to_a_space_is_whitespace() {
    let source = "<p>a&#32;&#10;b</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["a b"]);
    assert_eq!(source_of(source, &doc.blocks[0], 1, 2), "&#32;&#10;");
}

#[test]
fn a_no_break_space_stays_a_character() {
    let doc = parse("<p>a&nbsp;b</p>");
    assert_eq!(texts(&doc), ["a\u{a0}b"]);
}

#[test]
fn whitespace_run_is_one_collapse_token_with_its_whole_range() {
    let source = "<p>one \t\n  two</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["one two"]);
    let block = &doc.blocks[0];
    assert_eq!(
        kinds(block),
        [
            TokenKind::Identity,
            TokenKind::Collapse,
            TokenKind::Identity
        ]
    );
    assert_eq!(source_of(source, block, 3, 4), " \t\n  ");
}

#[test]
fn crlf_is_one_terminator_inside_a_whitespace_run() {
    let source = "<p>one\r\ntwo\r\n</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["one two"]);
    assert_eq!(source_of(source, &doc.blocks[0], 3, 4), "\r\n");
    // Trailing whitespace is not part of the block.
    assert_eq!(doc.blocks[0].span, Span::new(3, 11).unwrap());
}

#[test]
fn lone_cr_and_crlf_before_a_tag() {
    let source = "<p>one\r\n<b>two</b>\r</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["one two"]);
    // The run runs through the CRLF; the tag after it is a gap.
    assert_eq!(source_of(source, &doc.blocks[0], 3, 4), "\r\n");
}

#[test]
fn leading_and_trailing_whitespace_is_dropped() {
    let source = "<p>  \n padded \n </p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["padded"]);
    assert_eq!(doc.blocks[0].span, Span::new(7, 13).unwrap());
}

#[test]
fn inline_tags_do_not_split_a_word() {
    let source = "<p>un<b>believ</b>able</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["unbelievable"]);
    // A match across the tags includes the markup between the characters.
    assert_eq!(
        source_of(source, &doc.blocks[0], 0, 12),
        "un<b>believ</b>able"
    );
}

#[test]
fn text_inside_inline_elements_is_prose() {
    // Vale checks the prose inside `td` and `em`. So do we.
    let source = "<table><tr><td>The <em>leverage</em> is high</td></tr></table>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["The leverage is high"]);
    assert_eq!(source_of(source, &doc.blocks[0], 4, 12), "leverage");
}

#[test]
fn a_comment_between_characters_is_a_gap_not_a_word_break() {
    let source = "<p>a<!-- hidden -->b</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["ab"]);
}

#[test]
fn a_comment_holding_markup_hides_it() {
    let doc = parse("<p>before <!-- <p>inside</p> --> after</p>");
    assert_eq!(texts(&doc), ["before after"]);
}

#[test]
fn br_is_a_synthetic_space_at_the_end_of_the_previous_text() {
    let source = "<p>one<br>two</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["one two"]);
    let tokens = doc.blocks[0].projection.as_ref().unwrap().tokens();
    assert_eq!(tokens[1].kind, TokenKind::Wrap);
    // The wrap sits at byte 6, the end of "one", and has no source bytes.
    assert_eq!(tokens[1].src, Span::empty(6));
    // A match on the space alone is an empty span at that point.
    assert_eq!(source_of(source, &doc.blocks[0], 3, 4), "");
}

#[test]
fn br_next_to_whitespace_adds_no_second_space() {
    let doc = parse("<p>one <br> two</p>");
    assert_eq!(texts(&doc), ["one two"]);
}

#[test]
fn script_style_template_noscript_and_head_are_not_prose() {
    let source = "<head><title>T</title><style>p { x: 1 }</style></head>\
        <p>Keep</p><script>var a = '<b>x</b>'; // &amp;</script>\
        <template><p>no</p></template><noscript><p>no</p></noscript><p>Also</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["Keep", "Also"]);
}

#[test]
fn script_text_that_looks_like_a_closing_tag_of_another_element() {
    let doc = parse("<script>if (a < b) { x = '</p>'; }</script><p>Visible</p>");
    assert_eq!(texts(&doc), ["Visible"]);
}

#[test]
fn head_ends_at_the_first_element_that_does_not_belong_in_it() {
    // No `</head>`: the paragraph still counts.
    let doc = parse("<head><title>T</title><p>Body text</p>");
    assert_eq!(texts(&doc), ["Body text"]);
}

#[test]
fn pre_is_a_code_block_with_its_body_span() {
    let source = "<p>Run:</p><pre>cargo test\nok</pre><p>Done</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["Run:", "Done"]);
    let code = doc
        .blocks
        .iter()
        .find(|b| matches!(b.kind, BlockKind::CodeBlock { .. }))
        .unwrap();
    assert_eq!(
        &source[code.span.start()..code.span.end()],
        "<pre>cargo test\nok</pre>"
    );
    let BlockKind::CodeBlock { body, fenced, .. } = &code.kind else {
        unreachable!()
    };
    assert_eq!(&source[body.start()..body.end()], "cargo test\nok");
    assert!(!fenced);
    assert!(code.projection.is_none());
}

#[test]
fn pre_with_inner_code_is_one_block_and_no_code_span() {
    let source = "<pre><code>a &amp; b</code></pre>";
    let doc = parse(source);
    assert_eq!(doc.blocks.len(), 1);
    assert_eq!(doc.code_spans.len(), 0);
}

#[test]
fn inline_code_is_masked_and_listed() {
    let source = "<p>Use <code>foo()</code> now</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["Use \0 now"]);
    assert_eq!(doc.code_spans, [Span::new(7, 25).unwrap()]);
    assert_eq!(
        kinds(&doc.blocks[0]),
        [
            TokenKind::Identity,
            TokenKind::Collapse,
            TokenKind::Masked,
            TokenKind::Collapse,
            TokenKind::Identity
        ]
    );
    assert_eq!(
        source_of(source, &doc.blocks[0], 4, 5),
        "<code>foo()</code>"
    );
}

#[test]
fn kbd_and_samp_are_inline_code() {
    let doc = parse("<p>Press <kbd>Ctrl</kbd> or see <samp>out</samp>.</p>");
    assert_eq!(texts(&doc), ["Press \0 or see \0."]);
    assert_eq!(doc.code_spans.len(), 2);
}

#[test]
fn a_paragraph_of_only_code_is_not_prose() {
    let doc = parse("<p><code>x</code></p>");
    assert_eq!(texts(&doc).len(), 0);
    assert_eq!(doc.code_spans.len(), 1);
}

#[test]
fn hyphenated_words_stay_joined() {
    let source = "<p>A well-known, state-of-the-art tool.</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["A well-known, state-of-the-art tool."]);
}

#[test]
fn headings_carry_their_level() {
    let doc = parse("<h1>Title</h1><p>Body</p><h3>Sub</h3>");
    let got: Vec<_> = doc.blocks.iter().map(|b| b.kind.clone()).collect();
    assert_eq!(
        got,
        [
            BlockKind::Heading { level: 1 },
            BlockKind::Paragraph,
            BlockKind::Heading { level: 3 }
        ]
    );
}

#[test]
fn list_items_blockquote_and_div_are_separate_blocks() {
    let doc = parse(
        "<ul><li>First</li><li>Second <b>bold</b></li></ul>\
         <blockquote>Quoted</blockquote><div>Plain div</div>",
    );
    assert_eq!(texts(&doc), ["First", "Second bold", "Quoted", "Plain div"]);
}

#[test]
fn unclosed_paragraphs_and_items_end_at_the_next_block_tag() {
    let doc = parse("<p>one<p>two<ul><li>a<li>b</ul>tail");
    assert_eq!(texts(&doc), ["one", "two", "a", "b", "tail"]);
}

#[test]
fn blocks_nested_in_a_cell_stay_cell_text() {
    let doc = parse("<table><tr><td>a<p>b</p>c</td></tr></table>");
    let got: Vec<_> = doc
        .blocks
        .iter()
        .map(|b| {
            (
                b.kind.clone(),
                b.projection.as_ref().unwrap().text().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (BlockKind::TableCell { row: 0, column: 0 }, "a".to_owned()),
            (BlockKind::TableCell { row: 0, column: 0 }, "b".to_owned()),
            (BlockKind::TableCell { row: 0, column: 0 }, "c".to_owned()),
        ]
    );
}

#[test]
fn table_cells_know_their_row_and_column() {
    let doc = parse("<table><tr><th>H1</th><th>H2</th></tr><tr><td>a</td><td>b</td></tr></table>");
    let got: Vec<_> = doc
        .blocks
        .iter()
        .map(|b| {
            (
                b.kind.clone(),
                b.projection.as_ref().unwrap().text().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (BlockKind::TableCell { row: 0, column: 0 }, "H1".to_owned()),
            (BlockKind::TableCell { row: 0, column: 1 }, "H2".to_owned()),
            (BlockKind::TableCell { row: 1, column: 0 }, "a".to_owned()),
            (BlockKind::TableCell { row: 1, column: 1 }, "b".to_owned()),
        ]
    );
}

#[test]
fn a_nested_table_does_not_disturb_the_outer_counters() {
    let doc =
        parse("<table><tr><td>x<table><tr><td>in</td></tr></table></td><td>y</td></tr></table>");
    let cells: Vec<_> = doc
        .blocks
        .iter()
        .map(|b| {
            (
                b.kind.clone(),
                b.projection.as_ref().unwrap().text().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        cells,
        [
            (BlockKind::TableCell { row: 0, column: 0 }, "x".to_owned()),
            (BlockKind::TableCell { row: 0, column: 0 }, "in".to_owned()),
            (BlockKind::TableCell { row: 0, column: 1 }, "y".to_owned()),
        ]
    );
}

#[test]
fn a_byte_order_mark_is_not_text_but_offsets_count_it() {
    let source = "\u{feff}<p>Hi</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["Hi"]);
    // BOM is 3 bytes, `<p>` 3 more.
    assert_eq!(doc.blocks[0].span, Span::new(6, 8).unwrap());
}

#[test]
fn positions_are_relative_to_the_whole_source_when_the_body_is_a_range() {
    let source = "---\ntitle: x\n---\n<p>R&amp;D</p>";
    let body = Span::new(17, source.len()).unwrap();
    assert_eq!(&source[17..], "<p>R&amp;D</p>");
    let doc = parse_html(source, body).unwrap();
    assert_eq!(texts(&doc), ["R&D"]);
    assert_eq!(source_of(source, &doc.blocks[0], 1, 2), "&amp;");
}

#[test]
fn a_body_that_does_not_fit_is_an_error() {
    let bad = Span::new(0, 99).unwrap();
    assert!(parse_html("<p>x</p>", bad).is_err());
}

#[test]
fn text_with_no_tags_is_one_paragraph() {
    let doc = parse("just words\nover two lines");
    assert_eq!(texts(&doc), ["just words over two lines"]);
}

#[test]
fn empty_input_has_no_blocks() {
    assert_eq!(parse("").blocks.len(), 0);
}

#[test]
fn unclosed_inline_code_masks_to_the_end() {
    let source = "<p>see <code>never closed";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["see \0"]);
    assert_eq!(doc.code_spans, [Span::new(7, source.len()).unwrap()]);
}

#[test]
fn a_stray_end_tag_for_a_skipped_element_is_ignored() {
    let doc = parse("</pre></code><p>Kept</p></script>");
    assert_eq!(texts(&doc), ["Kept"]);
}

#[test]
fn an_unfinished_tag_at_the_end_is_a_gap() {
    let source = "<p>done <b";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["done"]);
}

#[test]
fn a_self_closing_slash_on_a_non_void_element_is_ignored() {
    // The HTML spec opens `<code>` here, so " b" is code. The paragraph ends, and a browser
    // reopens the code in the next one, so "c" is code too.
    let doc = parse("<p>a <code/> b</p><p>c</p>");
    assert_eq!(texts(&doc), ["a \0"]);
}

#[test]
fn an_end_tag_closes_the_nearest_open_element_of_its_name() {
    // `</pre>` has no open `<pre>`, so it does nothing and the code stays open until
    // `</code>`. The `y words` after it is prose again.
    let doc = parse("<p>x <code>c</pre> d</code> y words</p>");
    assert_eq!(texts(&doc), ["x \0 y words"]);
}

#[test]
fn text_after_head_metadata_is_prose_without_a_body_tag() {
    let source = "<title>T</title><meta charset=utf-8>Body text with no body tag.";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["Body text with no body tag."]);
    assert_eq!(source_of(source, &doc.blocks[0], 0, 4), "Body");
}

#[test]
fn text_inside_an_unclosed_head_ends_the_head() {
    let doc = parse("<head><script>x</script>Visible words");
    assert_eq!(texts(&doc), ["Visible words"]);
}

#[test]
fn a_replaced_character_keeps_the_text_and_the_positions_after_it() {
    // The tokenizer turns the NUL in RCDATA into U+FFFD, so the text no longer matches the
    // source byte for byte. The replacement maps to the NUL, and the text around it and
    // after it still maps to its own bytes.
    let source = "<textarea>a\0b</textarea><p>after</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["a\u{fffd}b", "after"]);
    assert_eq!(
        kinds(&doc.blocks[0]),
        [TokenKind::Identity, TokenKind::Opaque, TokenKind::Identity]
    );
    assert_eq!(source_of(source, &doc.blocks[0], 1, 2), "\0");
    assert_eq!(source_of(source, &doc.blocks[1], 0, 5), "after");
}

#[test]
fn prose_after_an_inner_table_returns_to_the_outer_cell() {
    let source = "<table><tr><td>a<table><tr><td>b</td></tr></table>c</td></tr></table>";
    let doc = parse(source);
    let got: Vec<_> = doc
        .blocks
        .iter()
        .map(|b| {
            (
                b.kind.clone(),
                b.projection.as_ref().unwrap().text().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (BlockKind::TableCell { row: 0, column: 0 }, "a".to_owned()),
            (BlockKind::TableCell { row: 0, column: 0 }, "b".to_owned()),
            (BlockKind::TableCell { row: 0, column: 0 }, "c".to_owned()),
        ]
    );
}

#[test]
fn cdata_in_svg_is_text_and_maps_to_its_content() {
    let source = "<svg><![CDATA[x < y & z]]></svg><p>after</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["x < y & z", "after"]);
    // The delimiters are gaps: the span covers the content only.
    assert_eq!(source_of(source, &doc.blocks[0], 0, 9), "x < y & z");
}

#[test]
fn cdata_in_html_is_a_comment() {
    let doc = parse("<p>a<![CDATA[hidden]]>b</p>");
    assert_eq!(texts(&doc), ["ab"]);
}

#[test]
fn a_title_is_metadata_wherever_it_sits() {
    let doc = parse("<p>one</p><title>not prose</title><p>two</p>");
    assert_eq!(texts(&doc), ["one", "two"]);
}

#[test]
fn an_end_tag_with_nothing_open_does_not_split_a_block() {
    // A browser ignores `</div>` here, so the text stays together.
    let doc = parse("<p>before</div>after</p>");
    assert_eq!(texts(&doc), ["beforeafter"]);
}

#[test]
fn an_empty_self_closed_svg_title_does_not_hide_what_follows() {
    // `<title/>` in SVG is a complete element. The text after it is still SVG text.
    let doc = parse("<svg><title/><text>after</text></svg>");
    assert_eq!(texts(&doc), ["after"]);
}

#[test]
fn cdata_at_an_svg_integration_point_is_text() {
    // The current node is an SVG element, so a CDATA section is text, even though HTML
    // rules apply to the tags inside `<desc>`.
    let source = "<svg><desc><![CDATA[x < y]]></desc></svg>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["x < y"]);
    assert_eq!(source_of(source, &doc.blocks[0], 0, 5), "x < y");
}

#[test]
fn cells_keep_their_own_row_and_column_when_end_tags_are_omitted() {
    // Nothing closes the inner cells or the inner table before `</table>`.
    let source = "<table><tr><td>a</td><td>b<table><tr><td>x<td>y</table></table>";
    let doc = parse(source);
    let got: Vec<_> = doc
        .blocks
        .iter()
        .map(|b| {
            (
                b.kind.clone(),
                b.projection.as_ref().unwrap().text().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (BlockKind::TableCell { row: 0, column: 0 }, "a".to_owned()),
            (BlockKind::TableCell { row: 0, column: 1 }, "b".to_owned()),
            (BlockKind::TableCell { row: 0, column: 0 }, "x".to_owned()),
            (BlockKind::TableCell { row: 0, column: 1 }, "y".to_owned()),
        ]
    );
}

#[test]
fn text_a_table_pulls_forward_stays_in_source_order() {
    // A tree builder moves "stray" in front of the table. Blocks still come in source order
    // and each projection maps to its own bytes.
    let source = "<table><tr>stray<td>cell</td></tr></table>tail";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["stray", "cell", "tail"]);
    assert_eq!(source_of(source, &doc.blocks[0], 0, 5), "stray");
}

#[test]
fn a_comment_after_the_last_child_belongs_to_the_code_span() {
    let source = "<p>a <code>x<!-- c --></code> b</p>";
    let doc = parse(source);
    // `<code>` starts at byte 5 and `</code>` ends at 29.
    assert_eq!(&source[5..29], "<code>x<!-- c --></code>");
    assert_eq!(doc.code_spans, [Span::new(5, 29).unwrap()]);
    assert_eq!(texts(&doc), ["a \0 b"]);
    assert_eq!(
        source_of(source, &doc.blocks[0], 2, 3),
        "<code>x<!-- c --></code>"
    );
}

#[test]
fn a_comment_after_the_last_child_belongs_to_the_pre_block() {
    let source = "<pre>x<!-- c --></pre>";
    let doc = parse(source);
    let BlockKind::CodeBlock { body, .. } = &doc.blocks[0].kind else {
        unreachable!()
    };
    assert_eq!(&source[body.start()..body.end()], "x<!-- c -->");
    assert_eq!(doc.blocks[0].span, Span::new(0, source.len()).unwrap());
}

#[test]
fn the_surviving_newline_keeps_its_own_bytes_after_one_is_dropped() {
    // A browser drops the first newline of a textarea. The second one is the content.
    let source = "<p>a<textarea>\n\n</textarea>b</p>";
    let doc = parse(source);
    assert_eq!(texts(&doc), ["a b"]);
    let tokens = doc.blocks[0].projection.as_ref().unwrap().tokens();
    assert_eq!(tokens[1].kind, TokenKind::Collapse);
    // `<p>a<textarea>` is 14 bytes: the dropped newline is byte 14 and the kept one 15.
    assert_eq!(tokens[1].src, Span::new(15, 16).unwrap());
}
