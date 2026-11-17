//! Context tags for a parsed document.
//!
//! `slopvac-context` reads a document through its `DocumentView` trait. This module adapts
//! a `slopvac-parse` document to it. Block text is the projected prose and block spans are
//! source bytes, so a tag span is a source range that a finding span can sit inside.

use crate::sub;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;

use frontmatter_spans::FrontMatter;
use slopvac_context::{
    BlockKind as ViewKind, BlockView, DocumentView, Error as ContextError, SentenceView, Tag,
    TagIndex, TagRegistry,
};
use slopvac_parse::{BlockKind, Container, Document, Projection};

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
    /// Steps taken by `split_interleaved`, for the tests that bound its work.
    visits: Cell<usize>,
    /// The entry that produced each span handed to a tagger as a block or a sentence.
    owners: RefCell<HashMap<(usize, usize), usize>>,
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
            let (mut kind, text, projection) = match (&block.kind, &block.projection) {
                (BlockKind::CodeBlock { body, .. }, _) => (
                    ViewKind::CodeBlock,
                    sub(source, body.start()..body.end()),
                    None,
                ),
                (BlockKind::Heading { .. }, Some(p)) => (ViewKind::Heading, p.text(), Some(p)),
                (BlockKind::Paragraph, Some(p)) => (ViewKind::Paragraph, p.text(), Some(p)),
                (BlockKind::TableCell { .. }, Some(p)) => (ViewKind::TableRow, p.text(), Some(p)),
                _ => continue,
            };
            // The innermost list or quote decides what a paragraph is to a tagger.
            if kind == ViewKind::Paragraph {
                match block.containers.last() {
                    Some(Container::ListItem { .. }) => kind = ViewKind::ListItem,
                    Some(Container::BlockQuote) => kind = ViewKind::Quote,
                    _ => {}
                }
            }
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
                BlockKind::Paragraph
                | BlockKind::Heading { .. }
                | BlockKind::Table
                | BlockKind::TableCell { .. }
                | BlockKind::Admonition(_)
                | _ => block.span.start()..block.span.end(),
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
            visits: Cell::new(0),
            owners: RefCell::new(HashMap::new()),
            source_len: source.len(),
            title,
            front_matter,
            entries,
            by_span,
        }
    }
}

