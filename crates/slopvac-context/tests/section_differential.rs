//! The section and paragraph tags of a structural tagger match the algorithm it replaced.
//!
//! The oracle below is the tagging code from before the single-pass section bounds and the
//! cached heading verdicts, copied unchanged where it is not about the view. It scans
//! backwards and forwards for every matching block, so it is quadratic, but easy to trust.
//! Documents are random trees of headings and paragraphs, some with blocks whose spans read
//! out of source order, the way the cells of a wrapped table do.

use std::ops::Range;

use proptest::prelude::*;

use slopvac_context::{
    BlockKind, BlockView, DocumentView, OwnedBlock, PlainDocument, Tag, Tagger, TaggerSpec,
};

fn level(b: &BlockView<'_>) -> u8 {
    b.heading_level
        .unwrap_or_else(|| u8::try_from(b.heading_path.len()).unwrap_or(u8::MAX))
}

/// Copied from the code before the rewrite.
fn section_span(blocks: &[BlockView<'_>], i: usize) -> Range<usize> {
    let owner = (0..=i)
        .rev()
        .find(|&j| blocks[j].kind == BlockKind::Heading);
    let (first, own_level) = match owner {
        Some(j) => (j, level(&blocks[j])),
        None => (0, 0),
    };
    let last = (i + 1..blocks.len())
        .find(|&k| blocks[k].kind == BlockKind::Heading && level(&blocks[k]) <= own_level.max(1))
        .map_or(blocks.len() - 1, |k| k - 1);
    let members = &blocks[first..=last];
    let start = members.iter().map(|b| b.span.start).min().unwrap_or(0);
    let end = members.iter().map(|b| b.span.end).max().unwrap_or(0);
    start..end
}

/// The tags the old `tag` produced for a tagger with id `t` and `heading_path: 'setup'`:
/// one per matching block, with duplicates removed in first-seen order.
fn old_tags(doc: &PlainDocument, scope: &str) -> Vec<(String, Range<usize>, String)> {
    let blocks = doc.blocks();
    let mut out: Vec<(String, Range<usize>, String)> = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        let heading_ok = b
            .heading_path
            .iter()
            .any(|h| h.to_lowercase().contains("setup"));
        if b.kind == BlockKind::CodeBlock || !heading_ok {
            continue;
        }
        let tag = if scope == "section" {
            (
                "k".to_owned(),
                section_span(&blocks, i),
                "t: section".to_owned(),
            )
        } else {
            ("k".to_owned(), b.span.clone(), "t: block".to_owned())
        };
        if !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}

fn new_tags(doc: &PlainDocument, scope: &str) -> Vec<(String, Range<usize>, String)> {
    let yaml = format!(
        "schema_version: 3\nid: t\ntag: k\nscope: {scope}\nwhen:\n  heading_path: 'setup'\n"
    );
    let tagger = TaggerSpec::from_yaml(&yaml).unwrap().compile().unwrap();
    let tags: Vec<Tag> = tagger.tag(doc).unwrap();
    let mut out: Vec<(String, Range<usize>, String)> = Vec::new();
    for t in tags {
        let tag = (t.kind, t.span, t.evidence);
        if !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}

#[derive(Debug, Clone)]
enum Item {
    Heading(usize, usize),
    Paragraph,
}

fn item() -> impl Strategy<Value = Item> {
    prop_oneof![
        (1usize..=4, 0usize..4).prop_map(|(l, t)| Item::Heading(l, t)),
        Just(Item::Paragraph),
    ]
}

fn render(items: &[Item]) -> String {
    const TEXTS: [&str; 4] = ["Setup", "Other", "Install Setup", "Next"];
    let mut src = String::new();
    for it in items {
        match it {
            Item::Heading(l, t) => {
                src.push_str(&"#".repeat(*l));
                src.push(' ');
                src.push_str(TEXTS[*t]);
                src.push_str("\n\n");
            }
            Item::Paragraph => src.push_str("Text here.\n\n"),
        }
    }
    src
}

/// Rotate the spans of the non-heading blocks by `by`, so some end after later blocks.
fn scramble(doc: &mut PlainDocument, by: usize) {
    let idx: Vec<usize> = doc
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.kind != BlockKind::Heading)
        .map(|(i, _)| i)
        .collect();
    if idx.is_empty() {
        return;
    }
    let spans: Vec<Range<usize>> = idx.iter().map(|&i| doc.blocks[i].span.clone()).collect();
    for (k, &i) in idx.iter().enumerate() {
        let block: &mut OwnedBlock = &mut doc.blocks[i];
        block.span = spans[(k + by) % spans.len()].clone();
    }
}

proptest! {
    #[test]
    fn section_tags_match_the_old_algorithm(
        items in prop::collection::vec(item(), 0..40),
        by in 0usize..8,
    ) {
        let mut doc = PlainDocument::from_markdown("a.md", &render(&items));
        scramble(&mut doc, by);
        prop_assert_eq!(new_tags(&doc, "section"), old_tags(&doc, "section"));
    }

    #[test]
    fn paragraph_tags_match_the_old_algorithm(items in prop::collection::vec(item(), 0..40)) {
        let doc = PlainDocument::from_markdown("a.md", &render(&items));
        prop_assert_eq!(new_tags(&doc, "paragraph"), old_tags(&doc, "paragraph"));
    }
}
