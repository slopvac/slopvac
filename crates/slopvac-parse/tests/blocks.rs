#![allow(missing_docs)]
//! Block structure of Markdown and MDX. Expected spans are checked by the source text
//! they cover, which is worked out from the input, not from the parser.

use slopvac_model::Span;
use slopvac_parse::{
    Admonition, AdmonitionSyntax, Block, BlockKind, Container, Document, ParseError, TokenKind,
    parse_markdown, parse_mdx,
};

fn whole(source: &str) -> Span {
    Span::new(0, source.len()).unwrap()
}

fn md(source: &str) -> Document {
    parse_markdown(source, whole(source)).unwrap()
}

fn mdx(source: &str) -> Document {
    parse_mdx(source, whole(source)).unwrap()
}

fn text<'a>(source: &'a str, block: &Block) -> &'a str {
    &source[block.span.start()..block.span.end()]
}

fn slice(source: &str, span: Span) -> &str {
    &source[span.start()..span.end()]
}

fn admonition(doc: &Document) -> (&Block, &Admonition) {
    doc.blocks
        .iter()
        .find_map(|b| match &b.kind {
            BlockKind::Admonition(a) => Some((b, a)),
            _ => None,
        })
        .expect("an admonition block")
}

fn paragraphs<'a>(source: &'a str, doc: &Document) -> Vec<&'a str> {
    doc.blocks
        .iter()
        .filter(|b| b.kind == BlockKind::Paragraph)
        .map(|b| text(source, b))
        .collect()
}

// ---- the body range ----

#[test]
fn only_the_body_is_parsed_and_positions_stay_in_the_whole_source() {
    let front = "---\ntitle: é *x*\n---\n";
    let source = format!("{front}Body text.\n");
    let body = Span::new(front.len(), source.len()).unwrap();
    let doc = parse_markdown(&source, body).unwrap();
    assert_eq!(doc.blocks.len(), 1);
    assert_eq!(text(&source, &doc.blocks[0]), "Body text.");
    assert_eq!(doc.blocks[0].span.start(), front.len());
}

#[test]
fn a_body_that_splits_a_code_point_is_rejected() {
    let source = "é text";
    let body = Span::new(1, source.len()).unwrap();
    assert!(matches!(
        parse_markdown(source, body),
        Err(ParseError::Body(_))
    ));
    assert!(matches!(parse_mdx(source, body), Err(ParseError::Body(_))));
}

#[test]
fn a_body_past_the_end_is_rejected() {
    let body = Span::new(0, 99).unwrap();
    assert!(matches!(
        parse_markdown("short", body),
        Err(ParseError::Body(_))
    ));
}

#[test]
fn a_byte_order_mark_is_not_content() {
    let source = "\u{feff}Hello there.\n";
    let doc = md(source);
    assert_eq!(text(source, &doc.blocks[0]), "Hello there.");
    assert_eq!(doc.blocks[0].span.start(), 3);
}

// ---- headings and prose blocks ----

#[test]
fn heading_attributes_are_not_prose() {
    let source = "## Install {#install .big}\n";
    for doc in [md(source), mdx(source)] {
        let heading = &doc.blocks[0];
        assert_eq!(heading.kind, BlockKind::Heading { level: 2 });
        assert_eq!(heading.projection.as_ref().unwrap().text(), "Install");
        let span = heading
            .projection
            .as_ref()
            .unwrap()
            .span_for_range(0, "Install".len());
        assert_eq!(slice(source, span), "Install");
    }
}

#[test]
fn tight_list_items_are_paragraphs_with_exact_spans() {
    let source = "- one\n- two *x*\n";
    let doc = md(source);
    assert_eq!(paragraphs(source, &doc), ["one", "two *x*"]);
}

#[test]
fn image_alt_text_is_not_prose() {
    let source = "![alt words](pic.png) after\n";
    let doc = md(source);
    assert_eq!(doc.blocks[0].projection.as_ref().unwrap().text(), " after");
}

// ---- tables ----

