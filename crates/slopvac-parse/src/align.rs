//! Prose builder and text alignment.
//!
//! A parser hands over decoded text and the source range it came from. [`align`] walks the
//! two together and emits projection tokens: identity runs, entities, backslash escapes
//! and soft wraps. Source the decoded text does not contain is a gap.

use pulldown_cmark::{Event, Parser};
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

    /// The last non-empty projected text. Masked comments have none.
    fn last_text(&self) -> Option<&str> {
        self.tokens
            .iter()
            .rev()
            .map(|(_, text, _)| text.as_str())
            .find(|text| !text.is_empty())
    }

    /// A synthetic space after the last token. Skipped when no text came before it and
    /// after whitespace.
    pub(crate) fn wrap(&mut self) {
        if self
            .last_text()
            .is_none_or(|text| text.ends_with(char::is_whitespace))
        {
            return;
        }
        let at = self.tokens.last().map_or(0, |(_, _, src)| src.end());
        self.tokens
            .push((TokenKind::Wrap, " ".to_owned(), Span::empty(at)));
    }

    /// Whether any token has projected text.
    pub(crate) fn has_text(&self) -> bool {
        self.last_text().is_some()
    }

    /// Build the projection. Wraps with no text after them go. With `trim`, trailing
    /// whitespace of the last identity run goes too, which is what is left of a stripped
    /// heading attribute.
    pub(crate) fn finish(mut self, trim: bool) -> Option<Projection> {
        let tail = self
            .tokens
            .iter()
            .rposition(|(kind, text, _)| *kind != TokenKind::Wrap && !text.is_empty())
            .map_or(0, |i| i + 1);
        let mut index = 0;
        self.tokens.retain(|(kind, ..)| {
            index += 1;
            !(index > tail && *kind == TokenKind::Wrap)
        });
        if trim
            && let Some((TokenKind::Identity, text, src)) = self
                .tokens
                .iter_mut()
                .rev()
                .find(|(_, text, _)| !text.is_empty())
        {
            let kept = text.trim_end().len();
            if kept < text.len() {
                text.truncate(kept);
                *src = Span::new(src.start(), src.start() + kept).expect("kept <= len");
            }
        }
        self.tokens
            .retain(|(kind, text, _)| !(text.is_empty() && matches!(kind, TokenKind::Identity)));
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
        if matches!(vc, '\n' | '\r') {
            // A soft wrap, with any of the three line endings. Trailing blanks, the
            // terminator and the next line's quote marker and indentation are gaps.
            flush(&mut tokens, &mut run, i, j);
            j += if value[j..].starts_with("\r\n") { 2 } else { 1 };
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
            continue;
        }
        if sc == '&'
            && let Some(len) = entity_len(&src[i..])
            && let Some(decoded) = decode_entity(&src[i..i + len])
            && value[j..].starts_with(&decoded)
        {
            flush(&mut tokens, &mut run, i, j);
            let take = decoded.len();
            tokens.push(Raw {
                kind: TokenKind::Entity,
                text: decoded,
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

/// Longest named reference: 31 letters and digits. Numeric ones are shorter.
const MAX_REFERENCE: usize = 33;

/// Length in bytes of a character reference at the start of `s`, `;` included.
///
/// Looks at no more than `MAX_REFERENCE + 1` bytes, so a run of `&` costs constant time
/// per character.
fn entity_len(s: &str) -> Option<usize> {
    let body = s.strip_prefix('&')?.as_bytes();
    let mut end = None;
    for (k, &b) in body.iter().take(MAX_REFERENCE + 1).enumerate() {
        match b {
            b';' => {
                end = Some(k);
                break;
            }
            b'#' if k == 0 => {}
            _ if b.is_ascii_alphanumeric() => {}
            _ => return None,
        }
    }
    let name = &body[..end?];
    let valid = match name.strip_prefix(b"#") {
        Some(digits) => match digits
            .strip_prefix(b"x")
            .or_else(|| digits.strip_prefix(b"X"))
        {
            Some(hex) => (1..=6).contains(&hex.len()) && hex.iter().all(u8::is_ascii_hexdigit),
            None => (1..=7).contains(&digits.len()) && digits.iter().all(u8::is_ascii_digit),
        },
        None => (1..=31).contains(&name.len()) && name[0].is_ascii_alphabetic(),
    };
    valid.then_some(name.len() + 2)
}

/// What a character reference decodes to, or `None` when the parser leaves it as text.
///
/// pulldown-cmark owns the entity table, so ask it.
fn decode_entity(reference: &str) -> Option<String> {
    let decoded: String = Parser::new(reference)
        .filter_map(|event| match event {
            Event::Text(text) => Some(text.into_string()),
            _ => None,
        })
        .collect();
    (decoded != reference).then_some(decoded)
}
