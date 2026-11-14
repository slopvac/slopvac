//! Sentence splitting with abbreviation exceptions and vertical-list structure.
//! A port of the Python reference (`analyze.split_sentences`). Offsets are bytes.

use std::sync::LazyLock;

use pcre2::bytes::Regex;

use crate::classify::{TextType, classify_text_type};
use crate::lexicon::INITIALISM_SENTENCE_OPENERS;
use crate::pattern::{TextError, find_all, matches_start, re};
use crate::pyrules::{is_digit, is_space, trim, trim_start};
use crate::tokens::{
    CODE_SPAN, IDENTIFIER, PAREN_SPAN, QUOTED_SPAN, URL_OR_PATH, WORDLIKE, count_words,
};

/// One sentence of a block of text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sentence {
    /// Trimmed sentence text.
    pub text: String,
    /// Byte offset of the first byte in the input.
    pub start: usize,
    /// Byte offset after the last byte in the input.
    pub end: usize,
    /// Words per STE 8.4 to 8.7.
    pub word_count: usize,
    /// Procedural, descriptive or safety.
    pub text_type: TextType,
}

static LIST_MARKER: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:[-*+]|\d+[.)]|[A-Za-z][.)])\s+"));
static COLON_END: LazyLock<Regex> = LazyLock::new(|| re(r":\s*\z"));
static DOTTED_INITIALISM: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?<![A-Za-z.])(?:[A-Za-z]\.){2,}\z"));

static INITIALISM_OPENER: LazyLock<Regex> = LazyLock::new(|| re(r"^[A-Z][a-z]*\b"));

fn is_closer(c: char) -> bool {
    matches!(c, '"' | '\'' | '”' | '’' | ')' | ']')
}

fn protected_ranges(text: &str) -> Result<Vec<(usize, usize)>, TextError> {
    let mut ranges = Vec::new();
    for pattern in [
        &*CODE_SPAN,
        &*QUOTED_SPAN,
        &*PAREN_SPAN,
        &*URL_OR_PATH,
        &*IDENTIFIER,
    ] {
        ranges.extend(find_all(pattern, text)?);
    }
    ranges.sort_unstable();
    Ok(ranges)
}

/// Python's `re.match(r"([A-Z][a-z]*)\b", text)` against the closed-class opener list.
fn is_initialism_opener(text: &str) -> Result<bool, TextError> {
    Ok(find_all(&INITIALISM_OPENER, text)?
        .first()
        .is_some_and(|&(start, end)| INITIALISM_SENTENCE_OPENERS.contains(&&text[start..end])))
}

fn terminal_cut(
    text: &str,
    index: usize,
    protected: &[(usize, usize)],
) -> Result<Option<usize>, TextError> {
    let mut protected_end = None;
    if let Some(&(_, end)) = protected.iter().find(|&&(s, e)| s <= index && index < e) {
        if !text[index + 1..end].trim_matches(is_closer).is_empty() {
            return Ok(None);
        }
        protected_end = Some(end);
    }
    if text.as_bytes()[index] == b'.' && DOTTED_INITIALISM.is_match(&text.as_bytes()[..=index])? {
        let rest = &text[index + 1..];
        let remainder = trim_start(rest);
        let gap = &rest[..rest.len() - remainder.len()];
        if !remainder.is_empty()
            && gap.chars().count() < 2
            && (gap != " " || !is_initialism_opener(remainder)?)
        {
            return Ok(None);
        }
    }
    let mut end = protected_end.unwrap_or(index + 1);
    while let Some(c) = text[end..].chars().next() {
        if !is_closer(c) {
            break;
        }
        end += c.len_utf8();
    }
    if text[end..].chars().next().is_some_and(|c| !is_space(c)) {
        return Ok(None);
    }
    Ok(match trim_start(&text[end..]).chars().next() {
        None => Some(end),
        Some(c) if c.is_uppercase() || is_digit(c) => Some(end),
        Some(_) => None,
    })
}

/// Python's `str.splitlines(keepends=True)` line breaks.
fn split_lines(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut iter = text.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        let mut end = None;
        match c {
            '\r' => {
                end = Some(if iter.peek().is_some_and(|&(_, n)| n == '\n') {
                    iter.next();
                    i + 2
                } else {
                    i + 1
                });
            }
            '\n' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}'
            | '\u{2029}' => end = Some(i + c.len_utf8()),
            _ => {}
        }
        if let Some(e) = end {
            out.push((start, &text[start..e]));
            start = e;
        }
    }
    if start < text.len() {
        out.push((start, &text[start..]));
    }
    out
}

/// Split a lead-in and its list items, keeping each item's source offset.
fn split_vertical_list(text: &str) -> Result<Vec<(&str, usize)>, TextError> {
    let lines = split_lines(text);
    if lines.len() < 2 {
        return Ok(vec![(text, 0)]);
    }
    for (index, (_, line)) in lines.iter().enumerate() {
        let body = line.trim_end_matches(['\r', '\n']);
        if !COLON_END.is_match(body.as_bytes())? {
            continue;
        }
        let mut tail: Vec<(usize, &str)> = Vec::new();
        for (k, (_, l)) in lines[index + 1..].iter().enumerate() {
            let part = trim(l);
            if !part.is_empty() {
                tail.push((index + 1 + k, part));
            }
        }
        let mut all_items = !tail.is_empty();
        for (_, part) in &tail {
            if !matches_start(&LIST_MARKER, part)? {
                all_items = false;
                break;
            }
        }
        if !all_items {
            continue;
        }
        let head_end = lines[index].0 + lines[index].1.len();
        let head_all = &text[..head_end];
        // Divergence from the Python reference, which reports offset 0 here even when
        // the text starts with whitespace. See tests/fixtures/divergences.md.
        let lead = head_all.len() - trim_start(head_all).len();
        let mut result = vec![(trim(head_all), lead)];
        for (line_index, part) in tail {
            let (offset, raw) = lines[line_index];
            let leading = raw.len() - trim_start(raw).len();
            result.push((part, offset + leading));
        }
        return Ok(result);
    }
    Ok(vec![(text, 0)])
}

/// Split prose into sentences, applying vertical-list structure before punctuation.
pub fn split_sentences(text: &str) -> Result<Vec<Sentence>, TextError> {
    if trim(text).is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for (chunk, chunk_offset) in split_vertical_list(text)? {
        let protected = protected_ranges(chunk)?;
        let mut cuts = Vec::new();
        let mut index = 0;
        while index < chunk.len() {
            let c = chunk[index..]
                .chars()
                .next()
                .expect("index on a char boundary");
            if matches!(c, '.' | '!' | '?')
                && let Some(end) = terminal_cut(chunk, index, &protected)?
            {
                cuts.push(end);
                index = end;
                continue;
            }
            index += c.len_utf8();
        }
        let mut start = 0;
        cuts.push(chunk.len());
        for end in cuts {
            let raw = &chunk[start..end];
            let part = trim(raw);
            let leading = raw.len() - trim_start(raw).len();
            let absolute = chunk_offset + start + leading;
            start = end;
            if part.is_empty() || !WORDLIKE.is_match(part.as_bytes())? {
                continue;
            }
            out.push(Sentence {
                text: part.to_owned(),
                start: absolute,
                end: absolute + part.len(),
                word_count: count_words(part)?,
                text_type: classify_text_type(part)?,
            });
        }
    }
    Ok(out)
}