#[test]
fn tables_expose_cells_with_row_and_column() {
    let source = "| a | b |\n|---|---|\n| c | d |\n";
    for doc in [md(source), mdx(source)] {
        assert_eq!(doc.blocks[0].kind, BlockKind::Table);
        assert_eq!(text(source, &doc.blocks[0]), source.trim_end());
        let cells: Vec<_> = doc.blocks[1..]
            .iter()
            .map(|b| {
                let BlockKind::TableCell { row, column } = b.kind else {
                    panic!("expected a cell, got {:?}", b.kind)
                };
                let proj = b.projection.as_ref().unwrap();
                let span = proj.span_for_range(0, 1);
                (row, column, proj.text().to_owned(), slice(source, span))
            })
            .collect();
        assert_eq!(
            cells,
            [
                (0, 0, "a".to_owned(), "a"),
                (0, 1, "b".to_owned(), "b"),
                (1, 0, "c".to_owned(), "c"),
                (1, 1, "d".to_owned(), "d"),
            ]
        );
    }
}

// ---- code blocks ----

#[test]
fn fenced_code_exposes_language_and_body_range() {
    let source = "Intro\n\n```rust,ignore title=x\nfn é() {}\nlet y;\n```\n\nOutro\n";
    for doc in [md(source), mdx(source)] {
        let code = doc
            .blocks
            .iter()
            .find(|b| matches!(b.kind, BlockKind::CodeBlock { .. }))
            .unwrap();
        let BlockKind::CodeBlock {
            lang,
            info,
            body,
            fenced,
        } = &code.kind
        else {
            unreachable!()
        };
        assert_eq!(lang.as_deref(), Some("rust,ignore"));
        assert_eq!(info.as_deref(), Some("rust,ignore title=x"));
        assert!(fenced);
        assert_eq!(slice(source, *body), "fn é() {}\nlet y;\n");
        assert!(text(source, code).starts_with("```rust"));
        assert!(text(source, code).ends_with("```"));
    }
}

