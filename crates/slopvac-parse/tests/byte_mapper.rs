#![allow(missing_docs)]
//! `ByteMapper::span` returns the span `Projection::span_for_bytes` returns, for every range
//! of every projection. The oracle below is the code of `span_for_bytes` and
//! `span_for_range` as it was before the mapper existed.

use proptest::prelude::*;
use slopvac_model::Span;
use slopvac_parse::{ByteMapper, Projection, TokenKind};

fn char_byte(text: &str, n: usize) -> usize {
    text.char_indices().nth(n).map_or(text.len(), |(i, _)| i)
}

fn oracle_range(p: &Projection, start: usize, end: usize) -> Span {
    let mut lo = usize::MAX;
    let mut hi = 0;
    let mut found = false;
    let mut point_at_start = None;
    let mut cursor = 0;
    for token in p.tokens() {
        let len = token.text.chars().count();
        let (t0, t1) = (cursor, cursor + len);
        cursor = t1;
        if start >= t0 && start < t1 && point_at_start.is_none() {
            point_at_start = Some(match token.kind {
                TokenKind::Identity => token.src.start() + char_byte(&token.text, start - t0),
                _ => token.src.start(),
            });
        }
        let (from, to) = (start.max(t0), end.min(t1));
        if from >= to || !token.kind.is_real() {
            continue;
        }
        let (first, last) = if token.kind == TokenKind::Identity {
            (
                token.src.start() + char_byte(&token.text, from - t0),
                token.src.start() + char_byte(&token.text, to - t0),
            )
        } else {
            (token.src.start(), token.src.end())
        };
        lo = lo.min(first);
        hi = hi.max(last);
        found = true;
    }
    if found {
        return Span::new(lo, hi).unwrap();
    }
    let at = point_at_start
        .or_else(|| p.tokens().last().map(|t| t.src.end()))
        .unwrap_or(0);
    Span::empty(at)
}

fn oracle(p: &Projection, start: usize, end: usize) -> Span {
    let cp = |b: usize| p.text()[..b].chars().count();
    oracle_range(p, cp(start), cp(end))
}

fn kind() -> impl Strategy<Value = TokenKind> {
    prop_oneof![
        Just(TokenKind::Identity),
        Just(TokenKind::Entity),
        Just(TokenKind::Backslash),
        Just(TokenKind::Opaque),
        Just(TokenKind::Collapse),
        Just(TokenKind::Masked),
        Just(TokenKind::Wrap),
    ]
}

/// Tokens with empty, ASCII and multibyte text, and source ranges in any order.
fn projection() -> impl Strategy<Value = Projection> {
    let token = (
        kind(),
        prop_oneof![
            Just(""),
            Just("a"),
            Just("ab "),
            Just("é"),
            Just("😀x"),
            Just("über")
        ],
        0usize..40,
        0usize..8,
    );
    prop::collection::vec(token, 0..12).prop_map(|tokens| {
        let mut p = Projection::new();
        for (kind, text, start, len) in tokens {
            p.push_unordered(kind, text, Span::new(start, start + len).unwrap());
        }
        p
    })
}

/// A projection and a range of its text with `start <= end`, both on character boundaries.
fn case() -> impl Strategy<Value = (Projection, usize, usize)> {
    projection().prop_flat_map(|p| {
        let bounds: Vec<usize> = p
            .text()
            .char_indices()
            .map(|(i, _)| i)
            .chain([p.text().len()])
            .collect();
        let n = bounds.len();
        (Just(p), Just(bounds), 0..n, 0..n).prop_map(|(p, bounds, a, b)| {
            let (a, b) = (bounds[a.min(b)], bounds[a.max(b)]);
            (p, a, b)
        })
    })
}

proptest! {
    #[test]
    fn the_mapper_returns_the_span_of_the_code_it_replaces((p, start, end) in case()) {
        prop_assert_eq!(ByteMapper::new(&p).span(start, end), Some(oracle(&p, start, end)));
    }
}

#[test]
fn a_range_that_is_not_on_the_text_maps_to_nothing() {
    let mut p = Projection::new();
    p.push(TokenKind::Identity, "é.", Span::new(0, 3).unwrap());
    let mapper = ByteMapper::new(&p);
    // Byte 1 is inside "é", and byte 4 is past the end of the 3-byte text.
    assert_eq!(mapper.span(1, 3), None);
    assert_eq!(mapper.span(0, 4), None);
    assert_eq!(mapper.span(0, 3), Some(Span::new(0, 3).unwrap()));
}
