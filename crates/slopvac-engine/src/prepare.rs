//! The text units a rule reads, cut from a parsed document by the rule's `scope`.

use std::cell::OnceCell;
use std::ops::Range;

use slopvac_model::SourceIndex;
use slopvac_model::Span;
use slopvac_parse::{
    Block, BlockKind, Document, ParseError, Projection, TokenKind, parse_html_segments,
};
use slopvac_rules::Scope;
use slopvac_text::{Sentence, TextError, cell_sentence, split_sentences};

use crate::directives::Lines;

/// `doc` with its raw HTML blocks (`<td>`, `<em>`, `<p>`) read as 2.x reads them: the prose
/// of one block is one paragraph of the kind `Paragraph`, in source order. Every consumer of
/// a document then sees the same text: rules of every kind, the counts, the metrics, the
/// context tags and the skip ranges. Sentences span the elements of the block, so a cell
/// without a final period runs into the next one.
///
/// As in 2.x, the paragraph is not a heading, table cell or list item whatever elements it
/// came from, and the lists and quotes of the enclosing Markdown do not hold it. Inline code
/// in the HTML (`<code>`) joins the code spans, and `<pre>` stays a code block. A Markdown
/// fence inside the block is code, not prose: it becomes a code block and is not shown to
/// the HTML parser.
///
/// # Errors
/// Returns a [`ParseError`] when an HTML block is too large or its segments are invalid.
pub(crate) fn with_html_prose(
    source: &str,
    index: &SourceIndex<'_>,
    mut doc: Document,
) -> Result<Document, ParseError> {
    let html_blocks = std::mem::take(&mut doc.html_blocks);
    for block in html_blocks {
        let (segments, fences) = split_fences(source, &block.segments);
        for fence in fences {
            doc.blocks.push(Block {
                kind: BlockKind::CodeBlock {
                    lang: None,
                    info: None,
                    body: fence,
                    fenced: true,
                },
                span: fence,
                projection: None,
                containers: Vec::new(),
            });
        }
        if segments.is_empty() {
            continue;
        }
        let html = parse_html_segments(source, index, &segments)?;
        doc.code_spans.extend(html.code_spans);
        let mut prose = Vec::new();
        for mut inner in html.blocks {
            if has_prose(&inner) {
                prose.push(inner);
            } else {
                inner.containers.clear();
                doc.blocks.push(inner);
            }
        }
        let (Some(first), Some(end)) = (
            prose.iter().map(|b| b.span.start()).min(),
            prose.iter().map(|b| b.span.end()).max(),
        ) else {
            continue;
        };
        doc.blocks.push(Block {
            kind: BlockKind::Paragraph,
            span: Span::new(first, end).expect("a block ends after it starts"),
            projection: Some(join(prose.iter().filter_map(|b| b.projection.as_ref()))),
            containers: Vec::new(),
        });
    }
    // Stable, so a container keeps its place before the blocks inside it.
    doc.blocks
        .sort_by_key(|b| (b.span.start(), std::cmp::Reverse(b.span.end())));
    doc.code_spans.sort_by_key(Span::start);
    Ok(doc)
}

/// The fence character and marker length when `line` opens or closes a Markdown fence:
/// 2.x's `_FENCE_START`, up to three whitespace characters of indent and three or more of `` ` `` or `~`.
fn fence_marker(line: &str) -> Option<(char, usize)> {
    // `\s{0,3}`: up to three whitespace characters of any kind, tabs included.
    let rest = line.trim_start_matches(char::is_whitespace);
    if line.chars().count() - rest.chars().count() > 3 {
        return None;
    }
    let ch = rest.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let run = rest.chars().take_while(|&c| c == ch).count();
    (run >= 3).then_some((ch, run))
}

