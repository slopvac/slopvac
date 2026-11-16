//! Part-of-speech tagging with Penn Treebank tags.
//!
//! This crate ports the averaged-perceptron tagger of
//! [prose v3.3.1](https://github.com/jdkato/prose) (the tagger Vale uses) and
//! embeds prose's English model. The `NN`, `VB`, `JJ` and `RB` families keep
//! their Vale meaning, so vocabulary rules written against Vale's tags keep
//! their semantics.
//!
//! [`Tagger::tag_text`] is the entry point. It takes UTF-8 prose, skips inline
//! code spans, keeps hyphenated words whole, splits sentences, and returns one
//! [`TaggedToken`] per token with its byte span in the input. Stripping other
//! Markdown or HTML markup is the caller's job; this crate sees only prose.
//!
//! # Licensing
//!
//! The code is Apache-2.0. The embedded model, `model/en.bin`, is derived from
//! UD_English-EWT and is licensed CC BY-SA 4.0; see `model/LICENSE-MODEL.md`.
//!
//! # Differences from prose
//!
//! - The emoticon shortcut (`:)` tagged `SYM`) is not ported.
//! - Only the English rune-template Penn model is supported; other models are
//!   rejected by [`Tagger::from_model_bytes`].

mod go_unicode;
mod model;
mod tokenize;
mod unicode_tables;

use tokenize::work;

use std::ops::Range;
use std::sync::OnceLock;

pub use model::ModelError;

/// The embedded English model (CC BY-SA 4.0, see `model/LICENSE-MODEL.md`).
const ENGLISH_MODEL: &[u8] = include_bytes!("../model/en.bin");

/// One token of the input and its Penn Treebank tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedToken<'a> {
    /// The token text, a slice of the input.
    pub text: &'a str,
    /// UTF-8 byte offsets of the token in the input.
    pub span: Range<usize>,
    /// The Penn Treebank tag, for example `NN`, `VBZ` or `JJ`. Punctuation
    /// carries its own tag (`.`, `,`, `:`).
    pub tag: &'a str,
}

/// A stretch of text to present to the tagger as an inline code span.
///
/// For text that is already prose, where the caller knows what is code. A code span
/// shows as a run of `*`, as in Vale, and produces no token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    /// UTF-8 byte offsets of the span in the text.
    pub range: Range<usize>,
    /// How many `*` stand in for the span. Vale uses the number of characters between the
    /// backticks. Zero counts as one.
    pub stars: usize,
}

/// A part-of-speech tagger.
#[derive(Debug)]
pub struct Tagger {
    model: model::Model,
}

