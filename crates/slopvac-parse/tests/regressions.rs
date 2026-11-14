#![allow(missing_docs)]
//! Regressions found in review. Each test goes through the public parse functions.

use markdown::{ParseOptions, mdast::Node, to_mdast};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
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
fn jsx_elements_inside_expressions_do_not_start_regexes() {
    let cases = [
        "a {cond && <p>text</p>} b\n",
        "a {cond && <p>don't</p>} b\n",
        "a {cond && <br/>} b\n",
        "a {a && <b>x</b>} and {c && <i>it's</i>} z\n",
        "a {list.map((x) => <li key={x}>{x}</li>)} b\n",
        "a {x < y && /re/.test(z)} b\n",
    ];
    for source in cases {
        assert!(parse_mdx(source, whole(source)).is_ok(), "{source:?}");
    }
}

#[test]
fn what_naive_brace_counting_accepts_is_accepted() {
    // Not valid JavaScript, but markdown-rs without a parser accepts all of these.
    let cases = [
        "a {x ' y} b\n",
        "a {x /* y} b\n",
        "a {`} b\n",
        "a {x ` y} b\n\nmore\n",
        "a {(x) /y} b\n",
        "a {/re} b\n",
    ];
    for source in cases {
        assert!(parse_mdx(source, whole(source)).is_ok(), "{source:?}");
    }
}