/// The lines of `text` with their terminators: LF, CRLF or a lone CR ends a line.
fn split_lines(text: &str) -> impl Iterator<Item = &str> {
    let bytes = text.as_bytes();
    let mut start = 0;
    std::iter::from_fn(move || {
        if start >= bytes.len() {
            return None;
        }
        let mut end = start;
        while end < bytes.len() && !matches!(bytes[end], b'\n' | b'\r') {
            end += 1;
        }
        if end < bytes.len() {
            end += if bytes[end] == b'\r' && bytes.get(end + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
        }
        let line = &text[start..end];
        start = end;
        Some(line)
    })
}

/// The segments of an HTML block without the lines of Markdown fences inside it, and the
/// source span of each fence (an unclosed one runs to the end of the block), as 2.x's
/// `_html_fence_lines` finds them. The HTML parser does not know Markdown fences, so it
/// would read their contents as prose.
fn split_fences(source: &str, segments: &[Span]) -> (Vec<Span>, Vec<Span>) {
    // Each line: where it starts, where it ends with its terminator, and its text without.
    let mut lines: Vec<(usize, usize, &str)> = Vec::new();
    for segment in segments {
        let mut at = segment.start();
        for line in split_lines(&source[segment.start()..segment.end()]) {
            lines.push((at, at + line.len(), line.trim_end_matches(['\r', '\n'])));
            at += line.len();
        }
    }
    let mut in_fence = vec![false; lines.len()];
    let mut fences = Vec::new();
    let mut open: Option<(char, usize, usize)> = None;
    for (i, (_, _, text)) in lines.iter().enumerate() {
        let marker = fence_marker(text);
        match open {
            None => {
                if let Some((ch, run)) = marker {
                    open = Some((ch, run, i));
                }
            }
            Some((ch, run, from)) => {
                if marker.is_some_and(|(c, n)| c == ch && n >= run) {
                    in_fence[from..=i].fill(true);
                    fences.push((from, i));
                    open = None;
                }
            }
        }
    }
    if let Some((_, _, from)) = open {
        in_fence[from..].fill(true);
        fences.push((from, lines.len() - 1));
    }
    let span =
        |start: usize, end: usize| Span::new(start, end).expect("a line ends after it starts");
    let fence_spans = fences
        .into_iter()
        .map(|(from, to)| span(lines[from].0, lines[to].0 + lines[to].2.len()))
        .collect();
    let mut kept: Vec<Span> = Vec::new();
    for (i, &(start, end, _)) in lines.iter().enumerate() {
        if in_fence[i] {
            continue;
        }
        match kept.last_mut() {
            Some(last) if last.end() == start => *last = span(last.start(), end),
            _ => kept.push(span(start, end)),
        }
    }
    (kept, fence_spans)
}

fn has_prose(block: &Block) -> bool {
    block
        .projection
        .as_ref()
        .is_some_and(|p| !p.text().is_empty())
}

/// `projections` as one text, with a synthetic space between them.
fn join<'p>(projections: impl Iterator<Item = &'p Projection>) -> Projection {
    let mut joined = Projection::new();
    let mut end = 0;
    let mut first = true;
    for projection in projections {
        let tokens = projection.tokens();
        if tokens.is_empty() {
            continue;
        }
        if !first {
            // The space between blocks bridges the source between them.
            joined.push_bridge(TokenKind::Wrap, " ", Span::empty(end));
        }
        first = false;
        for (i, token) in tokens.iter().enumerate() {
            if projection.is_bridge(i) {
                joined.push_bridge(token.kind, &token.text, token.src);
            } else {
                joined.push_unordered(token.kind, &token.text, token.src);
            }
            end = token.src.end();
        }
    }
    joined
}

/// How a unit's offsets map to source bytes.
#[derive(Clone, Copy)]
enum Map<'p> {
    /// The unit is projected prose.
    Projected(&'p Projection),
    /// The unit is a slice of the source.
    Source,
}

/// One text a lexical rule matches in.
pub(crate) struct Unit<'p> {
    /// The text to match.
    pub(crate) text: &'p str,
    /// Where `text` starts: in the projection, or in the source for a `raw` unit.
    base: usize,
    map: Map<'p>,
    /// The sentences of the projection that holds `text`, in the projection's offsets.
    pub(crate) sentences: Option<&'p [Sentence]>,
}

