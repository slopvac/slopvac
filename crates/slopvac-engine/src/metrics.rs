//! The metric rules: a counted measurement compared with a threshold.
//!
//! A port of `engine._run_metric` and `metrics` in the Python reference. A sentence metric
//! reports one hit per sentence over the threshold, a paragraph metric one per paragraph,
//! and a document metric at most one. Each hit gives the byte offset whose line the
//! finding carries; a metric finding has a line and no source range, as in 2.x.

use std::cell::OnceCell;
use std::ops::Range;

use slopvac_model::Span;
use slopvac_parse::{BlockKind, Document as Parsed};
use slopvac_rules::{Comparison, MetricMatch};
use slopvac_text::{
    TextError, TextType, bold_spans, clause_boundaries, coordinated_items, count_words,
    dashes_as_aside, is_bold_colon_bullet, longest_noun_stack, plain_words, suffix_counts,
    without_code_and_comments,
};

use crate::directives::Lines;
use crate::prepare::Prepared;

/// The sentence or paragraph that a hit measured.
#[derive(Debug, PartialEq)]
pub(crate) struct Unit<'t> {
    /// Where it sits in the source.
    pub(crate) span: Span,
    /// The projected text that holds it: its block.
    pub(crate) text: &'t str,
    /// The part of `text` that the measurement covers, without the quotation marks and the
    /// closing punctuation that wrap a quoted unit, so that a quotation detector sees a
    /// range inside the marks.
    pub(crate) range: Range<usize>,
}

/// `range` in `text` without the opening quotation marks and the closing marks and
/// sentence punctuation around it.
fn interior(text: &str, range: Range<usize>) -> Range<usize> {
    let inner = &text[range.clone()];
    let head = inner.len() - inner.trim_start_matches(['"', '“', '‘', '\'']).len();
    let tail = inner
        .trim_end_matches(['"', '”', '’', '\'', '.', '!', '?', ',', ';', ':'])
        .len();
    if head < tail {
        range.start + head..range.start + tail
    } else {
        range
    }
}

/// A measurement over the threshold.
#[derive(Debug, PartialEq)]
pub(crate) struct Hit<'t> {
    /// The measured unit, for the guards that read a source range. A document-wide
    /// measurement has none.
    pub(crate) unit: Option<Unit<'t>>,
    /// The byte offset whose line the finding reports.
    pub(crate) at: usize,
    /// The measured value, as `{match}` shows it.
    pub(crate) shown: String,
    /// The threshold, as `{replacement}` shows it.
    pub(crate) limit: String,
}

/// Whether the engine measures `metric`.
pub(crate) fn supported(metric: &str) -> bool {
    matches!(
        metric,
        "sentence_words"
            | "clause_boundaries"
            | "lead_in_words"
            | "paragraph_sentences"
            | "enumerating_paragraph_words"
            | "multiword_noun_words"
            | "coordinated_items"
            | "adjectives_per_noun"
            | "consecutive_bold_colon_bullets"
            | "bold_spans_per_1000_words"
            | "dash_per_1000_words"
    )
}

fn exceeds(value: f64, comparison: Comparison, threshold: f64) -> bool {
    match comparison {
        Comparison::Gt => value > threshold,
        Comparison::Gte => value >= threshold,
        Comparison::Lt => value < threshold,
        Comparison::Lte => value <= threshold,
    }
}

/// STE 5.1, 6.3 and 5.5: the word cap that stands in for a threshold of 0.
const fn word_cap(text_type: TextType) -> f64 {
    match text_type {
        TextType::Procedural | TextType::Safety => 20.0,
        TextType::Descriptive | TextType::Any => 25.0,
    }
}

/// A threshold as a whole number, as 2.x shows it, plus `offset`. It works on the float, so
/// a threshold beyond the integer range shows in full.
fn whole(threshold: f64, offset: usize) -> String {
    // `+ 0.0` turns -0.0, the truncation of -0.5, into 0.0.
    format!("{:.0}", threshold.trunc() + float(offset) + 0.0)
}

#[allow(clippy::cast_precision_loss)]
const fn float(count: usize) -> f64 {
    count as f64
}

/// A parsed document as the metrics read it.
pub(crate) struct Subject<'a> {
    prepared: &'a Prepared<'a>,
    parsed: &'a Parsed,
    lines: &'a Lines<'a>,
    front_matter: Option<Span>,
    prose: OnceCell<String>,
    markup: OnceCell<Result<String, TextError>>,
}