#[test]
fn a_deeply_braced_mdx_expression_still_parses() {
    // More closing braces than the checker reads: it hands back to plain counting. The
    // work bounds are unit tests in `jsexpr.rs`.
    let n = 3_000;
    let source = format!("{{{}{}}}\n", "{x".repeat(n), "}".repeat(n));
    assert!(parse_mdx(&source, whole(&source)).is_ok());
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
fn long_runs_of_ampersands_stay_literal() {
    // The work bound is a unit test in `align.rs`.
    let source = format!("{}\n", "&".repeat(20_000));
    for doc in [mdx(&source), md(&source)] {
        assert_eq!(prose(&doc)[0].len(), 20_000);
    }
}

#[test]
fn a_code_body_in_a_block_quote_excludes_the_closing_fence() {
    let cases = [
        ("> ```js\n> x\n> ```\n", "x\n"),
        ("> > ```\n> > a\n> > b\n> > ```\n", "a\n> > b\n"),
        ("- item\n\n  > ```\n  > x\n  > ```\n", "x\n"),
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
    assert_eq!(slice(source, bodies[0]), "x = 1\n");
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
fn deeply_nested_bang_blocks_all_become_admonitions() {
    // The work bound is a unit test in `mask.rs`.
    let depth = 300;
    let mut source = String::new();
    for level in 0..depth {
        source.push_str(&" ".repeat(4 * level));
        source.push_str("!!! note\n");
    }
    for _ in 0..depth {
        source.push_str(&" ".repeat(4 * (depth - 1)));
        source.push_str("text\n\n");
    }
    assert_eq!(admonitions(&md(&source)), depth);
}

// ---- review round 2: JavaScript scanning ----

#[test]
fn regex_literals_in_mdx_expressions_may_hold_quotes_and_braces() {
    let cases = [
        "a {/'/.test(x)} b\n",
        "a {x.replace(/}/g, 'k')} b\n",
        "a {/[/']/.test(s)} b\n",
        "a {/\\/'/.test(s)} b\n",
        "a {cond && /'/.test(s)} b\n",
        "a {(x) ? /'/ : 'y'} b\n",
    ];
    for source in cases {
        assert!(parse_mdx(source, whole(source)).is_ok(), "{source:?}");
    }
}

#[test]
fn division_in_mdx_expressions_is_not_a_regex() {
    // Read as a regex, the first slash would swallow the closing brace.
    let cases = [
        "a {n / 2} b\n",
        "a {(n) / 2} b\n",
        "a {list[0] / 2} b\n",
        "a {n / 2 + m / 3} b\n",
        "a {x++ / 2} b\n",
    ];
    for source in cases {
        assert!(parse_mdx(source, whole(source)).is_ok(), "{source:?}");
    }
}

#[test]
fn every_js_line_terminator_ends_a_line_comment() {
    for terminator in ["\n", "\r", "\r\n", "\u{2028}", "\u{2029}"] {
        let source = format!("a {{// it's{terminator}}} b\n");
        assert!(parse_mdx(&source, whole(&source)).is_ok(), "{source:?}");
    }
}

// ---- review round 2: code regions come from the parser ----

fn pulldown_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES
}

/// Code blocks of the unmasked source as pulldown-cmark reads them: the block range
/// without its final terminator and the hull of its text events. A blank first line
/// comes with its container prefix, so the hull starts at that line's terminator.
type Open = (std::ops::Range<usize>, Option<(usize, usize)>);

fn pulldown_code(source: &str) -> Vec<(Span, Option<Span>)> {
    let mut out = Vec::new();
    let mut open: Option<Open> = None;
    for (event, range) in Parser::new_ext(source, pulldown_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => open = Some((range, None)),
            Event::Text(_) => {
                if let Some((_, hull)) = open.as_mut() {
                    *hull = Some(match *hull {
                        None => {
                            let bytes = source.as_bytes();
                            let blank = matches!(&event, Event::Text(t) if matches!(&**t, "\n" | "\r\n" | "\r"));
                            let width = if bytes[..range.end].ends_with(b"\r\n") {
                                2
                            } else {
                                1
                            };
                            let mut start = if blank && range.len() > width {
                                range.end - width
                            } else {
                                range.start
                            };
                            if start > 0 && bytes[start - 1] == b'\r' && bytes[start] == b'\n' {
                                start -= 1;
                            }
                            (start, range.end)
                        }
                        Some((s, e)) => (s.min(range.start), e.max(range.end)),
                    });
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((range, hull)) = open.take() {
                    let kept = source[range.clone()].trim_end_matches(['\n', '\r']).len();
                    out.push((
                        Span::new(range.start, range.start + kept).unwrap(),
                        hull.map(|(s, e)| Span::new(s, e).unwrap()),
                    ));
                }
            }
            _ => {}
        }
    }
    out
}

fn mdast_code(source: &str) -> Vec<Span> {
    fn walk(node: &Node, source: &str, out: &mut Vec<Span>) {
        if let Node::Code(_) = node {
            let p = node.position().unwrap();
            let kept = source[p.start.offset..p.end.offset]
                .trim_end_matches(['\n', '\r'])
                .len();
            out.push(Span::new(p.start.offset, p.start.offset + kept).unwrap());
        }
        for child in node.children().into_iter().flatten() {
            walk(child, source, out);
        }
    }
    let options = ParseOptions {
        constructs: markdown::Constructs {
            gfm_table: true,
            gfm_strikethrough: true,
            gfm_task_list_item: true,
            ..markdown::Constructs::mdx()
        },
        ..ParseOptions::mdx()
    };
    let mut out = Vec::new();
    walk(&to_mdast(source, &options).unwrap(), source, &mut out);
    out
}

fn own_code(doc: &Document) -> Vec<(Span, Span)> {
    doc.blocks
        .iter()
        .filter_map(|b| match &b.kind {
            BlockKind::CodeBlock { body, .. } => Some((b.span, *body)),
            _ => None,
        })
        .collect()
}

/// Inputs where fences, containers and admonition markers meet.
const TRICKY: &[&str] = &[
    "```\ncode\n```\n",
    "   ```\ncode\n    ```\n::: note\ntext\n:::\n   ```\n",
    "- a\n\n  ```\n  ::: note\n  x\n  :::\n  ```\n\n::: tip\nreal\n:::\n",
    "> ```\n> ::: note\n> ```\n\n::: tip\nreal\n:::\n",
    "> ```\n> a\n> ```\n> ```\n",
    "```\na\n> ```\n",
    "    ```\n    code\n\nPara.\n\n::: note\nx\n:::\n",
    "~~~\n```\n::: note\n~~~\n::: tip\nz\n:::\n",
    "1. a\n   - b\n\n     ```\n     ::: note\n     :::\n     ```\n",
    "```\n:::\n```\n:::\n",
    "para\n```\ncode\n```\npara\n",
    "- ```\n  code\n  ```\n",
    "````\n```\n````\n",
    "```\n",
    "```py\n\n```\n",
    "> [!NOTE]\n> ```\n> x\n> ```\n",
    "```md\n> [!NOTE]\n> x\n```\n",
    "```\r\n::: note\r\n```\r\n::: tip\r\nx\r\n:::\r\n",
];

#[test]
fn markdown_code_regions_equal_the_parsers_own() {
    for source in TRICKY {
        let doc = md(source);
        let want = pulldown_code(source);
        let got = own_code(&doc);
        assert_eq!(
            got.iter().map(|(b, _)| *b).collect::<Vec<_>>(),
            want.iter().map(|(b, _)| *b).collect::<Vec<_>>(),
            "{source:?}"
        );
        for ((_, body), (block, hull)) in got.iter().zip(&want) {
            // The body is the parser's text, or an empty range inside the block.
            match hull {
                Some(hull) => assert_eq!(body, hull, "{source:?}"),
                None => assert!(
                    body.is_empty() && body.start() <= block.end() + 1,
                    "{source:?}"
                ),
            }
        }
    }
}

#[test]
fn mdx_code_regions_equal_the_parsers_own() {
    for source in TRICKY {
        let want = mdast_code(source);
        let got: Vec<_> = own_code(&mdx(source)).iter().map(|(b, _)| *b).collect();
        assert_eq!(got, want, "{source:?}");
    }
}

#[test]
fn markers_in_parser_confirmed_code_are_never_admonitions() {
    // (source, admonitions that are really there)
    let cases = [
        ("   ```\ncode\n    ```\n::: note\ntext\n:::\n   ```\n", 0),
        (
            "- a\n\n  ```\n  ::: note\n  x\n  :::\n  ```\n\n::: tip\nreal\n:::\n",
            1,
        ),
        ("> ```\n> ::: note\n> ```\n\n::: tip\nreal\n:::\n", 1),
        ("    ```\n    code\n\nPara.\n\n::: note\nx\n:::\n", 1),
        ("~~~\n```\n::: note\n~~~\n::: tip\nz\n:::\n", 1),
        (
            "1. a\n   - b\n\n     ```\n     ::: note\n     :::\n     ```\n",
            0,
        ),
        ("```\n:::\n```\n:::\n", 0),
    ];
    for (source, want) in cases {
        assert_eq!(admonitions(&md(source)), want, "{source:?}");
    }
}

#[test]
fn an_unclosed_fence_keeps_a_last_line_that_looks_like_a_quoted_fence() {
    let source = "```\na\n> ```\n";
    let doc = md(source);
    let bodies = code_bodies(&doc);
    assert_eq!(slice(source, bodies[0]), "a\n> ```\n");
    let doc = mdx(source);
    assert_eq!(slice(source, code_bodies(&doc)[0]), "a\n> ```\n");
}

#[test]
fn indented_code_with_fence_text_does_not_hide_a_later_admonition() {
    let source = "    ```\n    code\n\nPara.\n\n::: note\nx\n:::\n";
    let doc = md(source);
    assert_eq!(admonitions(&doc), 1);
    assert_eq!(code_bodies(&doc).len(), 1);
}

#[test]
fn both_parsers_expose_the_same_code_body_where_they_agree_on_the_block() {
    let agreeing = [0, 2, 3, 4, 5, 11, 12, 13, 14, 15, 16];
    for index in agreeing {
        let source = TRICKY[index];
        let in_md: Vec<_> = own_code(&md(source)).iter().map(|(_, b)| *b).collect();
        let in_mdx: Vec<_> = own_code(&mdx(source)).iter().map(|(_, b)| *b).collect();
        assert_eq!(in_md, in_mdx, "{source:?}");
    }
}

// ---- review round 3 ----

#[test]
fn quoted_fences_expose_the_same_body_with_every_line_ending() {
    for eol in ["\n", "\r\n", "\r"] {
        let cases = [
            ("> ```js\n> a\n> b\n> ```\n", "a\n> b\n"),
            ("- item\n\n  ```\n  a\n  b\n  ```\n", "a\n  b\n"),
        ];
        for (template, want) in cases {
            let source = template.replace('\n', eol);
            let want = want.replace('\n', eol);
            // pulldown-cmark does not read a lone CR as a line ending inside a quote.
            let docs = if eol == "\r" {
                vec![mdx(&source)]
            } else {
                vec![md(&source), mdx(&source)]
            };
            for doc in docs {
                let bodies = code_bodies(&doc);
                assert_eq!(bodies.len(), 1, "{source:?}");
                assert_eq!(slice(&source, bodies[0]), want, "{source:?}");
            }
        }
    }
}

#[test]
fn code_regions_equal_the_parsers_own_with_every_line_ending() {
    for eol in ["\r\n", "\r"] {
        for template in TRICKY {
            let source = template.replace("\r\n", "\n").replace('\n', eol);
            let got: Vec<_> = own_code(&md(&source)).iter().map(|(b, _)| *b).collect();
            let want: Vec<_> = pulldown_code(&source).iter().map(|(b, _)| *b).collect();
            assert_eq!(got, want, "{source:?}");
            let got: Vec<_> = own_code(&mdx(&source)).iter().map(|(b, _)| *b).collect();
            assert_eq!(got, mdast_code(&source), "{source:?}");
        }
    }
}

#[test]
fn an_mdx_directive_with_an_indented_closer_does_not_swallow_prose() {
    let source = "```{note} Title\nbody\n    ```\n\nFollowing prose.\n";
    let doc = mdx(source);
    assert_eq!(admonitions(&doc), 1);
    assert_eq!(prose(&doc), ["body", "Following prose."]);
    assert_eq!(code_bodies(&doc), []);
}

// ---- review round 4 ----

#[test]
fn mdx_soft_wraps_are_wrap_tokens_with_every_line_ending() {
    for eol in ["\n", "\r\n", "\r"] {
        let source = format!("foo{eol}bar baz{eol}*qux*");
        let doc = mdx(&source);
        let proj = doc.blocks[0].projection.as_ref().unwrap();
        assert_eq!(proj.text(), "foo bar baz qux", "{eol:?}");
        let tokens: Vec<_> = proj
            .tokens()
            .iter()
            .map(|t| (t.kind, t.src.start(), t.src.end()))
            .collect();
        let n = eol.len();
        assert_eq!(
            tokens,
            [
                (TokenKind::Identity, 0, 3),
                (TokenKind::Wrap, 3, 3),
                (TokenKind::Identity, 3 + n, 10 + n),
                (TokenKind::Wrap, 10 + n, 10 + n),
                (TokenKind::Identity, 11 + 2 * n, 14 + 2 * n),
            ],
            "{eol:?}"
        );
    }
}

#[test]
fn a_blank_first_body_line_loses_its_container_prefix() {
    let cases = [
        ("> ```\n>\n> a\n> ```\n", "\n> a\n"),
        ("> ```\n> \n> a\n> ```\n", "\n> a\n"),
        ("- item\n\n  ```\n\n  a\n  ```\n", "\n  a\n"),
    ];
    for eol in ["\n", "\r\n"] {
        for (template, want) in cases {
            let source = template.replace('\n', eol);
            let want = want.replace('\n', eol);
            for doc in [md(&source), mdx(&source)] {
                let bodies = code_bodies(&doc);
                assert_eq!(bodies.len(), 1, "{source:?}");
                assert_eq!(slice(&source, bodies[0]), want, "{source:?}");
            }
        }
    }
    // A lone CR: markdown-rs only, because pulldown-cmark does not read it inside quotes.
    let source = "> ```\r>\r> a\r> ```\r";
    assert_eq!(slice(source, code_bodies(&mdx(source))[0]), "\r> a\r");
}

#[test]
fn a_large_data_expression_does_not_disturb_earlier_expressions() {
    // The string holds a brace. Plain counting would end its expression inside it.
    let data = "{a:1},".repeat(1500);
    let source = format!("{{\"a }} b\"}} and then {{[{data}]}}\n");
    let doc = mdx(&source);
    let text = prose(&doc);
    assert_eq!(text.len(), 1);
    assert!(
        text[0].starts_with("a } b and then"),
        "{:?}",
        &text[0][..20.min(text[0].len())]
    );
}
