//! A pure-Rust comment lexer that ports the 2.x projection.
//!
//! 2.x found comments in `_comment_projection` (`pipeline.py`): one pass over the file
//! that tracks block comments and strings, so a `//` inside a string does not start a
//! comment. This lexer keeps that algorithm and returns the byte extent of each comment
//! instead of a blanked copy of the file. The per-language tables live in
//! [`crate::language`].
//!
//! It runs once over the bytes and never recurses, so its cost is linear. It links no C.
//!
//! Deliberate differences from 2.x, each of which made 2.x skip real comments:
//!
//! - 2.x used `//` for some languages and `#` for the rest, with `/* */` everywhere.
//!   Here every language has its own markers, so Lua, Haskell, Clojure, PHP and the
//!   others are read.
//! - 2.x opened a string at any `'`, which blanked the rest of a Rust file after the
//!   first lifetime. A language marked `char_literal` opens one only for `'x'`.
//! - 2.x let `"` and `'` strings run over line ends. Here they end at the line unless a
//!   backslash escapes the terminator, or the language allows multi-line strings.
//! - Block comments nest in the languages that nest them.

use std::ops::Range;

use crate::language::{Block, Quote, Syntax};

/// The comments in `source` and the number of steps the scan took.
pub(crate) struct Scan {
    pub(crate) comments: Vec<Range<usize>>,
    /// The terminated `"` and `'` strings, from opening to closing delimiter, when the
    /// syntax asks for them (`Syntax::docstrings`). Empty otherwise.
    pub(crate) strings: Vec<Range<usize>>,
    pub(crate) steps: usize,
}

enum State {
    Code,
    Block {
        start: usize,
        open: &'static str,
        close: String,
        nested: bool,
        depth: usize,
        line_start: bool,
    },
    Quote {
        quote: Quote,
        start: usize,
    },
    /// A Lua long-bracket string, such as `[==[ ... ]==]`.
    Long {
        close: String,
    },
}

/// The scan used more than its step budget.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OverBudget {
    /// Steps taken when the scan stopped.
    pub(crate) steps: usize,
}

/// Scan `source` for comments. The scan stops with [`OverBudget`] as soon as it has taken
/// more than `max_steps` steps, so a small budget bounds both time and the comment list.
pub(crate) fn scan(source: &str, syntax: &Syntax, max_steps: usize) -> Result<Scan, OverBudget> {
    let bytes = source.as_bytes();
    let bom = if source.starts_with('\u{feff}') { 3 } else { 0 };
    let mut comments = Vec::new();
    let mut strings = Vec::new();
    let mut state = State::Code;
    let mut at = 0;
    let mut steps = 0;
    while at < bytes.len() {
        steps += 1;
        if steps > max_steps {
            return Err(OverBudget { steps });
        }
        let at_line_start = at == bom || at == 0 || matches!(bytes[at - 1], b'\n' | b'\r');
        state = match state {
            State::Code => {
                if let Some(next) = code(source, syntax, at, bom, at_line_start, &mut comments) {
                    at = next.0;
                    next.1
                } else {
                    at += 1;
                    State::Code
                }
            }
            State::Block {
                start,
                open,
                close,
                nested,
                mut depth,
                line_start,
            } => {
                let here = &bytes[at..];
                if here.starts_with(close.as_bytes()) && (!line_start || at_line_start) {
                    at += close.len();
                    depth -= 1;
                    if depth == 0 {
                        let end = if line_start { line_end(bytes, at) } else { at };
                        comments.push(start..end);
                        at = end;
                        State::Code
                    } else {
                        State::Block {
                            start,
                            open,
                            close,
                            nested,
                            depth,
                            line_start,
                        }
                    }
                } else if nested && here.starts_with(open.as_bytes()) {
                    at += open.len();
                    State::Block {
                        start,
                        open,
                        close,
                        nested,
                        depth: depth + 1,
                        line_start,
                    }
                } else {
                    at += 1;
                    State::Block {
                        start,
                        open,
                        close,
                        nested,
                        depth,
                        line_start,
                    }
                }
            }
            State::Quote { quote, start } => {
                let collect = syntax.docstrings.then_some(&mut strings);
                in_quote(bytes, &mut at, (quote, start), collect)
            }
            State::Long { close } => {
                if bytes[at..].starts_with(close.as_bytes()) {
                    at += close.len();
                    State::Code
                } else {
                    at += 1;
                    State::Long { close }
                }
            }
        };
    }
    if let State::Block { start, .. } = state {
        comments.push(start..bytes.len());
    }
    Ok(Scan {
        comments,
        strings,
        steps,
    })
}

/// One step inside a string that opened at `start`. Returns the state after it and moves
/// `at` forward. A string that closes is recorded in `strings`, when given.
fn in_quote(
    bytes: &[u8],
    at: &mut usize,
    (quote, start): (Quote, usize),
    strings: Option<&mut Vec<Range<usize>>>,
) -> State {
    let here = &bytes[*at..];
    if bytes[*at] == b'\\' {
        // A backslash takes the next byte, and a CRLF as one.
        *at += 2;
        if bytes.get(*at - 1) == Some(&b'\r') && bytes.get(*at) == Some(&b'\n') {
            *at += 1;
        }
        State::Quote { quote, start }
    } else if here.starts_with(quote.delim.as_bytes()) {
        *at += quote.delim.len();
        if let Some(strings) = strings {
            strings.push(start..*at);
        }
        State::Code
    } else if matches!(bytes[*at], b'\n' | b'\r') && !quote.multiline {
        *at += 1;
        State::Code
    } else {
        *at += 1;
        State::Quote { quote, start }
    }
}