#[test]
fn code_body_range_edge_cases() {
    // (source, body text)
    let cases = [
        ("```\n```\n", ""),
        ("```py\n\n```\n", "\n"),
        ("~~~~\na\n~~~\nb\n~~~~\n", "a\n~~~\nb\n"),
        ("```\nunclosed\n", "unclosed\n"),
        ("```\r\na\r\nb\r\n```\r\n", "a\r\nb\r\n"),
        ("- item\n\n  ```js\n  x\n  ```\n", "x\n"),
    ];
    for (source, want) in cases {
        let doc = md(source);
        let body = doc
            .blocks
            .iter()
            .find_map(|b| match &b.kind {
                BlockKind::CodeBlock { body, .. } => Some(*body),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no code block in {source:?}"));
        assert_eq!(slice(source, body), want, "{source:?}");
    }
}

#[test]
fn indented_code_is_a_block_without_info() {
    let source = "Text.\n\n    let x = 1;\n";
    let doc = md(source);
    let BlockKind::CodeBlock { lang, fenced, .. } = &doc.blocks[1].kind else {
        panic!("expected code")
    };
    assert_eq!((lang.as_deref(), *fenced), (None, false));
}

#[test]
fn code_in_a_fence_is_not_admonition_syntax() {
    let source = "```md\n::: note\ntext\n:::\n```\n";
    let doc = md(source);
    assert!(
        !doc.blocks
            .iter()
            .any(|b| matches!(b.kind, BlockKind::Admonition(_)))
    );
}

// ---- inline code ----

#[test]
fn inline_code_spans_are_ranges_with_backticks() {
    let source = "Use `a` and ``b`c`` here\n\n| x |\n|---|\n| `d` |\n";
    for doc in [md(source), mdx(source)] {
        let spans: Vec<_> = doc.code_spans.iter().map(|s| slice(source, *s)).collect();
        assert_eq!(spans, ["`a`", "``b`c``", "`d`"]);
    }
}

// ---- admonitions ----

#[test]
fn colon_admonition_has_type_title_and_masked_markers() {
    let source = "Before.\n\n::: warning \"Careful now\"\nInside text.\n:::\n\nAfter.\n";
    for doc in [md(source), mdx(source)] {
        let (block, adm) = admonition(&doc);
        assert_eq!(adm.kind, "warning");
        assert_eq!(adm.title.as_deref(), Some("Careful now"));
        assert_eq!(slice(source, adm.title_span.unwrap()), "Careful now");
        assert_eq!(adm.syntax, AdmonitionSyntax::Colon);
        assert_eq!(
            text(source, block),
            "::: warning \"Careful now\"\nInside text.\n:::"
        );
        assert_eq!(
            paragraphs(source, &doc),
            ["Before.", "Inside text.", "After."]
        );
    }
}

#[test]
fn colon_admonition_spellings() {
    // (source, kind, title)
    let cases = [
        (":::note\nx\n:::\n", "note", None),
        (":::{tip} A title\nx\n:::\n", "tip", Some("A title")),
        (
            ":::{admonition} Own title\nx\n:::\n",
            "admonition",
            Some("Own title"),
        ),
        (":::note[Bracketed]\nx\n:::\n", "note", Some("Bracketed")),
        ("::: Danger\nx\n:::\n", "danger", None),
        (":::: info\nx\n::::\n", "info", None),
    ];
    for (source, kind, title) in cases {
        let doc = md(source);
        let (_, adm) = admonition(&doc);
        assert_eq!(adm.kind, kind, "{source:?}");
        assert_eq!(adm.title.as_deref(), title, "{source:?}");
    }
}

#[test]
fn nested_colon_admonitions_nest_by_span() {
    let source = ":::: outer\n::: inner\ntext\n:::\n::::\n";
    let doc = md(source);
    let kinds: Vec<_> = doc
        .blocks
        .iter()
        .filter_map(|b| match &b.kind {
            BlockKind::Admonition(a) => Some((a.kind.as_str(), text(source, b))),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        [
            ("outer", source.trim_end()),
            ("inner", "::: inner\ntext\n:::")
        ]
    );
}

#[test]
fn bang_admonition_body_keeps_exact_positions_after_indent_removal() {
    let source =
        "!!! note \"Heads up\"\n    First é line\n    second line\n\n    Third.\n\nOutside.\n";
    for doc in [md(source), mdx(source)] {
        let (block, adm) = admonition(&doc);
        assert_eq!(adm.kind, "note");
        assert_eq!(adm.title.as_deref(), Some("Heads up"));
        assert_eq!(adm.syntax, AdmonitionSyntax::Bang);
        assert_eq!(
            text(source, block),
            "!!! note \"Heads up\"\n    First é line\n    second line\n\n    Third."
        );
        assert_eq!(
            paragraphs(source, &doc),
            ["First é line\n    second line", "Third.", "Outside."]
        );
        // The word "second" maps to its own bytes, past the removed indentation.
        let proj = doc.blocks[1].projection.as_ref().unwrap();
        let at = proj.text()[..proj.text().find("second").unwrap()]
            .chars()
            .count();
        let span = proj.span_for_range(at, at + "second".len());
        assert_eq!(slice(source, span), "second");
    }
}

#[test]
fn collapsible_bang_admonition_without_title() {
    let source = "???+ tip\n    Body.\n";
    let doc = md(source);
    let (_, adm) = admonition(&doc);
    assert_eq!((adm.kind.as_str(), adm.title.as_deref()), ("tip", None));
}

#[test]
fn fenced_directive_is_an_admonition_and_other_directives_stay_code() {
    let source = "```{note} Mind this\nSome *text*.\n```\n\n```{code-block} python\nx = 1\n```\n";
    let doc = md(source);
    let (block, adm) = admonition(&doc);
    assert_eq!(adm.kind, "note");
    assert_eq!(adm.title.as_deref(), Some("Mind this"));
    assert_eq!(adm.syntax, AdmonitionSyntax::Directive);
    assert_eq!(
        text(source, block),
        "```{note} Mind this\nSome *text*.\n```"
    );
    let code = doc
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::CodeBlock { .. }))
        .count();
    assert_eq!(code, 1);
}

#[test]
fn gfm_alert_and_obsidian_callout() {
    let source = "> [!NOTE]\n> Plain alert.\n\n> [!tip]- Folded title\n> Body.\n\n> not an alert\n";
    for doc in [md(source), mdx(source)] {
        let found: Vec<_> = doc
            .blocks
            .iter()
            .filter_map(|b| match &b.kind {
                BlockKind::Admonition(a) => Some((
                    a.kind.as_str(),
                    a.title.as_deref(),
                    a.syntax,
                    text(source, b),
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            found,
            [
                (
                    "note",
                    None,
                    AdmonitionSyntax::Alert,
                    "> [!NOTE]\n> Plain alert."
                ),
                (
                    "tip",
                    Some("Folded title"),
                    AdmonitionSyntax::Alert,
                    "> [!tip]- Folded title\n> Body."
                ),
            ]
        );
        assert_eq!(
            paragraphs(source, &doc),
            ["Plain alert.", "Body.", "not an alert"]
        );
    }
}

#[test]
fn an_unclosed_container_runs_to_the_last_content_line() {
    let source = "::: note\ntext\n\n\n";
    let doc = md(source);
    let (block, _) = admonition(&doc);
    assert_eq!(text(source, block), "::: note\ntext");
}

// ---- projection tokens ----

fn kinds_and_texts(block: &Block) -> Vec<(TokenKind, String)> {
    block
        .projection
        .as_ref()
        .unwrap()
        .tokens()
        .iter()
        .map(|t| (t.kind, t.text.clone()))
        .collect()
}

#[test]
fn markdown_emits_the_tokens_the_position_vectors_assume() {
    type Case<'a> = (&'a str, &'a str, Vec<(TokenKind, &'a str)>);
    // (source, projected text, token kinds with their text) from positions/vectors.json.
    let cases: [Case<'_>; 6] = [
        (
            "AT&amp;T rocks",
            "AT&T rocks",
            vec![
                (TokenKind::Identity, "AT"),
                (TokenKind::Entity, "&"),
                (TokenKind::Identity, "T rocks"),
            ],
        ),
        (
            "a&#x1F600;b",
            "a\u{1F600}b",
            vec![
                (TokenKind::Identity, "a"),
                (TokenKind::Entity, "\u{1F600}"),
                (TokenKind::Identity, "b"),
            ],
        ),
        (
            "a\\*b\\*",
            "a*b*",
            vec![
                (TokenKind::Identity, "a"),
                (TokenKind::Backslash, "*"),
                (TokenKind::Identity, "b"),
                (TokenKind::Backslash, "*"),
            ],
        ),
        (
            "foo\nbar baz",
            "foo bar baz",
            vec![
                (TokenKind::Identity, "foo"),
                (TokenKind::Wrap, " "),
                (TokenKind::Identity, "bar baz"),
            ],
        ),
        (
            "foo\r\nbar",
            "foo bar",
            vec![
                (TokenKind::Identity, "foo"),
                (TokenKind::Wrap, " "),
                (TokenKind::Identity, "bar"),
            ],
        ),
        (
            "use `foo` now",
            "use \0 now",
            vec![
                (TokenKind::Identity, "use "),
                (TokenKind::Masked, "\0"),
                (TokenKind::Identity, " now"),
            ],
        ),
    ];
    for (source, projected, tokens) in cases {
        let block = &md(source).blocks[0];
        let want: Vec<_> = tokens.into_iter().map(|(k, t)| (k, t.to_owned())).collect();
        assert_eq!(kinds_and_texts(block), want, "{source:?}");
        assert_eq!(block.projection.as_ref().unwrap().text(), projected);
    }
}

#[test]
fn markdown_token_spans_match_the_vectors() {
    let source = "AT&amp;T rocks";
    let block = &md(source).blocks[0];
    let spans: Vec<_> = block
        .projection
        .as_ref()
        .unwrap()
        .tokens()
        .iter()
        .map(|t| (t.src.start(), t.src.end()))
        .collect();
    assert_eq!(spans, [(0, 2), (2, 7), (7, 14)]);

    let source = "foo\r\nbar";
    let tokens = md(source).blocks[0].projection.clone().unwrap();
    let spans: Vec<_> = tokens
        .tokens()
        .iter()
        .map(|t| (t.src.start(), t.src.end()))
        .collect();
    assert_eq!(spans, [(0, 3), (3, 3), (5, 8)]);
}

#[test]
fn mdx_emits_the_tokens_the_position_vectors_assume() {
    let source = "x \\{y\\}";
    let block = &mdx(source).blocks[0];
    assert_eq!(
        kinds_and_texts(block),
        [
            (TokenKind::Identity, "x ".to_owned()),
            (TokenKind::Backslash, "{".to_owned()),
            (TokenKind::Identity, "y".to_owned()),
            (TokenKind::Backslash, "}".to_owned()),
        ]
    );

    let source = "a {'{'} b";
    let block = &mdx(source).blocks[0];
    let spans: Vec<_> = block
        .projection
        .as_ref()
        .unwrap()
        .tokens()
        .iter()
        .map(|t| (t.kind, t.text.as_str(), t.src.start(), t.src.end()))
        .collect();
    assert_eq!(
        spans,
        [
            (TokenKind::Identity, "a ", 0, 2),
            (TokenKind::Opaque, "{", 2, 7),
            (TokenKind::Identity, " b", 7, 9),
        ]
    );
}

#[test]
fn mdx_expressions_that_are_not_string_literals_render_nothing() {
    let source = "a {props.name} b {`x ${y}`} c\n";
    let block = &mdx(source).blocks[0];
    assert_eq!(block.projection.as_ref().unwrap().text(), "a  b  c");
}

#[test]
fn mdx_jsx_tags_are_gaps_and_esm_is_not_prose() {
    let source = "import A from 'a'\nexport const b = 1\n\nSee <b>bold</b> text.\n";
    let doc = mdx(source);
    assert_eq!(doc.blocks.len(), 1);
    assert_eq!(
        doc.blocks[0].projection.as_ref().unwrap().text(),
        "See bold text."
    );
}

#[test]
fn mdx_syntax_errors_are_reported() {
    let result = parse_mdx("Unclosed {expression\n", whole("Unclosed {expression\n"));
    assert!(matches!(result, Err(ParseError::Mdx(_))));
}

#[test]
fn hard_breaks_are_collapse_tokens_over_the_break() {
    let source = "end  \nnext";
    let tokens = md(source).blocks[0].projection.clone().unwrap();
    let kinds: Vec<_> = tokens
        .tokens()
        .iter()
        .map(|t| (t.kind, t.src.start(), t.src.end()))
        .collect();
    assert_eq!(
        kinds,
        [
            (TokenKind::Identity, 0, 3),
            (TokenKind::Collapse, 3, 6),
            (TokenKind::Identity, 6, 10),
        ]
    );
}

#[test]
fn html_comments_in_prose_are_masked() {
    let source = "foo<!--x-->";
    let tokens = md(source).blocks[0].projection.clone().unwrap();
    let got: Vec<_> = tokens
        .tokens()
        .iter()
        .map(|t| (t.kind, t.text.as_str(), t.src.start(), t.src.end()))
        .collect();
    assert_eq!(
        got,
        [
            (TokenKind::Identity, "foo", 0, 3),
            (TokenKind::Masked, "", 3, 11)
        ]
    );
}

#[test]
fn blocks_come_in_source_order_with_containers_first() {
    let source = "# H\n\n::: note\npara\n:::\n\n| a |\n|---|\n";
    let starts: Vec<_> = md(source).blocks.iter().map(|b| b.span.start()).collect();
    let mut sorted = starts.clone();
    sorted.sort_unstable();
    assert_eq!(starts, sorted);
}

fn html_segments<'a>(source: &'a str, doc: &Document) -> Vec<Vec<&'a str>> {
    doc.html_blocks
        .iter()
        .map(|block| block.segments.iter().map(|s| slice(source, *s)).collect())
        .collect()
}

