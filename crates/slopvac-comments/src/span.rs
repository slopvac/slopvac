//! Byte spans and the projection that maps extracted text back to them.
//!
//! These types follow `docs/contract/positions.md` and match the shape of
//! `slopvac_model::Span` and `slopvac_parse::Projection`. Neither is on `main` yet, so
//! this crate keeps a small local copy. Swapping in the shared types is a rename.

use thiserror::Error;

/// A half-open range `[start, end)` of byte offsets into the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Span {
    start: usize,
    end: usize,
}

/// A span whose start is after its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("span start {start} is after end {end}")]
pub struct SpanError {
    /// Start offset.
    pub start: usize,
    /// End offset.
    pub end: usize,
}

impl Span {
    /// Build a span, rejecting `start > end`.
    pub const fn new(start: usize, end: usize) -> Result<Self, SpanError> {
        if start > end {
            Err(SpanError { start, end })
        } else {
            Ok(Self { start, end })
        }
    }

    /// A zero-width span at `offset`.
    #[must_use]
    pub const fn empty(offset: usize) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    /// First byte of the span.
    #[must_use]
    pub const fn start(&self) -> usize {
        self.start
    }

    /// First byte after the span.
    #[must_use]
    pub const fn end(&self) -> usize {
        self.end
    }

    /// Whether the span has no width.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Length in bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.end - self.start
    }
}

/// How a token's projected text relates to its source bytes.
///
/// Comment text is never decoded, so only two of the contract's kinds occur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// The source characters themselves. Each code point maps to its own bytes.
    Identity,
    /// One synthetic space where two comment lines join. Its source range is empty.
    Wrap,
}

/// One run of projected text and the source range it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Kind.
    pub kind: TokenKind,
    /// Projected text.
    pub text: String,
    /// Source byte range.
    pub src: Span,
}

/// A projected text and its mapping to source bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Projection {
    tokens: Vec<Token>,
    text: String,
}

impl Projection {
    /// An empty projection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a token. Source ranges must ascend and not overlap.
    ///
    /// # Panics
    /// Panics if `src` starts before the previous token's source end.
    pub fn push(&mut self, kind: TokenKind, text: &str, src: Span) {
        if let Some(last) = self.tokens.last() {
            assert!(
                src.start() >= last.src.end(),
                "token range {src:?} overlaps the previous token {:?}",
                last.src
            );
        }
        self.text.push_str(text);
        self.tokens.push(Token {
            kind,
            text: text.to_owned(),
            src,
        });
    }

    /// The projected text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The tokens in order.
    #[must_use]
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    /// Map a projected range, in code points, to a source span.
    ///
    /// Synthetic points at the edges are dropped. With real points left, the span runs
    /// from the smallest source start to the largest source end. Otherwise it is empty
    /// at the source start of the point at `start`, or at the source end of the last
    /// token when `start` is at the end of the text.
    #[must_use]
    pub fn span_for_range(&self, start: usize, end: usize) -> Span {
        let mut lo = usize::MAX;
        let mut hi = 0;
        let mut found = false;
        let mut point_at_start = None;
        let mut cursor = 0;
        for token in &self.tokens {
            let len = token.text.chars().count();
            let (t0, t1) = (cursor, cursor + len);
            cursor = t1;
            if start >= t0 && start < t1 && point_at_start.is_none() {
                point_at_start = Some(match token.kind {
                    TokenKind::Identity => token.src.start() + char_byte(&token.text, start - t0),
                    TokenKind::Wrap => token.src.start(),
                });
            }
            let (a, b) = (start.max(t0), end.min(t1));
            if a >= b || token.kind == TokenKind::Wrap {
                continue;
            }
            lo = lo.min(token.src.start() + char_byte(&token.text, a - t0));
            hi = hi.max(token.src.start() + char_byte(&token.text, b - t0));
            found = true;
        }
        if found {
            return Span { start: lo, end: hi };
        }
        let at = point_at_start
            .or_else(|| self.tokens.last().map(|t| t.src.end()))
            .unwrap_or(0);
        Span::empty(at)
    }

    /// Map a projected range given in bytes of [`Self::text`] to a source span.
    ///
    /// # Panics
    /// Panics if an offset is not on a character boundary of the projected text.
    #[must_use]
    pub fn span_for_bytes(&self, start: usize, end: usize) -> Span {
        let cp = |b: usize| self.text[..b].chars().count();
        self.span_for_range(cp(start), cp(end))
    }
}

/// Byte offset of the `n`th character of `text`, or its length.
fn char_byte(text: &str, n: usize) -> usize {
    text.char_indices().nth(n).map_or(text.len(), |(i, _)| i)
}