impl View<'_> {
    /// Replace the tags that sit on interleaved text by tags on the text that contributed.
    ///
    /// The cells of a table whose left cell wraps over several lines read cell by cell, so
    /// the extent of the left cell contains the right cell. A tag on a sentence or a block
    /// of the left cell has that extent, and a match in the right cell would inherit the tag.
    /// Such a tag becomes one tag for each line of the cell's own text. A tag that covers
    /// several blocks, or that no sentence or block of an entry produced, is left alone.
    ///
    /// The work is `O(n log n)` in the number of entries and `O(1)` for each tag: a sweep
    /// over the entries sorted by start finds those that overlap another, and the owner of a
    /// tag is the entry whose sentence or block produced its span.
    fn split_interleaved(&self, index: TagIndex) -> TagIndex {
        let mut order: Vec<(usize, &Range<usize>)> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.projection.is_some())
            .map(|(i, e)| (i, &e.span))
            .collect();
        order.sort_by_key(|(_, span)| span.start);
        // An entry overlaps another when an earlier one ends after it starts, or the next
        // one starts before it ends. Sorted by start, those two checks see every overlap.
        let mut marked = vec![false; self.entries.len()];
        let mut latest_end = 0;
        for (k, &(i, span)) in order.iter().enumerate() {
            self.visits.set(self.visits.get() + 1);
            if span.start < span.end {
                let earlier = latest_end > span.start;
                let later = order
                    .get(k + 1)
                    .is_some_and(|(_, next)| next.start < span.end);
                if let Some(slot) = marked.get_mut(i) {
                    *slot = earlier || later;
                }
            }
            latest_end = latest_end.max(span.end);
        }
        if !marked.contains(&true) {
            return index;
        }
        let owners = self.owners.borrow();
        let mut extents: BTreeMap<usize, Vec<Range<usize>>> = BTreeMap::new();
        let mut tags = Vec::with_capacity(index.tags().len());
        for (group, tag) in index.tags().iter().enumerate() {
            self.visits.set(self.visits.get() + 1);
            let owner = owners
                .get(&(tag.span.start, tag.span.end))
                .copied()
                .filter(|&i| marked.get(i).copied().unwrap_or(false));
            let runs = owner.map(|i| {
                extents.entry(i).or_insert_with(|| {
                    self.entries
                        .get(i)
                        .and_then(|e| e.projection)
                        .map(|p| p.extents().iter().map(|s| s.start()..s.end()).collect())
                        .unwrap_or_default()
                })
            });
            let mut pieces = Vec::new();
            for run in runs.into_iter().flatten() {
                self.visits.set(self.visits.get() + 1);
                let from = run.start.max(tag.span.start);
                let to = run.end.min(tag.span.end);
                if from < to {
                    pieces.push((
                        Tag {
                            span: from..to,
                            ..tag.clone()
                        },
                        group,
                    ));
                }
            }
            if pieces.is_empty() {
                tags.push((tag.clone(), group));
            } else {
                tags.extend(pieces);
            }
        }
        TagIndex::from_pieces(tags)
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
        let mut owners = self.owners.borrow_mut();
        self.entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                owners.insert((e.span.start, e.span.end), i);
                e
            })
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
        let Some((owner, projection)) = self
            .by_span
            .get(&(block.span.start, block.span.end))
            .and_then(|&i| {
                self.entries
                    .get(i)
                    .and_then(|e| e.projection)
                    .map(|p| (i, p))
            })
        else {
            return split;
        };
        let mut owners = self.owners.borrow_mut();
        // The splitter assumes the text sits at the block span. Map each sentence through
        // the projection so its span is a source range.
        split
            .into_iter()
            .map(|s| {
                // A sentence that starts before its block cannot be mapped: keep it as the
                // splitter gave it.
                let Some(from) = s.span.start.checked_sub(block.span.start) else {
                    return s;
                };
                let span = projection.span_for_bytes(from, from + s.text.len());
                owners.insert((span.start(), span.end()), owner);
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
    let view = View::new(path, source, doc, front_matter);
    let index = registry.tag_document(&view)?;
    Ok(view.split_interleaved(index))
}

#[cfg(test)]
mod tests {
    use slopvac_parse::{Block, TokenKind};

    use super::*;

    /// `count` cells that fell back to line precision, so each has the span of the whole row.
    fn row_of_fallback_cells(count: usize) -> Document {
        let row = slopvac_model::Span::new(0, 1000).unwrap();
        let blocks = (0..count)
            .map(|_| {
                let mut projection = Projection::new();
                projection.push(TokenKind::Opaque, "cell", row);
                Block {
                    kind: BlockKind::TableCell { row: 0, column: 0 },
                    span: row,
                    projection: Some(projection),
                    containers: Vec::new(),
                }
            })
            .collect();
        Document {
            blocks,
            code_spans: Vec::new(),
            html_blocks: Vec::new(),
        }
    }

    #[test]
    fn splitting_interleaved_tags_takes_a_fixed_number_of_steps_for_each_entry() {
        let count = 20_000;
        let doc = row_of_fallback_cells(count);
        let view = View::new("t.rst", "", &doc, None);
        // One tag on every cell, as a block tagger would produce.
        let tags = (0..count)
            .map(|_| Tag {
                kind: "requirement".to_owned(),
                span: 0..1000,
                confidence: 1.0,
                evidence: String::new(),
            })
            .collect();
        view.blocks();
        let _ = view.split_interleaved(TagIndex::from_tags(tags));
        // The sweep visits each entry once, and each tag its owner and its extents.
        assert!(
            view.visits.get() <= 8 * count,
            "{} steps for {count} entries",
            view.visits.get()
        );
    }
}
