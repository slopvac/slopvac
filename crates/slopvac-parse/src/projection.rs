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
    /// Indexes of the tokens that bridge a gap in the source, in order.
    bridges: Vec<usize>,
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

    /// Append a token whose source range may lie before the previous token's.
    ///
    /// A document that reads in an order other than the source order needs this: the cells of
    /// a table whose left cell wraps over several lines read cell by cell, so the right cell
    /// starts in the source before the left cell ends. [`Self::span_for_range`] maps a
    /// range over such tokens to the hull of their source ranges: the smallest start and the
    /// largest end, so the span never inverts.
    pub fn push_unordered(&mut self, kind: TokenKind, text: &str, src: Span) {
        self.text.push_str(text);
        self.tokens.push(Token {
            kind,
            text: text.to_owned(),
            src,
        });
    }

    /// Append a token that bridges source text that is not part of this projection, such as
    /// the whitespace between two lines of a table cell, which covers the cells beside it.
    /// The token joins the text like any other, but it belongs to no extent.
    pub fn push_bridge(&mut self, kind: TokenKind, text: &str, src: Span) {
        self.bridges.push(self.tokens.len());
        self.push_unordered(kind, text, src);
    }

    /// The source ranges the text sits on: the hull of each run of tokens between bridges.
    /// Without bridges that is one range, the hull of all real tokens, or none when there
    /// is no real token. A caller that turns a block into a source range for tags or
    /// exclusions uses these, so that text between the ranges is not taken for the block's.
    #[must_use]
    pub fn extents(&self) -> Vec<Span> {
        let mut extents = Vec::new();
        let mut current: Option<(usize, usize)> = None;
        let mut next_bridge = self.bridges.iter().peekable();
        for (i, token) in self.tokens.iter().enumerate() {
            if next_bridge.next_if(|&&b| b == i).is_some() {
                extents.extend(current.take());
                continue;
            }
            if !token.kind.is_real() {
                continue;
            }
            let (start, end) = (token.src.start(), token.src.end());
            current = Some(current.map_or((start, end), |(a, b)| (a.min(start), b.max(end))));
        }
        extents.extend(current);
        extents
            .into_iter()
            .map(|(a, b)| Span::new(a, b).expect("start <= end"))
            .collect()
    }

    /// Whether token `index` bridges source text that is not part of the projection.
    #[must_use]
    pub fn is_bridge(&self, index: usize) -> bool {
        self.bridges.binary_search(&index).is_ok()
    }

    /// Like [`Self::extents`], for the tokens that overlap the projected bytes `start..end`:
    /// the source ranges a match over those bytes is made of. A match over one run of
    /// ordinary text has one range, its hull. A match across the line break of a wrapped table
    /// cell has one range for each line, and not the cells in between.
    #[must_use]
    pub fn extents_in_bytes(&self, start: usize, end: usize) -> Vec<Span> {
        let mut extents = Vec::new();
        let mut current: Option<(usize, usize)> = None;
        let mut at = 0;
        for (i, token) in self.tokens.iter().enumerate() {
            let (from, to) = (at, at + token.text.len());
            at = to;
            if from >= end {
                break;
            }
            if to <= start {
                continue;
            }
            if self.is_bridge(i) {
                extents.extend(current.take());
                continue;
            }
            if !token.kind.is_real() {
                continue;
            }
            let (a, b) = (token.src.start(), token.src.end());
            current = Some(current.map_or((a, b), |(x, y)| (x.min(a), y.max(b))));
        }
        extents.extend(current);
        extents
            .into_iter()
            .map(|(a, b)| Span::new(a, b).expect("start <= end"))
            .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn at(a: usize, b: usize) -> Span {
        Span::new(a, b).unwrap()
    }

    #[test]
    fn extents_of_ordered_text_are_the_one_hull() {
        let mut p = Projection::new();
        p.push(TokenKind::Identity, "ab", at(2, 4));
        p.push(TokenKind::Collapse, " ", at(4, 5));
        p.push(TokenKind::Identity, "cd", at(5, 7));
        assert_eq!(p.extents(), [at(2, 7)]);
    }

    #[test]
    fn a_bridge_token_ends_an_extent_and_belongs_to_none() {
        // A cell on two lines: its second line starts after the text of the cell beside it.
        let mut p = Projection::new();
        p.push(TokenKind::Identity, "ab", at(2, 4));
        p.push_bridge(TokenKind::Collapse, " ", at(4, 30));
        p.push_unordered(TokenKind::Identity, "cd", at(30, 32));
        assert_eq!(p.extents(), [at(2, 4), at(30, 32)]);
    }

    #[test]
    fn a_match_across_a_bridge_has_one_range_for_each_side() {
        let mut p = Projection::new();
        p.push(TokenKind::Identity, "ab", at(2, 4));
        p.push_bridge(TokenKind::Collapse, " ", at(4, 30));
        p.push_unordered(TokenKind::Identity, "cd", at(30, 32));
        // "b c": the last byte of the first word, the bridge, the first byte of the second.
        assert_eq!(p.extents_in_bytes(1, 4), [at(2, 4), at(30, 32)]);
        // Inside one word the range is exact to the token.
        assert_eq!(p.extents_in_bytes(3, 5), [at(30, 32)]);
        assert_eq!(p.extents_in_bytes(0, 2), [at(2, 4)]);
    }

    #[test]
    fn a_projection_with_no_real_token_has_no_extent() {
        let mut p = Projection::new();
        p.push(TokenKind::Wrap, " ", Span::empty(3));
        assert_eq!(p.extents(), []);
    }
}
