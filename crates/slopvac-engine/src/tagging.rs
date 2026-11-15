//! Context tags for a parsed document.
//!
//! `slopvac-context` reads a document through its `DocumentView` trait. This module adapts
//! a `slopvac-parse` document to it. Block text is the projected prose and block spans are
//! source bytes, so a tag span is a source range that a finding span can sit inside.

use std::collections::BTreeMap;

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
    /// Offsets of the line terminator bytes, to tell whether a source range crosses a line.
    breaks: Vec<usize>,
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
                (BlockKind::CodeBlock { body, .. }, _) => {
                    (ViewKind::CodeBlock, &source[body.start()..body.end()], None)
                }
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
            breaks: source
                .bytes()
                .enumerate()
                .filter(|&(_, b)| b == b'\n' || b == b'\r')
                .map(|(i, _)| i)
                .collect(),
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
    /// several blocks is left alone.
    fn split_interleaved(&self, index: TagIndex) -> TagIndex {
        // Entries with prose whose extent overlaps another's.
        let mut order: Vec<usize> = (0..self.entries.len())
            .filter(|&i| self.entries[i].projection.is_some())
            .collect();
        order.sort_by_key(|&i| self.entries[i].span.start);
        let mut marked = vec![false; self.entries.len()];
        let mut active: Vec<usize> = Vec::new();
        for &i in &order {
            let start = self.entries[i].span.start;
            active.retain(|&j| self.entries[j].span.end > start);
            if !active.is_empty() {
                marked[i] = true;
                for &j in &active {
                    marked[j] = true;
                }
            }
            active.push(i);
        }
        let interleaved: Vec<usize> = order.into_iter().filter(|&i| marked[i]).collect();
        let Some(longest) = interleaved
            .iter()
            .map(|&i| self.entries[i].span.end - self.entries[i].span.start)
            .max()
        else {
            return index;
        };
        let mut tags = Vec::with_capacity(index.tags().len());
        for tag in index.tags() {
            // The smallest interleaved extent that holds the tag.
            let after =
                interleaved.partition_point(|&i| self.entries[i].span.start <= tag.span.start);
            let owner = interleaved[..after]
                .iter()
                .rev()
                .take_while(|&&i| {
                    tag.span.end - self.entries[i].span.start.min(tag.span.end) <= longest
                })
                .filter(|&&i| tag.span.end <= self.entries[i].span.end)
                .min_by_key(|&&i| self.entries[i].span.end - self.entries[i].span.start);
            let runs = owner.map(|&i| self.runs(self.entries[i].projection.expect("prose")));
            let mut pieces = Vec::new();
            for run in runs.iter().flatten() {
                let from = run.start.max(tag.span.start);
                let to = run.end.min(tag.span.end);
                if from < to {
                    pieces.push(Tag {
                        span: from..to,
                        ..tag.clone()
                    });
                }
            }
            if pieces.is_empty() {
                tags.push(tag.clone());
            } else {
                tags.extend(pieces);
            }
        }
        TagIndex::from_tags(tags)
    }

    /// The source ranges of the text of a projection, one for each line it sits on. A token
    /// that crosses a line ends a range and belongs to none.
    fn runs(&self, projection: &Projection) -> Vec<std::ops::Range<usize>> {
        let line_of = |at: usize| self.breaks.partition_point(|&b| b < at);
        let mut runs = Vec::new();
        let mut current: Option<(usize, usize)> = None;
        for token in projection.tokens() {
            if !token.kind.is_real() {
                continue;
            }
            let (start, end) = (token.src.start(), token.src.end());
            if line_of(start) != line_of(end) {
                runs.extend(current.take().map(|(a, b)| a..b));
                continue;
            }
            current = Some(current.map_or((start, end), |(a, b)| (a.min(start), b.max(end))));
        }
        runs.extend(current.map(|(a, b)| a..b));
        runs
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
    let view = View::new(path, source, doc, front_matter);
    let index = registry.tag_document(&view)?;
    Ok(view.split_interleaved(index))
}
