//! Sentence splitting with abbreviation exceptions and vertical-list structure.
//! A port of the Python reference (`analyze.split_sentences`). Offsets are bytes.

use std::sync::LazyLock;

use slopvac_pcre2::Regex;

use crate::classify::{TextType, classify_text_type};
use crate::lexicon::INITIALISM_SENTENCE_OPENERS;
use crate::pattern::{TextError, find_all, matches_start, re};
use crate::pyrules::{is_digit, is_upper, py_isspace, py_lstrip, py_strip, regex_space};
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

/// A table cell as one sentence, unsplit, or none when it has no word-like text.
///
/// The reference makes one sentence of each cell, whatever punctuation it holds, so a cell
/// of two sentences counts as one sentence of the words of both. `start` and `end` cover
/// the trimmed text.
pub fn cell_sentence(text: &str) -> Result<Option<Sentence>, TextError> {
    let part = py_strip(text);
    if part.is_empty() || !WORDLIKE.is_match(part)? {
        return Ok(None);
    }
    let leading = text.len() - py_lstrip(text).len();
    counted::built();
    Ok(Some(Sentence {
        text: part.to_owned(),
        start: leading,
        end: leading + part.len(),
        word_count: count_words(part)?,
        text_type: classify_text_type(part)?,
    }))
}

static LIST_MARKER: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:[-*+]|\d+[.)]|[A-Za-z][.)])\s+"));
static COLON_END: LazyLock<Regex> = LazyLock::new(|| re(r":\s*\z"));
/// The reference tests `(?<![A-Za-z.])(?:[A-Za-z]\.){2,}\z` on the prefix ending at each
/// full stop, and a pattern scan from the start of the prefix makes the split quadratic.
/// `runs[i]` is the number of `X.` pairs that end at byte `i`; only a maximal run can pass
/// the lookbehind, because a shorter start follows a full stop.
fn dotted_runs(bytes: &[u8]) -> Vec<usize> {
    let mut runs = vec![0; bytes.len()];
    for i in 1..bytes.len() {
        if bytes[i] == b'.' && bytes[i - 1].is_ascii_alphabetic() {
            runs[i] = 1 + if i >= 2 { runs[i - 2] } else { 0 };
        }
    }
    runs
}

/// Whether the prefix `bytes[..=index]` ends in a dotted initialism such as `e.g.`.
fn ends_in_dotted_initialism(bytes: &[u8], runs: &[usize], index: usize, work: &mut Work) -> bool {
    work.tick(1);
    let pairs = runs[index];
    if pairs < 2 {
        return false;
    }
    let start = index + 1 - 2 * pairs;
    work.tick(1);
    start == 0 || !(bytes[start - 1].is_ascii_alphabetic() || bytes[start - 1] == b'.')
}

/// Protected spans sorted by start, with a running maximum of their ends so that the first
/// span containing a byte is a binary search instead of a scan.
struct Protected {
    ranges: Vec<(usize, usize)>,
    max_end: Vec<usize>,
}

impl Protected {
    fn new(ranges: Vec<(usize, usize)>) -> Self {
        let mut top = 0;
        let max_end = ranges
            .iter()
            .map(|&(_, e)| {
                top = top.max(e);
                top
            })
            .collect();
        Self { ranges, max_end }
    }

    /// End of the first span (in start order) with `start <= index < end`.
    fn end_containing(&self, index: usize, work: &mut Work) -> Option<usize> {
        let starts = self.ranges.partition_point(|&(s, _)| {
            work.tick(1);
            s <= index
        });
        let first = self.max_end.partition_point(|&e| {
            work.tick(1);
            e <= index
        });
        (first < starts).then(|| self.ranges[first].1)
    }
}

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

/// Per-chunk lookup tables, built in one pass each.
struct Scan {
    protected: Protected,
    runs: Vec<usize>,
}

