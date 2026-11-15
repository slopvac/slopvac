//! The text units a rule reads, cut from a parsed document by the rule's `scope`.

use std::cell::OnceCell;
use std::ops::Range;

use slopvac_model::Span;
use slopvac_parse::{BlockKind, Document, Projection, TokenKind};
use slopvac_rules::Scope;
use slopvac_text::{Sentence, TextError, split_sentences};

use crate::directives::Lines;

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
    composite: OnceCell<Result<Vec<Composite>, TextError>>,
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
            blocks.push(ProseBlock {
                kind: &block.kind,
                contained: !block.containers.is_empty(),
                projection,
                sentences: split_sentences(projection.text())?,
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

    /// The whole document as projections, with a synthetic space between blocks.
    ///
    /// A projection holds source ranges in ascending order. A block that starts before the
    /// previous one ends, such as the right cell of a table beside a wrapped left cell, would
    /// break that order, so it starts the next projection. No block is left out.
    fn composites(&self) -> Result<&[Composite], TextError> {
        self.composite
            .get_or_init(|| {
                let mut groups: Vec<Projection> = Vec::new();
                let mut projection = Projection::new();
                let mut end = 0;
                let mut first = true;
                for block in &self.blocks {
                    let tokens = block.projection.tokens();
                    let Some(head) = tokens.first() else { continue };
                    if head.src.start() < end {
                        groups.push(std::mem::take(&mut projection));
                        end = 0;
                        first = true;
                    }
                    if !first {
                        projection.push(TokenKind::Wrap, " ", Span::empty(end));
                    }
                    first = false;
                    for token in tokens {
                        projection.push(token.kind, &token.text, token.src);
                        end = token.src.end();
                    }
                }
                groups.push(projection);
                groups
                    .into_iter()
                    .map(|projection| {
                        let sentences = split_sentences(projection.text())?;
                        Ok(Composite {
                            projection,
                            sentences,
                        })
                    })
                    .collect()
            })
            .as_ref()
            .map(Vec::as_slice)
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
                for composite in self.composites()? {
                    if !composite.projection.text().is_empty() {
                        units.push(Unit {
                            text: composite.projection.text(),
                            base: 0,
                            map: Map::Projected(&composite.projection),
                            sentences: Some(&composite.sentences),
                        });
                    }
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