impl Unit<'_> {
    /// Where `text` starts in the projection `sentences` was cut from.
    pub(crate) const fn base(&self) -> usize {
        self.base
    }

    /// The source ranges a match is made of, given in bytes of `text`: one for each run of
    /// source text its tokens cover. Text that reads in the source order has one, the span.
    #[allow(clippy::single_range_in_vec_init)]
    pub(crate) fn ranges(&self, range: &Range<usize>) -> Vec<Range<usize>> {
        let (start, end) = (self.base + range.start, self.base + range.end);
        match self.map {
            Map::Projected(p) => {
                let extents = p.extents_in_bytes(start, end);
                if extents.is_empty() {
                    let span = self.span(range);
                    vec![span.start()..span.end()]
                } else {
                    extents.iter().map(|s| s.start()..s.end()).collect()
                }
            }
            Map::Source => vec![start..end],
        }
    }

    /// The source span of a match, given in bytes of `text`.
    pub(crate) fn span(&self, range: &Range<usize>) -> Span {
        let (start, end) = (self.base + range.start, self.base + range.end);
        match self.map {
            Map::Projected(p) => p.span_for_bytes(start, end),
            Map::Source => Span::new(start, end).expect("a match ends after it starts"),
        }
    }
}

/// A block with prose.
pub(crate) struct ProseBlock<'a> {
    pub(crate) kind: &'a BlockKind,
    /// Whether a list item or a block quote holds the block.
    pub(crate) contained: bool,
    pub(crate) projection: &'a Projection,
    pub(crate) sentences: Vec<Sentence>,
}

struct Composite {
    projection: Projection,
    sentences: Vec<Sentence>,
}

/// A parsed document with its sentences split.
pub(crate) struct Prepared<'a> {
    source: &'a str,
    blocks: Vec<ProseBlock<'a>>,
    composite: OnceCell<Result<Composite, TextError>>,
}

impl<'a> Prepared<'a> {
    pub(crate) fn new(source: &'a str, doc: &'a Document) -> Result<Self, TextError> {
        let mut blocks = Vec::new();
        for block in &doc.blocks {
            let Some(projection) = &block.projection else {
                continue;
            };
            if projection.text().is_empty() {
                continue;
            }
            // The reference counts a table cell as one sentence, whatever it contains.
            let sentences = if matches!(block.kind, BlockKind::TableCell { .. }) {
                cell_sentence(projection.text())?.into_iter().collect()
            } else {
                split_sentences(projection.text())?
            };
            blocks.push(ProseBlock {
                kind: &block.kind,
                contained: !block.containers.is_empty(),
                projection,
                sentences,
            });
        }
        Ok(Self {
            source,
            blocks,
            composite: OnceCell::new(),
        })
    }

    /// The blocks with prose, in document order.
    pub(crate) fn blocks(&self) -> &[ProseBlock<'a>] {
        &self.blocks
    }

    /// Words in the sentences of every prose block.
    pub(crate) fn words(&self) -> usize {
        self.blocks
            .iter()
            .flat_map(|b| &b.sentences)
            .map(|s| s.word_count)
            .sum()
    }

    pub(crate) fn sentence_count(&self) -> usize {
        self.blocks.iter().map(|b| b.sentences.len()).sum()
    }

    pub(crate) fn paragraph_count(&self) -> usize {
        self.blocks
            .iter()
            .filter(|b| matches!(b.kind, BlockKind::Paragraph) && !b.contained)
            .count()
    }

