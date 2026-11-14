#![allow(missing_docs)]
//! Container ancestry of blocks: the lists and block quotes that hold each block.

use slopvac_model::Span;
use slopvac_parse::{Container, Document, parse_markdown, parse_mdx};

fn whole(source: &str) -> Span {
    Span::new(0, source.len()).unwrap()
}

fn md(source: &str) -> Document {
    parse_markdown(source, whole(source)).unwrap()
}

fn mdx(source: &str) -> Document {
    parse_mdx(source, whole(source)).unwrap()
}

/// `(text of the block, its containers)` for every block that has a projection or is code.
fn ancestry(source: &str, doc: &Document) -> Vec<(String, Vec<Container>)> {
    doc.blocks
        .iter()
        .filter(|b| {
            b.projection.is_some() || matches!(b.kind, slopvac_parse::BlockKind::CodeBlock { .. })
        })
        .map(|b| {
            (
                source[b.span.start()..b.span.end()].to_owned(),
                b.containers.clone(),
            )
        })
        .collect()
}

const ITEM: Container = Container::ListItem {
    ordered: false,
    depth: 1,
};

type Parser = fn(&str) -> Document;

#[test]
fn a_top_level_paragraph_has_no_container() {
    for parse in [md as Parser, mdx as Parser] {
        let source = "Plain text.\n";
        assert_eq!(
            ancestry(source, &parse(source)),
            [("Plain text.".to_owned(), vec![])]
        );
    }
}

#[test]
fn list_items_record_order_and_depth_for_markdown_and_mdx() {
    let source = "- one\n  1. two\n- three\n";
    let want = vec![
        ("one".to_owned(), vec![ITEM]),
        (
            "two".to_owned(),
            vec![
                ITEM,
                Container::ListItem {
                    ordered: true,
                    depth: 2,
                },
            ],
        ),
        ("three".to_owned(), vec![ITEM]),
    ];
    for parse in [md as Parser, mdx as Parser] {
        assert_eq!(ancestry(source, &parse(source)), want);
    }
}

#[test]
fn a_loose_list_item_paragraph_is_inside_its_item() {
    let source = "1. first\n\n   second\n\nafter\n";
    let ordered = Container::ListItem {
        ordered: true,
        depth: 1,
    };
    for parse in [md as Parser, mdx as Parser] {
        let got = ancestry(source, &parse(source));
        assert_eq!(got[0], ("first".to_owned(), vec![ordered]));
        assert_eq!(got[1], ("second".to_owned(), vec![ordered]));
        assert_eq!(got[2], ("after".to_owned(), vec![]));
    }
}

#[test]
fn block_quotes_and_nesting_in_lists_are_recorded_outer_to_inner() {
    let source = "> quoted\n\n- item\n\n  > inside\n";
    let want = vec![
        ("quoted".to_owned(), vec![Container::BlockQuote]),
        ("item".to_owned(), vec![ITEM]),
        ("inside".to_owned(), vec![ITEM, Container::BlockQuote]),
    ];
    for parse in [md as Parser, mdx as Parser] {
        let got = ancestry(source, &parse(source));
        // The quote prefix belongs to the block span for the nested paragraph.
        assert_eq!(
            got.iter().map(|g| g.1.clone()).collect::<Vec<_>>(),
            want.iter().map(|w| w.1.clone()).collect::<Vec<_>>()
        );
    }
}

#[test]
fn code_blocks_carry_their_containers() {
    let source = "- item\n\n  ```\n  code\n  ```\n";
    for parse in [md as Parser, mdx as Parser] {
        let doc = parse(source);
        let code = doc
            .blocks
            .iter()
            .find(|b| matches!(b.kind, slopvac_parse::BlockKind::CodeBlock { .. }))
            .unwrap();
        assert_eq!(code.containers, [ITEM]);
    }
}