impl Tagger {
    /// The tagger for the embedded English model, built on first use.
    ///
    /// # Panics
    ///
    /// Never in a released build: a test decodes the embedded model.
    #[must_use]
    pub fn english() -> &'static Self {
        static ENGLISH: OnceLock<Tagger> = OnceLock::new();
        ENGLISH.get_or_init(|| {
            Self::from_model_bytes(ENGLISH_MODEL).expect("the embedded model decodes")
        })
    }

    /// Builds a tagger from a model in prose's flat format (`PROSETAG`).
    pub fn from_model_bytes(data: &[u8]) -> Result<Self, ModelError> {
        Ok(Self {
            model: model::Model::parse(data)?,
        })
    }

    /// Tags UTF-8 prose.
    ///
    /// Tokens come back in input order. Inline code spans produce no tokens.
    /// Each sentence is tagged on its own, as prose does.
    #[must_use]
    pub fn tag_text<'a>(&'a self, text: &'a str) -> Vec<TaggedToken<'a>> {
        self.tag_blocks(text, std::slice::from_ref(&(0..text.len())))
    }

    /// Tags the given byte ranges of `text` as separate blocks.
    ///
    /// Vale tags each Markdown block (a paragraph, list item, heading or table
    /// cell) on its own, and sentences never cross blocks. A scope layer that
    /// knows the blocks passes their ranges here. Spans in the result are
    /// offsets into `text`, not into the block. Each range must lie on
    /// character boundaries and inside `text`.
    ///
    /// # Panics
    ///
    /// If a range is out of bounds or splits a UTF-8 sequence.
    #[must_use]
    pub fn tag_blocks<'a>(
        &'a self,
        text: &'a str,
        blocks: &[Range<usize>],
    ) -> Vec<TaggedToken<'a>> {
        self.tag_with(text, blocks, None)
    }

    /// Like [`Tagger::tag_blocks`], for text whose code spans the caller knows.
    ///
    /// Nothing in `text` is read as Markdown: a backtick is an ordinary character, and
    /// each [`Mask`] is a code span. Masks must ascend, not overlap, and each must lie
    /// inside one block.
    ///
    /// # Panics
    ///
    /// If a block or mask is out of bounds, splits a UTF-8 sequence, or a mask overlaps
    /// another or crosses the edge of a block.
    #[must_use]
    pub fn tag_blocks_masked<'a>(
        &'a self,
        text: &'a str,
        blocks: &[Range<usize>],
        masks: &[Mask],
    ) -> Vec<TaggedToken<'a>> {
        self.tag_with(text, blocks, Some(masks))
    }

    fn tag_with<'a>(
        &'a self,
        text: &'a str,
        blocks: &[Range<usize>],
        masks: Option<&[Mask]>,
    ) -> Vec<TaggedToken<'a>> {
        let mut out = Vec::new();
        for block in blocks {
            let base = block.start;
            let slice = &text[block.clone()];
            let tokens = match masks {
                None => tokenize::tokenize(slice),
                Some(masks) => {
                    let local: Vec<(Range<usize>, usize)> = masks
                        .iter()
                        .filter(|m| m.range.start < block.end && m.range.end > block.start)
                        .map(|m| {
                            assert!(
                                m.range.start >= block.start && m.range.end <= block.end,
                                "a mask crosses the edge of a block"
                            );
                            (m.range.start - base..m.range.end - base, m.stars)
                        })
                        .collect();
                    assert!(
                        local.windows(2).all(|w| w[0].0.end <= w[1].0.start),
                        "masks must ascend and not overlap"
                    );
                    tokenize::tokenize_masked(slice, &local)
                }
            };
            for sentence in tokenize::sentences(slice, &tokens) {
                work::tick(sentence.len());
                let words: Vec<&str> = tokens[sentence.clone()]
                    .iter()
                    .map(|t| t.word.as_str())
                    .collect();
                let tags = self.model.tag(&words);
                for (token, tag) in tokens[sentence].iter().zip(tags) {
                    if !token.placeholder {
                        let span = base + token.span.start..base + token.span.end;
                        out.push(TaggedToken {
                            text: &text[span.clone()],
                            span,
                            tag,
                        });
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_a_simple_sentence() {
        let tagger = Tagger::english();
        let got: Vec<(&str, &str)> = tagger
            .tag_text("The queue is empty.")
            .iter()
            .map(|t| (t.text, t.tag))
            .collect();
        assert_eq!(
            got,
            [
                ("The", "DT"),
                ("queue", "NN"),
                ("is", "VBZ"),
                ("empty", "JJ"),
                (".", ".")
            ]
        );
    }

    #[test]
    fn inline_code_is_not_reported_and_spans_are_utf8_bytes() {
        let text = "Caf\u{e9}: run `kubectl get` now.";
        let got: Vec<(&str, std::ops::Range<usize>)> = Tagger::english()
            .tag_text(text)
            .into_iter()
            .map(|t| (t.text, t.span))
            .collect();
        assert_eq!(
            got,
            [
                ("Caf\u{e9}", 0..5),
                (":", 5..6),
                ("run", 7..10),
                ("now", 25..28),
                (".", 28..29),
            ]
        );
    }

    #[test]
    fn blocks_are_tagged_independently_with_spans_in_the_whole_text() {
        // Two list items without terminal punctuation: as one text they form a
        // single sentence; as blocks they are two.
        let text = "- Ensure the server restarts\n- Added the feature";
        let tagger = Tagger::english();
        let first = 2..28;
        let second = 31..text.len();
        let blocks = tagger.tag_blocks(text, &[first.clone(), second.clone()]);
        let alone: Vec<(String, usize, String)> = [first, second]
            .into_iter()
            .flat_map(|b| {
                let base = b.start;
                tagger
                    .tag_text(&text[b])
                    .into_iter()
                    .map(move |t| (t.text.to_owned(), t.span.start + base, t.tag.to_owned()))
            })
            .collect();
        let got: Vec<(String, usize, String)> = blocks
            .iter()
            .map(|t| (t.text.to_owned(), t.span.start, t.tag.to_owned()))
            .collect();
        assert_eq!(got, alone);
        assert!(blocks.iter().all(|t| &text[t.span.clone()] == t.text));
        assert_eq!(blocks.first().map(|t| t.text), Some("Ensure"));
        assert_eq!(blocks.last().map(|t| t.text), Some("feature"));
    }

    #[test]
    fn an_empty_input_has_no_tokens() {
        assert!(matches!(Tagger::english().tag_text("").as_slice(), []));
    }

    fn pairs<'a>(tokens: &[TaggedToken<'a>]) -> Vec<(&'a str, std::ops::Range<usize>, &'a str)> {
        tokens
            .iter()
            .map(|t| (t.text, t.span.clone(), t.tag))
            .collect()
    }

    #[test]
    fn a_mask_over_a_code_span_tags_like_the_backtick_form() {
        let text = "Run `kubectl get` now, then wait.";
        let tagger = Tagger::english();
        let found = tagger.tag_blocks(text, std::slice::from_ref(&(0..text.len())));
        // The span `kubectl get` is 13 bytes at 4..17, with 11 characters inside.
        let masks = [Mask {
            range: 4..17,
            stars: 11,
        }];
        let given = tagger.tag_blocks_masked(text, std::slice::from_ref(&(0..text.len())), &masks);
        assert_eq!(pairs(&given), pairs(&found));
    }

    #[test]
    fn backticks_are_ordinary_text_when_masks_are_given() {
        // Found as Markdown, the two backticks would open and close a code span that
        // hides "the deploy". With explicit masks they are punctuation.
        let text = "A ` mark, the deploy and ` end.";
        let tagger = Tagger::english();
        let words = |tokens: Vec<TaggedToken<'_>>| -> Vec<String> {
            tokens.iter().map(|t| t.text.to_owned()).collect()
        };
        assert!(!words(tagger.tag_text(text)).contains(&"deploy".to_owned()));
        let masked = tagger.tag_blocks_masked(text, std::slice::from_ref(&(0..text.len())), &[]);
        assert!(words(masked).contains(&"deploy".to_owned()));
    }

    #[test]
    fn a_mask_drops_its_own_token_and_keeps_spans_in_the_whole_text() {
        let text = "ab\n\nXX the deploy";
        let tagger = Tagger::english();
        let masks = [Mask {
            range: 4..6,
            stars: 2,
        }];
        let got = tagger.tag_blocks_masked(text, &[0..2, 4..text.len()], &masks);
        let words: Vec<(&str, std::ops::Range<usize>)> =
            got.iter().map(|t| (t.text, t.span.clone())).collect();
        assert_eq!(words, [("ab", 0..2), ("the", 7..10), ("deploy", 11..17)]);
    }

    #[test]
    #[should_panic(expected = "crosses the edge of a block")]
    fn a_mask_across_a_block_edge_is_refused() {
        let text = "abcdef";
        let masks = [Mask {
            range: 1..4,
            stars: 1,
        }];
        let _ = Tagger::english().tag_blocks_masked(text, &[0..3, 3..6], &masks);
    }

    /// Work units allowed per input byte for tokenizing and tagging together,
    /// and a constant allowance. The linear code spends at most 175 units per
    /// byte on these inputs (a word costs about 500 in the model).
    const PER_BYTE: u64 = 200;
    const ALLOWANCE: u64 = 1_000;

    fn budget(text: &str) -> u64 {
        PER_BYTE * text.len() as u64 + ALLOWANCE
    }

    fn work_of(f: impl FnOnce()) -> u64 {
        work::take();
        f();
        work::take()
    }

    /// The same words, with a line break after every hundred spaces.
    fn one_line_per_hundred_words(text: &str) -> String {
        let mut out = String::with_capacity(text.len() + text.len() / 100);
        for (i, piece) in text.split_inclusive(' ').enumerate() {
            out.push_str(piece);
            if i % 100 == 99 {
                out.push('\n');
            }
        }
        out
    }

    fn tag_text_work(text: &str) -> u64 {
        work_of(|| {
            std::hint::black_box(Tagger::english().tag_text(text));
        })
    }

    /// One block per line, as a scope layer would hand them over.
    fn tag_blocks_work(text: &str) -> u64 {
        let mut blocks = Vec::new();
        let mut start = 0;
        for line in text.split_inclusive('\n') {
            blocks.push(start..start + line.len());
            start += line.len();
        }
        if blocks.is_empty() {
            blocks.push(0..0);
        }
        work_of(|| {
            std::hint::black_box(Tagger::english().tag_blocks(text, &blocks));
        })
    }

    #[test]
    fn tagging_stays_within_a_linear_work_budget_on_adversarial_inputs() {
        type Measure = (&'static str, fn(&str) -> u64);
        let measures: [Measure; 2] = [("tag_text", tag_text_work), ("tag_blocks", tag_blocks_work)];
        for name in tokenize::adversarial::NAMES {
            let n = if name == "backtick runs" {
                200_000
            } else {
                40_000
            };
            let text = tokenize::adversarial::adversarial(name, n);
            let lined = one_line_per_hundred_words(&text);
            for (api, measure) in measures {
                let input = if api == "tag_blocks" { &lined } else { &text };
                let used = measure(input);
                assert!(
                    used <= budget(input),
                    "{api} on {name}: {used} units for {} bytes exceeds {}",
                    input.len(),
                    budget(input)
                );
                let doubled_input = if api == "tag_blocks" {
                    format!("{input}{input}")
                } else {
                    tokenize::adversarial::adversarial(name, 2 * n)
                };
                let doubled = measure(&doubled_input);
                assert!(
                    doubled * 10 <= used * 22 + 10_000,
                    "{api} on {name}: doubling the input took work from {used} to {doubled}"
                );
            }
        }
    }

    /// Defects put back into the tagging path, one per loop. Each must exceed
    /// the budget, which is what makes the gate above meaningful.
    #[test]
    fn the_budget_rejects_a_context_rebuilt_for_every_word() {
        let text = tokenize::adversarial::adversarial("long sentence", 20_000);
        let used = work_of(|| {
            let tokens = tokenize::tokenize(&text);
            for sentence in tokenize::sentences(&text, &tokens) {
                let words: Vec<&str> = tokens[sentence].iter().map(|t| t.word.as_str()).collect();
                std::hint::black_box(model::oracle::tag_rebuilding_context(
                    &Tagger::english().model,
                    &words,
                ));
            }
        });
        assert!(
            used > budget(&text),
            "only {used} units for {} bytes (budget {})",
            text.len(),
            budget(&text)
        );
    }

    #[test]
    fn the_budget_rejects_tagging_every_sentence_with_all_the_words_before_it() {
        let text = "Yes! ".repeat(2_000);
        let used = work_of(|| {
            let tokens = tokenize::tokenize(&text);
            for sentence in tokenize::sentences(&text, &tokens) {
                work::tick(sentence.end);
                let words: Vec<&str> = tokens[..sentence.end]
                    .iter()
                    .map(|t| t.word.as_str())
                    .collect();
                std::hint::black_box(Tagger::english().model.tag(&words));
            }
        });
        assert!(
            used > budget(&text),
            "only {used} units for {} bytes (budget {})",
            text.len(),
            budget(&text)
        );
    }
}
