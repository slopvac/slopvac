//! Projection: the mapping from extracted prose back to source bytes.
//!
//! Rules match projected text. A parser drops markup, decodes entities and joins soft
//! wraps, so a match offset is not a source offset. A [`Projection`] carries the mapping
//! as an ordered list of tokens. See `docs/contract/positions.md`.

use slopvac_model::Span;

/// How a token's projected text relates to its source bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// The source characters themselves. Each code point maps to its own bytes.
    Identity,
    /// A decoded entity. Maps to the whole entity.
    Entity,
    /// A backslash escape. Maps to the backslash and the character.
    Backslash,
    /// Text a syntax rule produced, such as a JSX string expression.
    Opaque,
    /// One space for a whitespace run. Maps to the whole run.
    Collapse,
    /// A placeholder for text that rules must not read, such as inline code.
    Masked,
    /// One synthetic space for a soft wrap. Has an empty source range.
    Wrap,
}

impl TokenKind {
    /// Whether the token has a real, non-empty source range.
    #[must_use]
    pub const fn is_real(self) -> bool {
        !matches!(self, Self::Wrap)
    }
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
                    _ => token.src.start(),
                });
            }
            let (a, b) = (start.max(t0), end.min(t1));
            if a >= b || !token.kind.is_real() {
                continue;
            }
            let (s, e) = if token.kind == TokenKind::Identity {
                (
                    token.src.start() + char_byte(&token.text, a - t0),
                    token.src.start() + char_byte(&token.text, b - t0),
                )
            } else {
                (token.src.start(), token.src.end())
            };
            lo = lo.min(s);
            hi = hi.max(e);
            found = true;
        }
        if found {
            return Span::new(lo, hi).expect("lo <= hi: both come from valid token spans");
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