impl<'a> Subject<'a> {
    pub(crate) const fn new(
        prepared: &'a Prepared<'a>,
        parsed: &'a Parsed,
        lines: &'a Lines<'a>,
        front_matter: Option<Span>,
    ) -> Self {
        Self {
            prepared,
            parsed,
            lines,
            front_matter,
            prose: OnceCell::new(),
            markup: OnceCell::new(),
        }
    }

    /// The projected prose of every block, one block to a line.
    fn prose(&self) -> &str {
        self.prose.get_or_init(|| {
            let texts: Vec<&str> = self
                .prepared
                .blocks()
                .iter()
                .map(|b| b.projection.text())
                .collect();
            texts.join("\n")
        })
    }

    /// The source without code blocks and front matter, with markup intact.
    fn markup(&self) -> Result<&str, TextError> {
        self.markup
            .get_or_init(|| {
                let mut skip = std::collections::HashSet::new();
                let mut hide = |span: Span| {
                    let last = span.end().saturating_sub(1).max(span.start());
                    skip.extend(self.lines.line_of(span.start())..=self.lines.line_of(last));
                };
                for block in &self.parsed.blocks {
                    if matches!(block.kind, BlockKind::CodeBlock { .. }) {
                        hide(block.span);
                    }
                }
                if let Some(front) = self.front_matter {
                    hide(front);
                }
                let kept: Vec<&str> = self
                    .lines
                    .iter()
                    .filter(|(n, _)| !skip.contains(n))
                    .map(|(_, text)| text)
                    .collect();
                without_code_and_comments(&kept.join("\n"))
            })
            .as_ref()
            .map(String::as_str)
            .map_err(Clone::clone)
    }