/// Look for something that starts at `at` in code. Returns the next offset and state.
fn code(
    source: &str,
    syntax: &Syntax,
    at: usize,
    bom: usize,
    at_line_start: bool,
    comments: &mut Vec<Range<usize>>,
) -> Option<(usize, State)> {
    let bytes = source.as_bytes();
    let here = &bytes[at..];
    for block in syntax.blocks {
        let (open, close, nested, line_start) = match *block {
            Block::Fixed {
                open,
                close,
                nested,
                line_start,
                ..
            } => (open, close.to_owned(), nested, line_start),
            Block::Lua => match lua_open(here) {
                Some((open, close)) => (open, close, false, false),
                None => continue,
            },
        };
        if here.starts_with(open.as_bytes()) && (!line_start || at_line_start) {
            return Some((
                at + open.len(),
                State::Block {
                    start: at,
                    open,
                    close,
                    nested,
                    depth: 1,
                    line_start,
                },
            ));
        }
    }
    if syntax.backslash_char && here.first() == Some(&b'\\') {
        // One character, then any letters or digits (`\newline`, `\u0041`).
        let first = source.get(at + 1..)?.chars().next()?;
        let mut end = at + 1 + first.len_utf8();
        while bytes.get(end).is_some_and(u8::is_ascii_alphanumeric) {
            end += 1;
        }
        return Some((end, State::Code));
    }
    // A leading `#![` is a Rust inner attribute, not a shebang.
    let shebang = at == bom
        && here.get(2) != Some(&b'[')
        && syntax
            .shebang
            .iter()
            .any(|marker| here.starts_with(marker.as_bytes()));
    let marker = syntax
        .line
        .iter()
        .any(|marker| here.starts_with(marker.as_bytes()))
        || shebang;
    if marker {
        let end = comment_end(bytes, at, syntax.continuation);
        comments.push(at..end);
        return Some((end, State::Code));
    }
    if syntax.long_strings
        && let Some((len, close)) = long_bracket(here)
    {
        return Some((at + len, State::Long { close }));
    }
    for quote in syntax.strings {
        if !here.starts_with(quote.delim.as_bytes()) {
            continue;
        }
        if quote.char_literal {
            // The delimiter is ASCII, so `at` is a code point boundary.
            if let Some(len) = source.get(at..).and_then(char_literal) {
                return Some((at + len, State::Code));
            }
            continue;
        }
        return Some((
            at + quote.delim.len(),
            State::Quote {
                quote: *quote,
                start: at,
            },
        ));
    }
    None
}

/// The end of a line comment that starts at `from`. With `continuation`, a backslash
/// right before a line terminator carries the comment onto the next line.
fn comment_end(bytes: &[u8], from: usize, continuation: bool) -> usize {
    let mut end = line_end(bytes, from);
    while continuation && end > from && bytes[end - 1] == b'\\' && end < bytes.len() {
        let next = if bytes[end] == b'\r' && bytes.get(end + 1) == Some(&b'\n') {
            end + 2
        } else {
            end + 1
        };
        end = line_end(bytes, next);
    }
    end
}

/// The index of the line terminator at or after `from`, or the end of the source.
fn line_end(bytes: &[u8], from: usize) -> usize {
    bytes[from..]
        .iter()
        .position(|&byte| matches!(byte, b'\n' | b'\r'))
        .map_or(bytes.len(), |offset| from + offset)
}

/// The length of a character literal at the start of `text`, which begins with `'`.
fn char_literal(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(1) == Some(&b'\\') {
        // An escape such as `\n`, `\'` or `\u{1F600}`: skip the escaped character, then
        // the literal closes at the next `'` within 12 bytes. The search is over bytes, and
        // `'` is ASCII, so a multi-byte character in the window cannot hide it.
        let escaped = text.get(2..)?.chars().next()?;
        let start = 2 + escaped.len_utf8();
        let close = bytes
            .get(start..)?
            .iter()
            .take(12)
            .position(|&byte| byte == b'\'')?;
        return Some(start + close + 1);
    }
    let first = text.get(1..)?.chars().next()?;
    if matches!(first, '\'' | '\n' | '\r') {
        return None;
    }
    let end = 1 + first.len_utf8();
    (bytes.get(end) == Some(&b'\'')).then_some(end + 1)
}

/// A Lua long-bracket string opener and its closer: `[[` and `]]`, or `[==[` and `]==]`.
fn long_bracket(text: &[u8]) -> Option<(usize, String)> {
    let rest = text.strip_prefix(b"[")?;
    let level = rest.iter().take_while(|&&byte| byte == b'=').count();
    if rest.get(level) != Some(&b'[') {
        return None;
    }
    Some((level + 2, format!("]{}]", "=".repeat(level))))
}

/// A Lua long-bracket comment opener and its closer, such as `--[==[` and `]==]`.
fn lua_open(text: &[u8]) -> Option<(&'static str, String)> {
    let rest = text.strip_prefix(b"--[")?;
    let level = rest.iter().take_while(|&&byte| byte == b'=').count();
    if rest.get(level) != Some(&b'[') {
        return None;
    }
    // The opener varies with the level, so state keeps the closer and a fixed opener
    // that never matches a nested form: Lua comments do not nest.
    Some(("--[", format!("]{}]", "=".repeat(level))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Language;

    #[test]
    fn the_scan_stops_one_step_past_the_budget_on_a_large_input() {
        let source = "// word\n".repeat(500_000);
        let result = scan(&source, Language::Rust.syntax(), 100);
        assert_eq!(result.err(), Some(OverBudget { steps: 101 }));
    }

    #[test]
    fn the_scan_keeps_its_comments_within_the_budget() {
        let source = "// a\n// b\n// c\n";
        let scan = scan(source, Language::Rust.syntax(), 1_000).expect("within budget");
        assert_eq!(scan.comments.len(), 3);
    }
}
