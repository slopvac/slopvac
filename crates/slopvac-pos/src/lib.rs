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

mod model;
mod tokenize;

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
        let tokens = tokenize::tokenize(text);
        let mut out = Vec::with_capacity(tokens.len());
        for sentence in tokenize::sentences(text, &tokens) {
            let words: Vec<&str> = tokens[sentence.clone()]
                .iter()
                .map(|t| t.word.as_str())
                .collect();
            let tags = self.model.tag(&words);
            for (token, tag) in tokens[sentence].iter().zip(tags) {
                if !token.placeholder {
                    out.push(TaggedToken {
                        text: &text[token.span.clone()],
                        span: token.span.clone(),
                        tag,
                    });
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
    fn an_empty_input_has_no_tokens() {
        assert!(matches!(Tagger::english().tag_text("").as_slice(), []));
    }
}