    /// The whole document as one projection, with a synthetic space between blocks.
    ///
    /// Blocks read in block order. That is not always source order: the right cell of a
    /// table beside a wrapped left cell starts in the source before the left cell ends. Such
    /// a block still joins the one text, so a document rule sees every block, anchors and
    /// sentence splits work over all of it, and a match that crosses blocks maps to the hull
    /// of the source spans of its tokens.
    fn composite(&self) -> Result<&Composite, TextError> {
        self.composite
            .get_or_init(|| {
                let projection = join(self.blocks.iter().map(|b| b.projection));
                let sentences = split_sentences(projection.text())?;
                Ok(Composite {
                    projection,
                    sentences,
                })
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// The units a rule of `scope` matches in.
    pub(crate) fn units(&self, scope: Scope) -> Result<Vec<Unit<'_>>, TextError> {
        let mut units = Vec::new();
        match scope {
            Scope::Raw => {
                let lines = Lines::new(self.source);
                for (number, text) in lines.iter() {
                    units.push(Unit {
                        text,
                        base: lines.start(number),
                        map: Map::Source,
                        sentences: None,
                    });
                }
            }
            Scope::Document => {
                let composite = self.composite()?;
                if !composite.projection.text().is_empty() {
                    units.push(Unit {
                        text: composite.projection.text(),
                        base: 0,
                        map: Map::Projected(&composite.projection),
                        sentences: Some(&composite.sentences),
                    });
                }
            }
            Scope::Prose | Scope::Paragraph | Scope::Heading => {
                for block in &self.blocks {
                    if scope == Scope::Heading && !matches!(block.kind, BlockKind::Heading { .. }) {
                        continue;
                    }
                    units.push(Unit {
                        text: block.projection.text(),
                        base: 0,
                        map: Map::Projected(block.projection),
                        sentences: Some(&block.sentences),
                    });
                }
            }
            Scope::Sentence | Scope::BodySentence => {
                for block in &self.blocks {
                    if scope == Scope::BodySentence
                        && matches!(
                            block.kind,
                            BlockKind::Heading { .. } | BlockKind::TableCell { .. }
                        )
                    {
                        continue;
                    }
                    for s in &block.sentences {
                        units.push(Unit {
                            text: &block.projection.text()[s.start..s.end],
                            base: s.start,
                            map: Map::Projected(block.projection),
                            sentences: Some(&block.sentences),
                        });
                    }
                }
            }
        }
        Ok(units)
    }
}

#[cfg(test)]
mod tests {
    use slopvac_parse::{Block, Container};

    use super::*;

    fn block(tokens: &[(TokenKind, &str, usize, usize)]) -> Block {
        let mut projection = Projection::new();
        for &(kind, text, start, end) in tokens {
            projection.push(kind, text, Span::new(start, end).unwrap());
        }
        let start = tokens.iter().map(|t| t.2).min().unwrap();
        let end = tokens.iter().map(|t| t.3).max().unwrap();
        Block {
            kind: BlockKind::Paragraph,
            span: Span::new(start, end).unwrap(),
            projection: Some(projection),
            containers: Vec::<Container>::new(),
        }
    }

    /// A left cell that wraps (source 10..44) and a right cell inside its extent (20..25).
    fn interleaved() -> Document {
        Document {
            blocks: vec![
                block(&[
                    (TokenKind::Identity, "Left", 10, 14),
                    (TokenKind::Collapse, " ", 14, 15),
                    (TokenKind::Identity, "tail.", 39, 44),
                ]),
                block(&[(TokenKind::Identity, "Right", 20, 25)]),
            ],
            code_spans: Vec::new(),
            html_blocks: Vec::new(),
        }
    }

    #[test]
    fn interleaved_blocks_make_one_document_text_in_reading_order() {
        let doc = interleaved();
        let prepared = Prepared::new("", &doc).unwrap();
        let units = prepared.units(Scope::Document).unwrap();
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].text, "Left tail. Right");
        // The sentence split sees the whole text: one full sentence, then the next one.
        let sentences = units[0].sentences.unwrap();
        assert_eq!(sentences.len(), 2);
    }

    #[test]
    fn a_match_over_interleaved_tokens_maps_to_the_hull_of_their_spans() {
        let doc = interleaved();
        let prepared = Prepared::new("", &doc).unwrap();
        let units = prepared.units(Scope::Document).unwrap();
        let from = units[0].text.find("tail").unwrap();
        let to = units[0].text.len();
        // "tail." is at 39..44 and "Right" at 20..25: the hull is 20..44.
        let span = units[0].span(&(from..to));
        assert_eq!((span.start(), span.end()), (20, 44));
        // Inside one token the span stays exact.
        let at = units[0].text.find("Right").unwrap();
        let exact = units[0].span(&(at..at + 5));
        assert_eq!((exact.start(), exact.end()), (20, 25));
    }
}