#[test]
fn raw_html_blocks_are_listed_by_source_range() {
    let source = "Intro.\n\n<table>\n<tr><td>a</td></tr>\n</table>\n\nText.\n\n<div>b</div>\n";
    assert_eq!(
        html_segments(source, &md(source)),
        [
            vec!["<table>\n<tr><td>a</td></tr>\n</table>"],
            vec!["<div>b</div>"]
        ]
    );
}

#[test]
fn a_raw_html_block_in_a_block_quote_is_one_block_with_a_segment_per_line() {
    let source = "> <pre>\n> one\n> two</pre>\n";
    assert_eq!(
        html_segments(source, &md(source)),
        [vec!["<pre>\n", "one\n", "two</pre>"]]
    );
}

#[test]
fn two_raw_html_blocks_stay_two_blocks() {
    let source = "> <p>a</p>\n\n> <p>b</p>\n";
    assert_eq!(
        html_segments(source, &md(source)),
        [vec!["<p>a</p>"], vec!["<p>b</p>"]]
    );
}
#[test]
fn raw_html_blocks_carry_their_containers() {
    let quoted = "> <p>a</p>\n";
    assert_eq!(
        md(quoted).html_blocks[0].containers,
        [Container::BlockQuote]
    );
    let listed = "- item\n\n  <p>a</p>\n";
    assert_eq!(
        md(listed).html_blocks[0].containers,
        [Container::ListItem {
            ordered: false,
            depth: 1
        }]
    );
    let top = "<p>a</p>\n";
    assert_eq!(md(top).html_blocks[0].containers, []);
}