fn terminal_cut(
    text: &str,
    index: usize,
    scan: &Scan,
    work: &mut Work,
) -> Result<Option<usize>, TextError> {
    let mut protected_end = None;
    if let Some(end) = scan.protected.end_containing(index, work) {
        if !text[index + 1..end].trim_matches(is_closer).is_empty() {
            return Ok(None);
        }
        protected_end = Some(end);
    }
    if text.as_bytes()[index] == b'.'
        && ends_in_dotted_initialism(text.as_bytes(), &scan.runs, index, work)
    {
        // Python measures this gap with `re.match(r"\s*", ...)`: regex whitespace.
        let rest = &text[index + 1..];
        let gap_len = rest.find(|c| !regex_space(c)).unwrap_or(rest.len());
        let (gap, remainder) = rest.split_at(gap_len);
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
    if text[end..].chars().next().is_some_and(|c| !py_isspace(c)) {
        return Ok(None);
    }
    Ok(match py_lstrip(&text[end..]).chars().next() {
        None => Some(end),
        Some(c) if is_upper(c) || is_digit(c) => Some(end),
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

/// For each line index, whether the non-empty lines after it exist and all start with a
/// list marker.
struct ListItemSuffixes {
    /// `(any non-empty line from here on, all of them are list items)` per line index.
    from: Vec<(bool, bool)>,
}

impl ListItemSuffixes {
    fn all_items_after(&self, index: usize) -> bool {
        let (any, all) = self.from[index + 1];
        any && all
    }
}

fn list_item_suffixes(
    lines: &[(usize, &str)],
    work: &mut Work,
) -> Result<ListItemSuffixes, TextError> {
    let mut from = vec![(false, true); lines.len() + 1];
    for (k, (_, line)) in lines.iter().enumerate().rev() {
        work.tick(1);
        let part = py_strip(line);
        from[k] = if part.is_empty() {
            from[k + 1]
        } else {
            (true, from[k + 1].1 && matches_start(&LIST_MARKER, part)?)
        };
    }
    Ok(ListItemSuffixes { from })
}

/// Split a lead-in and its list items, keeping each item's source offset.
fn split_vertical_list<'a>(
    text: &'a str,
    work: &mut Work,
) -> Result<Vec<(&'a str, usize)>, TextError> {
    let lines = split_lines(text);
    if lines.len() < 2 {
        return Ok(vec![(text, 0)]);
    }
    let mut items = None;
    for (index, (_, line)) in lines.iter().enumerate() {
        let body = line.trim_end_matches(['\r', '\n']);
        if !COLON_END.is_match(body)? {
            continue;
        }
        // One backward pass answers "are all non-empty lines from here on list items".
        let items = match &items {
            Some(items) => items,
            None => items.insert(list_item_suffixes(&lines, work)?),
        };
        if !items.all_items_after(index) {
            continue;
        }
        let mut tail: Vec<(usize, &str)> = Vec::new();
        for (k, (_, l)) in lines[index + 1..].iter().enumerate() {
            work.tick(1);
            let part = py_strip(l);
            if !part.is_empty() {
                tail.push((index + 1 + k, part));
            }
        }
        let head_end = lines[index].0 + lines[index].1.len();
        let head_all = &text[..head_end];
        // Divergence from the Python reference, which reports offset 0 here even when
        // the text starts with whitespace. See tests/fixtures/divergences.md.
        let lead = head_all.len() - py_lstrip(head_all).len();
        let mut result = vec![(py_strip(head_all), lead)];
        for (line_index, part) in tail {
            let (offset, raw) = lines[line_index];
            let leading = raw.len() - py_lstrip(raw).len();
            result.push((part, offset + leading));
        }
        return Ok(result);
    }
    Ok(vec![(text, 0)])
}

/// Sentence records built on this thread, for the tests that bound allocation.
#[cfg(feature = "work-counters")]
mod counted {
    use std::cell::Cell;

    thread_local! {
        static BUILT: Cell<usize> = const { Cell::new(0) };
        static CUTS: Cell<usize> = const { Cell::new(0) };
        static HELD: Cell<usize> = const { Cell::new(0) };
    }

    pub(super) fn cut() {
        CUTS.with(|c| c.set(c.get() + 1));
    }

    pub(super) fn take_cuts() -> usize {
        CUTS.with(|c| c.replace(0))
    }

    pub(super) fn held(n: usize) {
        HELD.with(|c| c.set(c.get().max(n)));
    }

    pub(super) fn take_held() -> usize {
        HELD.with(|c| c.replace(0))
    }

    pub(super) fn built() {
        BUILT.with(|c| c.set(c.get() + 1));
    }

    pub(super) fn take() -> usize {
        BUILT.with(|c| c.replace(0))
    }
}

#[cfg(not(feature = "work-counters"))]
mod counted {
    #[inline(always)]
    pub(super) fn built() {}

    #[inline(always)]
    pub(super) fn cut() {}

    #[inline(always)]
    pub(super) fn held(_n: usize) {}
}

/// The sentence records [`split_sentences`] and [`split_sentences_within`] built on this
/// thread since the last call. A test that lints a document of very short sentences and
/// reads the count shows whether the split stops at its limit or builds every sentence first.
#[cfg(feature = "work-counters")]
#[doc(hidden)]
#[must_use]
pub fn take_sentences_built() -> usize {
    counted::take()
}

/// The sentence boundaries the splitter found on this thread since the last call. A split
/// that stops at its limit finds about that many, and one that finds every boundary before it
/// checks the limit finds them all.
#[cfg(feature = "work-counters")]
#[doc(hidden)]
#[must_use]
pub fn take_cuts_found() -> usize {
    counted::take_cuts()
}

/// The most sentence records one split held at once on this thread since the last call.
#[cfg(feature = "work-counters")]
#[doc(hidden)]
#[must_use]
pub fn take_peak_sentences_held() -> usize {
    counted::take_held()
}

/// Split prose into sentences, applying vertical-list structure before punctuation.
pub fn split_sentences(text: &str) -> Result<Vec<Sentence>, TextError> {
    split_sentences_counted(text, &mut Work::default())
}

/// Split prose as [`split_sentences`] does, but stop as soon as more than `max` sentences
/// are found, so the result never holds more than `max`. Returns `None` in that case.
///
/// # Errors
/// Returns a [`TextError`] when a pattern fails.
pub fn split_sentences_within(text: &str, max: usize) -> Result<Option<Vec<Sentence>>, TextError> {
    split_limited(text, &mut Work::default(), max)
}

/// Units of scanning the splitter does outside the single pass each pattern makes over a
/// chunk. One unit is a byte handed to a matcher, a span or table entry visited by a
/// lookup, or a line visited while building a list tail. Tests bound it by the input
/// length, so a hot path that scans per terminator or per line shows up as a large count.
#[derive(Debug, Default)]
struct Work(usize);

impl Work {
    fn tick(&mut self, units: usize) {
        self.0 += units;
    }
}

fn split_sentences_counted(text: &str, work: &mut Work) -> Result<Vec<Sentence>, TextError> {
    Ok(split_limited(text, work, usize::MAX)?.unwrap_or_default())
}

/// The sentence in `start..end` of `chunk`, which starts at `chunk_offset` in the text, or
/// `None` when it holds no word.
fn sentence_in(
    chunk: &str,
    chunk_offset: usize,
    start: usize,
    end: usize,
) -> Result<Option<Sentence>, TextError> {
    let raw = chunk.get(start..end).unwrap_or_default();
    let part = py_strip(raw);
    let leading = raw.len() - py_lstrip(raw).len();
    let absolute = chunk_offset + start + leading;
    if part.is_empty() || !WORDLIKE.is_match(part)? {
        return Ok(None);
    }
    counted::built();
    Ok(Some(Sentence {
        text: part.to_owned(),
        start: absolute,
        end: absolute + part.len(),
        word_count: count_words(part)?,
        text_type: classify_text_type(part)?,
    }))
}

/// Hand each sentence of `text` to `visit` as the splitter finds it, and stop when `visit`
/// returns `false`. Nothing is held between sentences, so the caller decides what to keep.
fn each_sentence(
    text: &str,
    work: &mut Work,
    mut visit: impl FnMut(Sentence) -> bool,
) -> Result<(), TextError> {
    if py_strip(text).is_empty() {
        return Ok(());
    }
    for (chunk, chunk_offset) in split_vertical_list(text, work)? {
        work.tick(chunk.len());
        let scan = Scan {
            protected: Protected::new(protected_ranges(chunk)?),
            runs: dotted_runs(chunk.as_bytes()),
        };
        let mut start = 0;
        let mut index = 0;
        while index < chunk.len() {
            let c = chunk[index..]
                .chars()
                .next()
                .expect("index on a char boundary");
            if matches!(c, '.' | '!' | '?')
                && let Some(end) = terminal_cut(chunk, index, &scan, work)?
            {
                counted::cut();
                if let Some(found) = sentence_in(chunk, chunk_offset, start, end)?
                    && !visit(found)
                {
                    return Ok(());
                }
                start = end;
                index = end;
                continue;
            }
            index += c.len_utf8();
        }
        if let Some(found) = sentence_in(chunk, chunk_offset, start, chunk.len())?
            && !visit(found)
        {
            return Ok(());
        }
    }
    Ok(())
}

/// The sentences of `text`, or `None` when there are more than `max`.
fn split_limited(
    text: &str,
    work: &mut Work,
    max: usize,
) -> Result<Option<Vec<Sentence>>, TextError> {
    let mut out = Vec::new();
    let mut over = false;
    each_sentence(text, work, |found| {
        if out.len() >= max {
            over = true;
            return false;
        }
        counted::held(out.len() + 1);
        out.push(found);
        true
    })?;
    Ok((!over).then_some(out))
}

/// The last `keep` sentences of `text`, nearest last, as [`split_sentences`] splits it, held
/// at most `keep` at a time whatever the length of the text.
///
/// # Errors
/// Returns a [`TextError`] when a pattern fails.
pub fn split_sentences_tail(text: &str, keep: usize) -> Result<Vec<Sentence>, TextError> {
    let mut tail = std::collections::VecDeque::new();
    each_sentence(text, &mut Work::default(), |found| {
        if tail.len() >= keep {
            tail.pop_front();
        }
        tail.push_back(found);
        counted::held(tail.len());
        true
    })?;
    Ok(tail.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cell_with_two_sentences_is_one_sentence() {
        let cell = cell_sentence(" Stops it. Then waits. ").unwrap().unwrap();
        assert_eq!(cell.text, "Stops it. Then waits.");
        assert_eq!((cell.start, cell.end, cell.word_count), (1, 22, 4));
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test: compares the limited split with the unlimited one on a short text"
    )]
    fn splitting_within_a_limit_stops_past_it_and_matches_the_plain_split_under_it() {
        let text = "Go on. Stop it. Wait.";
        assert_eq!(split_sentences_within(text, 2).unwrap(), None);
        assert_eq!(
            split_sentences_within(text, 3).unwrap(),
            Some(split_sentences(text).unwrap())
        );
    }

    #[test]
    fn a_cell_without_words_is_no_sentence() {
        assert_eq!(cell_sentence("  ---  ").unwrap(), None);
    }

    fn shapes() -> Vec<(&'static str, String)> {
        let n = 3000;
        vec![
            ("a. ", "a. ".repeat(n)),
            (". ", ". ".repeat(n)),
            ("- ", "- ".repeat(n)),
            ("A ", "A ".repeat(n)),
            ("a.b.", "a.b.".repeat(n)),
            ("x:\n", "x:\n".repeat(n)),
            ("`a`. ", "`a`. ".repeat(n)),
            ("(a). ", "(a). ".repeat(n)),
            ("em space", "A.\u{2003}".repeat(n)),
            ("nbsp", "a.\u{a0}".repeat(n)),
            ("1. ", "1. ".repeat(n)),
        ]
    }

    #[test]
    fn splitting_does_linear_work_on_repetitive_input() {
        for (name, text) in shapes() {
            let mut work = Work::default();
            split_sentences_counted(&text, &mut work).unwrap();
            assert!(
                work.0 <= 8 * text.len(),
                "{name:?}: {} units for {} bytes",
                work.0,
                text.len()
            );
        }
    }

    /// Every string over a small alphabet up to this length.
    fn all_strings(alphabet: &[char], max_len: usize) -> Vec<String> {
        let mut all = vec![String::new()];
        let mut layer = vec![String::new()];
        for _ in 0..max_len {
            layer = layer
                .iter()
                .flat_map(|s| alphabet.iter().map(move |c| format!("{s}{c}")))
                .collect();
            all.extend(layer.iter().cloned());
        }
        all
    }

    #[test]
    fn the_dotted_initialism_scan_agrees_with_the_reference_pattern() {
        let pattern = re(r"(?<![A-Za-z.])(?:[A-Za-z]\.){2,}\z");
        for text in all_strings(&['a', 'B', '.', ' ', 'é'], 8) {
            let bytes = text.as_bytes();
            let runs = dotted_runs(bytes);
            for index in (0..bytes.len()).filter(|&i| bytes[i] == b'.') {
                assert_eq!(
                    ends_in_dotted_initialism(bytes, &runs, index, &mut Work::default()),
                    pattern.is_match(&text[..=index]).unwrap(),
                    "{text:?} at {index}"
                );
            }
        }
    }

    #[test]
    fn the_protected_lookup_returns_the_first_span_containing_the_byte() {
        let spans: [&[(usize, usize)]; 4] = [
            &[(0, 10), (2, 4), (5, 6)],
            &[(0, 3), (1, 9), (4, 5), (4, 12)],
            &[(2, 3), (2, 7), (8, 9)],
            &[],
        ];
        for ranges in spans {
            let protected = Protected::new(ranges.to_vec());
            for index in 0..14 {
                let expected = ranges
                    .iter()
                    .find(|&&(s, e)| s <= index && index < e)
                    .map(|&(_, e)| e);
                assert_eq!(
                    protected.end_containing(index, &mut Work::default()),
                    expected,
                    "{ranges:?} {index}"
                );
            }
        }
    }
}
