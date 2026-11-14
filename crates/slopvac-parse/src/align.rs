//! Prose builder and text alignment.
//!
//! A parser hands over decoded text and the source range it came from. [`align`] walks the
//! two together and emits projection tokens: identity runs, entities, backslash escapes
//! and soft wraps. Source the decoded text does not contain is a gap.

use slopvac_model::Span;

use crate::mask::Masked;
use crate::{Projection, TokenKind};

/// Collects tokens for one prose block.
#[derive(Debug, Default)]
pub(crate) struct ProseBuilder {
    tokens: Vec<(TokenKind, String, Span)>,
}

impl ProseBuilder {
    pub(crate) fn push(&mut self, kind: TokenKind, text: &str, src: Span) {
        self.tokens.push((kind, text.to_owned(), src));
    }

    /// A synthetic space after the last token. Skipped at the start and after whitespace.
    pub(crate) fn wrap(&mut self) {
        let Some((_, text, src)) = self.tokens.last() else {
            return;
        };
        if text.is_empty() || text.ends_with(char::is_whitespace) {
            return;
        }
        let at = Span::empty(src.end());
        self.tokens.push((TokenKind::Wrap, " ".to_owned(), at));
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Build the projection. Trailing wraps go. With `trim`, trailing whitespace of the
    /// last identity run goes too, which is what is left of a stripped heading attribute.
    pub(crate) fn finish(mut self, trim: bool) -> Option<Projection> {
        while self
            .tokens
            .last()
            .is_some_and(|(kind, ..)| *kind == TokenKind::Wrap)
        {
            self.tokens.pop();
        }
        if trim && let Some((TokenKind::Identity, text, src)) = self.tokens.last_mut() {
            let kept = text.trim_end().len();
            if kept < text.len() {
                text.truncate(kept);
                *src = Span::new(src.start(), src.start() + kept).expect("kept <= len");
            }
            if text.is_empty() {
                self.tokens.pop();
            }
        }
        if self.tokens.is_empty() {
            return None;
        }
        let mut projection = Projection::new();
        for (kind, text, src) in &self.tokens {
            projection.push(*kind, text, *src);
        }
        Some(projection)
    }
}

/// One token in masked coordinates. `Wrap` carries no range.
struct Raw {
    kind: TokenKind,
    text: String,
    start: usize,
    end: usize,
}

/// Align `value`, the decoded text of the node at masked range `node`, with `masked`.
///
/// When the two do not line up, the whole node becomes one opaque token, which still
/// maps every match back to the node.
pub(crate) fn align(
    builder: &mut ProseBuilder,
    coords: &Masked,
    masked: &str,
    node: (usize, usize),
    value: &str,
) {
    let (node_start, node_end) = node;
    let tokens = walk(&masked[node_start..node_end], node_start, value).unwrap_or_else(|| {
        vec![Raw {
            kind: TokenKind::Opaque,
            text: value.to_owned(),
            start: node_start,
            end: node_end,
        }]
    });
    for token in tokens {
        if token.kind == TokenKind::Wrap {
            builder.wrap();
        } else {
            let span = coords.span(token.start, token.end);
            builder.push(token.kind, &token.text, span);
        }
    }
}

fn walk(src: &str, base: usize, value: &str) -> Option<Vec<Raw>> {
    let mut tokens: Vec<Raw> = Vec::new();
    let (mut i, mut j) = (0, 0);
    // Start of the open identity run, in `src` and `value`.
    let mut run: Option<(usize, usize)> = None;
    let flush = |tokens: &mut Vec<Raw>, run: &mut Option<(usize, usize)>, i: usize, j: usize| {
        if let Some((si, sj)) = run.take() {
            tokens.push(Raw {
                kind: TokenKind::Identity,
                text: value[sj..j].to_owned(),
                start: base + si,
                end: base + i,
            });
        }
    };
    while j < value.len() {
        let vc = value[j..].chars().next()?;
        let sc = src[i..].chars().next()?;
        if vc == '\n' && matches!(sc, ' ' | '\t' | '\n' | '\r') {
            // A soft wrap. Trailing blanks, the terminator and the next line's quote
            // marker and indentation are gaps.
            flush(&mut tokens, &mut run, i, j);
            while src[i..].starts_with([' ', '\t']) {
                i += 1;
            }
            i += if src[i..].starts_with("\r\n") {
                2
            } else if src[i..].starts_with(['\n', '\r']) {
                1
            } else {
                return None;
            };
            while src[i..].starts_with([' ', '\t', '>']) {
                i += 1;
            }
            tokens.push(Raw {
                kind: TokenKind::Wrap,
                text: " ".to_owned(),
                start: 0,
                end: 0,
            });
            j += 1;
            continue;
        }
        if sc == '&'
            && let Some(len) = entity_len(&src[i..])
            && !value[j..].starts_with(&src[i..i + len])
        {
            flush(&mut tokens, &mut run, i, j);
            let after = src[i + len..].chars().next();
            let take = decoded_len(&value[j..], after);
            tokens.push(Raw {
                kind: TokenKind::Entity,
                text: value[j..j + take].to_owned(),
                start: base + i,
                end: base + i + len,
            });
            i += len;
            j += take;
            continue;
        }
        if sc == '\\'
            && let Some(next) = src[i + 1..].chars().next()
            && next.is_ascii_punctuation()
            && next == vc
        {
            flush(&mut tokens, &mut run, i, j);
            let len = 1 + next.len_utf8();
            tokens.push(Raw {
                kind: TokenKind::Backslash,
                text: next.to_string(),
                start: base + i,
                end: base + i + len,
            });
            i += len;
            j += next.len_utf8();
            continue;
        }
        if sc == vc {
            run.get_or_insert((i, j));
            i += sc.len_utf8();
            j += vc.len_utf8();
            continue;
        }
        // Markup around the text, such as the angle brackets of an autolink.
        if matches!(sc, '<' | '>') {
            flush(&mut tokens, &mut run, i, j);
            i += 1;
            continue;
        }
        return None;
    }
    flush(&mut tokens, &mut run, i, j);
    src[i..]
        .chars()
        .all(|c| c.is_whitespace() || c == '>')
        .then_some(tokens)
}

/// Length in bytes of a character reference at the start of `s`, `;` included.
fn entity_len(s: &str) -> Option<usize> {
    let body = s.strip_prefix('&')?;
    let end = body.find(';')?;
    let name = &body[..end];
    let valid = match name.strip_prefix('#') {
        Some(digits) => match digits.strip_prefix(['x', 'X']) {
            Some(hex) => (1..=6).contains(&hex.len()) && hex.bytes().all(|b| b.is_ascii_hexdigit()),
            None => (1..=7).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit()),
        },
        None => {
            (1..=32).contains(&name.len())
                && name.starts_with(|c: char| c.is_ascii_alphabetic())
                && name.bytes().all(|b| b.is_ascii_alphanumeric())
        }
    };
    valid.then_some(end + 2)
}

/// Bytes of `value` that one entity decoded to: one or two code points. `after` is the
/// source character that follows the entity.
fn decoded_len(value: &str, after: Option<char>) -> usize {
    let mut chars = value.char_indices();
    let first_end = chars.next().map_or(0, |(i, c)| i + c.len_utf8());
    let second_end = chars.next().map(|(i, c)| i + c.len_utf8());
    let continues = |at: usize| value[at..].chars().next() == after;
    match second_end {
        Some(two) if !continues(first_end) && continues(two) => two,
        _ => first_end,
    }
}
