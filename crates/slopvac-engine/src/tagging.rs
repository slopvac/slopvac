//! Context tags for a parsed document.
//!
//! `slopvac-context` reads a document through its `DocumentView` trait. This module adapts
//! a `slopvac-parse` document to it. Block text is the projected prose and block spans are
//! source bytes, so a tag span is a source range that a finding span can sit inside.

use std::collections::BTreeMap;

use frontmatter_spans::FrontMatter;
use slopvac_context::{
    BlockKind as ViewKind, BlockView, DocumentView, Error as ContextError, SentenceView, TagIndex,
    TagRegistry,
};
use slopvac_parse::{BlockKind, Document, Projection};

struct Entry<'a> {
    kind: ViewKind,
    span: std::ops::Range<usize>,
    text: &'a str,
    heading_path: Vec<&'a str>,
    heading_level: Option<u8>,
    admonition_title: Option<&'a str>,
    projection: Option<&'a Projection>,
}

/// A parsed document as the taggers read it.
struct View<'a> {
    path: &'a str,
    source_len: usize,
    title: Option<&'a str>,
    front_matter: Option<&'a FrontMatter<'a>>,
    entries: Vec<Entry<'a>>,
    by_span: BTreeMap<(usize, usize), usize>,
}

impl<'a> View<'a> {
    fn new(
        path: &'a str,
        source: &'a str,
        doc: &'a Document,
        front_matter: Option<&'a FrontMatter<'a>>,
    ) -> Self {
        let mut entries: Vec<Entry<'a>> = Vec::new();
        let mut headings: Vec<(u8, &'a str)> = Vec::new();
        let mut title = None;
        // Open admonitions as (end of span, title).
        let mut admonitions: Vec<(usize, Option<&'a str>)> = Vec::new();
        for block in &doc.blocks {
            while admonitions
                .last()
                .is_some_and(|(end, _)| block.span.start() >= *end)
            {
                admonitions.pop();
            }
            if let BlockKind::Admonition(a) = &block.kind {
                admonitions.push((block.span.end(), a.title.as_deref()));
                continue;
            }
            let (kind, text, projection) = match (&block.kind, &block.projection) {
                (BlockKind::CodeBlock { body, .. }, _) => {
                    (ViewKind::CodeBlock, &source[body.start()..body.end()], None)
                }
                (BlockKind::Heading { .. }, Some(p)) => (ViewKind::Heading, p.text(), Some(p)),
                (BlockKind::Paragraph, Some(p)) => (ViewKind::Paragraph, p.text(), Some(p)),
                (BlockKind::TableCell { .. }, Some(p)) => (ViewKind::TableRow, p.text(), Some(p)),
                _ => continue,
            };
            let mut level = None;
            if let BlockKind::Heading { level: l } = &block.kind {
                while headings.last().is_some_and(|(top, _)| top >= l) {
                    headings.pop();
                }
                headings.push((*l, text));
                level = Some(*l);
                title.get_or_insert(text);
            }
            let span = match &block.kind {
                BlockKind::CodeBlock { body, .. } => body.start()..body.end(),
                _ => block.span.start()..block.span.end(),
            };
            entries.push(Entry {
                kind,
                span,
                text,
                heading_path: headings.iter().map(|(_, t)| *t).collect(),
                heading_level: level,
                admonition_title: admonitions.iter().rev().find_map(|(_, t)| *t),
                projection,
            });
        }
        let by_span = entries
            .iter()
            .enumerate()
            .map(|(i, e)| ((e.span.start, e.span.end), i))
            .collect();
        Self {
            path,
            source_len: source.len(),
            title,
            front_matter,
            entries,
            by_span,
        }
    }
}

impl DocumentView for View<'_> {
    fn path(&self) -> &str {
        self.path
    }

    fn title(&self) -> Option<&str> {
        self.front_matter
            .and_then(|f| f.get("title"))
            .and_then(|n| n.as_str())
            .or(self.title)
    }

    fn front_matter(&self, key: &str) -> Option<&str> {
        self.front_matter?.get(key)?.as_str()
    }

    fn source_len(&self) -> usize {
        self.source_len
    }

    fn blocks(&self) -> Vec<BlockView<'_>> {
        self.entries
            .iter()
            .map(|e| BlockView {
                kind: e.kind,
                span: e.span.clone(),
                text: e.text,
                heading_path: e.heading_path.clone(),
                heading_level: e.heading_level,
                admonition_title: e.admonition_title,
            })
            .collect()
    }

    fn sentences<'a>(&self, block: &BlockView<'a>) -> Vec<SentenceView<'a>> {
        let split = slopvac_context::split_sentences(block);
        let Some(projection) = self
            .by_span
            .get(&(block.span.start, block.span.end))
            .and_then(|&i| self.entries[i].projection)
        else {
            return split;
        };
        // The splitter assumes the text sits at the block span. Map each sentence through
        // the projection so its span is a source range.
        split
            .into_iter()
            .map(|s| {
                let from = s.span.start - block.span.start;
                let span = projection.span_for_bytes(from, from + s.text.len());
                SentenceView {
                    span: span.start()..span.end(),
                    text: s.text,
                }
            })
            .collect()
    }
}

/// Run every tagger over `doc`.
pub(crate) fn tag(
    registry: &TagRegistry,
    path: &str,
    source: &str,
    doc: &Document,
    front_matter: Option<&FrontMatter<'_>>,
) -> Result<TagIndex, ContextError> {
    registry.tag_document(&View::new(path, source, doc, front_matter))
}