    /// Every hit of `m`. `text_type` is the rule's: a sentence-length rule skips sentences
    /// of another type.
    pub(crate) fn measure(
        &'a self,
        m: &MetricMatch,
        text_type: TextType,
    ) -> Result<Vec<Hit<'a>>, TextError> {
        let (cmp, limit) = (m.comparison, m.threshold);
        match m.metric.as_str() {
            "sentence_words" => {
                self.sentences(
                    cmp,
                    // A threshold of 0 means the cap of the sentence's own text type.
                    |s| {
                        if limit == 0.0 {
                            word_cap(s.text_type)
                        } else {
                            limit
                        }
                    },
                    0,
                    |s| {
                        Ok((text_type == TextType::Any || s.text_type == text_type)
                            .then_some(s.word_count))
                    },
                )
            }
            "clause_boundaries" => {
                self.sentences(cmp, |_| limit, 1, |s| clause_boundaries(&s.text).map(Some))
            }
            "lead_in_words" => self.sentences(
                cmp,
                |_| limit,
                0,
                |s| {
                    s.text
                        .split_once(':')
                        .map(|(head, _)| count_words(head))
                        .transpose()
                },
            ),
            "multiword_noun_words" => {
                self.sentences(cmp, |_| limit, 0, |s| longest_noun_stack(&s.text).map(Some))
            }
            "coordinated_items" => {
                self.sentences(cmp, |_| limit, 0, |s| coordinated_items(&s.text).map(Some))
            }
            "paragraph_sentences" => self.paragraphs(cmp, limit, |b| Ok(Some(b.sentences.len()))),
            "enumerating_paragraph_words" => self.paragraphs(cmp, limit, |b| {
                let mut lists = 0;
                for s in b.sentences {
                    if coordinated_items(&s.text)? >= 3 {
                        lists += 1;
                    }
                }
                if lists < 3 {
                    return Ok(None);
                }
                count_words(b.projection.text()).map(Some)
            }),
            "adjectives_per_noun" => {
                let (adjectives, nouns) = suffix_counts(self.prose())?;
                let value = if nouns == 0 {
                    0.0
                } else {
                    float(adjectives) / float(nouns)
                };
                Ok(Self::document_hit(value, cmp, limit))
            }
            "consecutive_bold_colon_bullets" => {
                let (mut longest, mut run) = (0, 0);
                for (_, line) in self.lines.iter() {
                    if is_bold_colon_bullet(line)? {
                        run += 1;
                        longest = longest.max(run);
                    } else if !line.chars().all(char::is_whitespace) {
                        run = 0;
                    }
                }
                Ok(Self::document_hit(float(longest), cmp, limit))
            }
            "bold_spans_per_1000_words" => {
                let count = bold_spans(self.markup()?)?;
                Ok(self.density(count, cmp, limit))
            }
            "dash_per_1000_words" => {
                let count = dashes_as_aside(self.markup()?)?;
                Ok(self.density(count, cmp, limit))
            }
            _ => Ok(Vec::new()),
        }
    }

    /// One hit per sentence whose count `count` gives and which exceeds the threshold.
    /// `offset` shifts the numbers shown, for a count of boundaries that reads as clauses.
    fn sentences(
        &self,
        cmp: Comparison,
        limit: impl Fn(&slopvac_text::Sentence) -> f64,
        offset: usize,
        count: impl Fn(&slopvac_text::Sentence) -> Result<Option<usize>, TextError>,
    ) -> Result<Vec<Hit<'a>>, TextError> {
        let mut hits = Vec::new();
        for block in self.prepared.blocks() {
            for sentence in &block.sentences {
                let Some(n) = count(sentence)? else { continue };
                let limit = limit(sentence);
                if exceeds(float(n), cmp, limit) {
                    let span = block
                        .projection
                        .span_for_bytes(sentence.start, sentence.end);
                    hits.push(Hit {
                        unit: Some(Unit {
                            span,
                            text: block.projection.text(),
                            range: interior(block.projection.text(), sentence.start..sentence.end),
                        }),
                        at: span.start(),
                        shown: n.saturating_add(offset).to_string(),
                        limit: whole(limit, offset),
                    });
                }
            }
        }
        Ok(hits)
    }

    /// One hit per paragraph (not a list item or a quote) whose count exceeds the threshold.
    fn paragraphs(
        &self,
        cmp: Comparison,
        limit: f64,
        count: impl Fn(&crate::prepare::Paragraph<'_>) -> Result<Option<usize>, TextError>,
    ) -> Result<Vec<Hit<'a>>, TextError> {
        let mut hits = Vec::new();
        for block in self.prepared.paragraphs() {
            let Some(n) = count(&block)? else { continue };
            if exceeds(float(n), cmp, limit) {
                let span = block
                    .projection
                    .span_for_bytes(0, block.projection.text().len());
                hits.push(Hit {
                    unit: Some(Unit {
                        span,
                        text: block.projection.text(),
                        range: interior(block.projection.text(), 0..block.projection.text().len()),
                    }),
                    at: span.start(),
                    shown: n.to_string(),
                    limit: whole(limit, 0),
                });
            }
        }
        Ok(hits)
    }

    /// A hit at the top of the file when a document-wide value exceeds the threshold.
    fn document_hit(value: f64, cmp: Comparison, limit: f64) -> Vec<Hit<'a>> {
        if exceeds(value, cmp, limit) {
            vec![Hit {
                unit: None,
                at: 0,
                shown: format!("{value:.2}"),
                limit: format!("{limit:.2}"),
            }]
        } else {
            Vec::new()
        }
    }

    /// A markup count per 1000 words of prose.
    fn density(&self, count: usize, cmp: Comparison, limit: f64) -> Vec<Hit<'a>> {
        let words = plain_words(self.prose());
        let value = if words == 0 {
            0.0
        } else {
            float(count) / float(words) * 1000.0
        };
        Self::document_hit(value, cmp, limit)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_unit_of_a_block_reads_the_one_text_of_that_block() {
        let source = "One two. Three four. Five six. Seven eight.\n\nNine ten. Eleven twelve.\n";
        let body = Span::new(0, source.len()).unwrap();
        let parsed = slopvac_parse::parse_markdown(source, body).unwrap();
        let prepared = Prepared::new(source, &parsed).unwrap();
        let lines = Lines::new(source);
        let subject = Subject::new(&prepared, &parsed, &lines, None);
        let m: MetricMatch = serde_json::from_value(serde_json::json!(
            {"metric": "sentence_words", "threshold": 1, "comparison": "gt"}
        ))
        .unwrap();
        let hits = subject.measure(&m, TextType::Any).unwrap();
        assert_eq!(hits.len(), 6);
        let starts: Vec<*const u8> = prepared
            .blocks()
            .iter()
            .map(|b| b.projection.text().as_ptr())
            .collect();
        let texts: Vec<*const u8> = hits
            .iter()
            .map(|h| h.unit.as_ref().unwrap().text.as_ptr())
            .collect();
        // Four hits read the first block's text and two the second's, with no copy.
        assert_eq!(texts[..4], [starts[0]; 4]);
        assert_eq!(texts[4..], [starts[1]; 2]);
    }
}
