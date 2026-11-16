//! Doc- and repo-level proper-noun inference.
//!
//! Capitalised words in the middle of a sentence are weak evidence of a name.
//! This module weighs that evidence over one document, or over every document
//! of a repository, and reports the words and phrases that pass. The result
//! feeds `terms suggest` and the heading checks. It never exempts a word on its
//! own: the caller decides what to do with a candidate.
//!
//! Evidence comes from body prose only. Headings, fenced code, inline code,
//! link destinations and URLs are skipped, because a heading capitalises words
//! by style and says nothing about names. A capitalised word at the start of a
//! sentence is skipped for the same reason.
//!
//! A word passes when at least 80% of its mid-sentence uses are capitalised.
//! It needs two capitalised uses, or one when it is not in the dictionary: a
//! coinage such as `Kubernetes` is a name on first sight, while `Python` or
//! `Desktop` could be a capitalised common word. A phrase is judged first: two
//! adjacent capitalised words pass as a pair under the same rules, using the
//! lower-case uses of the pair, and chains of passing pairs form one phrase.
//!
//! Only the shape `Xxxx` (one capital, then lower-case ASCII letters) counts.
//! Acronyms and mixed case such as `GitHub` need no inference.
//!
//! Lower-case evidence for a pair comes from the document that holds a
//! capitalised use of the pair, so a pair that is only lower case elsewhere in
//! the repository is not held against it.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ops::Range;

pub mod ab;

use serde::{Deserialize, Serialize};

/// Minimum share of capitalised uses, in fifths.
const RATIO_NUM: u32 = 4;
const RATIO_DEN: u32 = 5;

/// Capitalised and lower-case mid-sentence uses of a word or pair.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// Capitalised uses in the middle of a sentence.
    pub hits: u32,
    /// Lower-case uses.
    pub lower: u32,
}

impl Evidence {
    fn passes(self, need: u32) -> bool {
        self.hits >= need && self.hits * RATIO_DEN >= RATIO_NUM * (self.hits + self.lower)
    }

    fn add(&mut self, other: Self) {
        self.hits += other.hits;
        self.lower += other.lower;
    }
}

/// Words that count as ordinary vocabulary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dictionary(BTreeSet<String>);

impl Dictionary {
    /// A dictionary of the given words, compared in lower case.
    pub fn from_words<I, S>(words: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self(
            words
                .into_iter()
                .map(|w| w.as_ref().to_ascii_lowercase())
                .collect(),
        )
    }

    /// Every word that `counts` saw in lower case.
    #[must_use]
    pub fn from_counts(counts: &Counts) -> Self {
        Self(
            counts
                .words
                .iter()
                .filter(|(_, e)| e.lower > 0)
                .map(|(w, _)| w.clone())
                .collect(),
        )
    }

    /// Whether `word` (any case) is in the dictionary.
    #[must_use]
    pub fn contains(&self, word: &str) -> bool {
        self.0.contains(&word.to_ascii_lowercase())
    }
}

/// Mid-sentence word and pair counts over some body text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    words: BTreeMap<String, Evidence>,
    pairs: BTreeMap<String, Evidence>,
}

impl Counts {
    /// Add the body prose of `text` (Markdown or plain text).
    pub fn observe(&mut self, text: &str) {
        let toks = scan(text, true);
        for t in &toks {
            match t.shape {
                Shape::Cap if !t.starts => self.words.entry(t.key.clone()).or_default().hits += 1,
                Shape::Lower => self.words.entry(t.key.clone()).or_default().lower += 1,
                _ => {}
            }
        }
        let mut seen: HashSet<String> = HashSet::new();
        let mut buf = String::new();
        for pair in toks.windows(2) {
            if pair[1].joined
                && !pair[0].starts
                && pair[0].shape == Shape::Cap
                && pair[1].shape == Shape::Cap
            {
                pair_key(&mut buf, &pair[0].key, &pair[1].key);
                self.pairs.entry(buf.clone()).or_default().hits += 1;
                seen.insert(buf.clone());
            }
        }
        for pair in toks.windows(2) {
            if pair[1].joined && pair[0].shape == Shape::Lower && pair[1].shape == Shape::Lower {
                pair_key(&mut buf, &pair[0].key, &pair[1].key);
                if seen.contains(buf.as_str()) {
                    self.pairs.entry(buf.clone()).or_default().lower += 1;
                }
            }
        }
    }

    /// Add the counts of another set, as if both had been observed together.
    pub fn merge(&mut self, other: &Self) {
        for (k, e) in &other.words {
            self.words.entry(k.clone()).or_default().add(*e);
        }
        for (k, e) in &other.pairs {
            self.pairs.entry(k.clone()).or_default().add(*e);
        }
    }
}

fn pair_key(buf: &mut String, a: &str, b: &str) {
    buf.clear();
    buf.push_str(a);
    buf.push(' ');
    buf.push_str(b);
}

/// Per-file counts with a content digest, so a repository scan recounts only
/// the files that changed. Serialise it as the cache.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoCounts {
    files: BTreeMap<String, FileCounts>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FileCounts {
    digest: u64,
    counts: Counts,
}

/// FNV-1a: a cache key, not a security boundary.
fn digest(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

impl RepoCounts {
    /// Count `text` under `name`. Returns `false` when the cache already holds
    /// this exact text for `name`.
    pub fn update(&mut self, name: &str, text: &str) -> bool {
        let digest = digest(text);
        if self.files.get(name).is_some_and(|f| f.digest == digest) {
            return false;
        }
        let mut counts = Counts::default();
        counts.observe(text);
        self.files
            .insert(name.to_owned(), FileCounts { digest, counts });
        true
    }

    /// Forget `name`. Returns whether it was cached.
    pub fn remove(&mut self, name: &str) -> bool {
        self.files.remove(name).is_some()
    }

    /// The counts of every cached file together.
    #[must_use]
    pub fn total(&self) -> Counts {
        let mut all = Counts::default();
        for f in self.files.values() {
            all.merge(&f.counts);
        }
        all
    }
}

/// Words and phrases that passed, with the evidence for each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inferred {
    words: BTreeMap<String, Evidence>,
    pairs: BTreeMap<String, Evidence>,
}

/// Weigh `counts` against `dict`.
#[must_use]
pub fn infer(counts: &Counts, dict: &Dictionary) -> Inferred {
    let words = counts
        .words
        .iter()
        .filter(|(w, e)| e.passes(if dict.contains(w) { 2 } else { 1 }))
        .map(|(w, e)| (w.clone(), *e))
        .collect();
    let pairs = counts
        .pairs
        .iter()
        .filter(|(p, e)| {
            let both_known = p.split(' ').all(|w| dict.contains(w));
            e.passes(if both_known { 2 } else { 1 })
        })
        .map(|(p, e)| (p.clone(), *e))
        .collect();
    Inferred { words, pairs }
}

impl Inferred {
    /// Single words (lower case) with their evidence, in order.
    pub fn words(&self) -> impl Iterator<Item = (&str, Evidence)> {
        self.words.iter().map(|(w, e)| (w.as_str(), *e))
    }

    /// Two-word phrases (lower case) with their evidence, in order.
    pub fn phrases(&self) -> impl Iterator<Item = (&str, Evidence)> {
        self.pairs.iter().map(|(p, e)| (p.as_str(), *e))
    }

    /// Byte ranges of the capitalised words in `text` that this inference
    /// names. Chains of phrases are taken first, then single words.
    #[must_use]
    pub fn mask(&self, text: &str) -> Vec<Range<usize>> {
        let toks = scan(text, false);
        let mut out = Vec::new();
        let mut buf = String::new();
        let mut i = 0;
        while i < toks.len() {
            if toks[i].shape != Shape::Cap {
                i += 1;
                continue;
            }
            let mut j = i;
            while j + 1 < toks.len() && toks[j + 1].joined && toks[j + 1].shape == Shape::Cap {
                pair_key(&mut buf, &toks[j].key, &toks[j + 1].key);
                if !self.pairs.contains_key(buf.as_str()) {
                    break;
                }
                j += 1;
            }
            if j > i || self.words.contains_key(&toks[i].key) {
                out.extend(toks[i..=j].iter().map(|t| t.range.clone()));
            }
            i = j + 1;
        }
        out
    }

    /// Capitalised words in a heading, after the first word, that no inferred
    /// name explains. A title-case check counts these instead of every
    /// capitalised word, so a heading made of names is not flagged. This only
    /// counts; the caller decides what the count means.
    #[must_use]
    pub fn residual_capitals(&self, heading: &str) -> usize {
        let named = self.mask(heading);
        // The first word is the first thing in the heading. A leading code span
        // is that thing, so the first word after it is not exempt.
        let leads_with_code = blank_spans(heading.trim_start()).starts_with('\u{1}');
        scan(heading, false)
            .iter()
            .skip(usize::from(!leads_with_code))
            .filter(|t| t.shape == Shape::Cap && !named.contains(&t.range))
            .count()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// `Xxxx`: one capital, then lower-case ASCII letters.
    Cap,
    /// Lower-case ASCII letters only.
    Lower,
    Other,
}

struct Token {
    range: Range<usize>,
    shape: Shape,
    /// Lower-case spelling.
    key: String,
    /// First word of a sentence, list item, quote or paragraph.
    starts: bool,
    /// Follows the previous token after exactly one space.
    joined: bool,
}

fn shape_of(word: &str) -> Shape {
    let b = word.as_bytes();
    if b.len() < 2 || !b.iter().all(u8::is_ascii_alphabetic) {
        return Shape::Other;
    }
    if b[1..].iter().all(u8::is_ascii_lowercase) {
        if b[0].is_ascii_uppercase() {
            return Shape::Cap;
        }
        if b[0].is_ascii_lowercase() {
            return Shape::Lower;
        }
    }
    Shape::Other
}

/// Length of the code span that opens `rest`: a run of backticks up to the
/// next run of the same length. `None` when the run never closes.
fn code_span_len(rest: &str) -> Option<usize> {
    let open = rest.bytes().take_while(|b| *b == b'`').count();
    let mut at = open;
    while let Some(found) = rest[at..].find('`') {
        let start = at + found;
        let run = rest[start..].bytes().take_while(|b| *b == b'`').count();
        if run == open {
            return Some(start + run);
        }
        at = start + run;
    }
    None
}

/// Replace code spans, link destinations, URLs and tags with a filler that
/// keeps byte offsets and separates tokens.
fn blank_spans(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < line.len() {
        let rest = &line[i..];
        let end = if rest.starts_with('`') {
            code_span_len(rest)
        } else if rest.starts_with("](") {
            rest.find(')').map(|j| j + 1)
        } else if rest.starts_with("http://") || rest.starts_with("https://") {
            Some(rest.find(char::is_whitespace).unwrap_or(rest.len()))
        } else if rest.starts_with('<')
            && rest[1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '/')
        {
            rest.find('>').map(|j| j + 1)
        } else {
            None
        };
        if let Some(n) = end {
            out.push_str(&"\u{1}".repeat(n));
            i += n;
        } else if let Some(c) = rest.chars().next() {
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// Blank `[text](#anchor)`: a table of contents repeats the headings, so its
/// link text is an echo of them, not an independent use of the words.
fn blank_anchor_links(line: &str) -> String {
    let mut out = line.to_owned();
    let mut from = 0;
    while let Some(at) = out[from..].find("](#").map(|i| i + from) {
        let Some(open) = out[..at].rfind('[') else {
            break;
        };
        let Some(close) = out[at..].find(')').map(|i| i + at + 1) else {
            break;
        };
        out.replace_range(open..close, &"\u{1}".repeat(close - open));
        from = close;
    }
    out
}

/// A line that is only bold text works as a heading.
fn is_bold_only(line: &str) -> bool {
    let t = line.trim();
    ["**", "__"]
        .iter()
        .any(|m| t.len() > 4 && t.starts_with(m) && t.ends_with(m) && t.matches(m).count() == 2)
}

/// Length of a block marker (`>`, list bullet, ordered number) at the start of
/// `line`, after indentation.
fn marker_len(line: &str) -> usize {
    let mut at = 0;
    loop {
        let rest = &line[at..];
        let trimmed = rest.trim_start_matches([' ', '\t']);
        let indent = rest.len() - trimmed.len();
        let step = if trimmed.starts_with('>') {
            1
        } else if trimmed.starts_with(['-', '*', '+']) && trimmed[1..].starts_with([' ', '\t']) {
            2
        } else {
            let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
            if digits > 0
                && trimmed[digits..].starts_with(['.', ')'])
                && trimmed[digits + 1..].starts_with([' ', '\t'])
            {
                digits + 2
            } else {
                return at;
            }
        };
        at += indent + step;
    }
}

/// Whether the text between two tokens ends a sentence.
fn ends_sentence(gap: &str, last_in_line: bool) -> bool {
    gap.char_indices().any(|(i, c)| {
        if !matches!(c, '.' | '?' | '!' | ':') {
            return false;
        }
        let after = gap[i + c.len_utf8()..]
            .trim_start_matches(['"', '\'', ')', ']', '*', '_', '\u{201d}', '\u{2019}']);
        after.starts_with(char::is_whitespace) || (last_in_line && after.is_empty())
    })
}

/// A `===` or `---` underline, which turns the line above it into a heading.
fn is_setext_rule(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 2 && (t.bytes().all(|b| b == b'=') || t.bytes().all(|b| b == b'-'))
}

/// An opening or closing fence: its character, run length and what follows.
fn fence_of(rest: &str) -> Option<(char, usize, &str)> {
    let t = rest.trim_start_matches([' ', '\t']);
    let ch = t.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let run = t.chars().take_while(|c| *c == ch).count();
    (run >= 3).then(|| (ch, run, &t[run..]))
}

fn is_atx_heading(rest: &str) -> bool {
    let t = rest.trim_start_matches([' ', '\t']);
    let after = t.trim_start_matches('#');
    t.starts_with('#')
        && t.len() - after.len() <= 6
        && (after.is_empty() || after.starts_with([' ', '\t']))
}

fn scan(text: &str, body: bool) -> Vec<Token> {
    let mut out = Vec::new();
    let mut start = true;
    let mut fence: Option<(char, usize)> = None;
    let mut front: Option<&str> = None;
    let mut offset = 0;
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    for (n, raw) in lines.iter().enumerate() {
        let base = offset;
        offset += raw.len();
        let line = raw.trim_end_matches(['\n', '\r']);
        let trimmed = line.trim();
        if body {
            if n == 0 && matches!(trimmed, "---" | "+++") {
                front = Some(if trimmed == "---" { "---" } else { "+++" });
                continue;
            }
            if let Some(delim) = front {
                if trimmed == delim || (delim == "---" && trimmed == "...") {
                    front = None;
                    start = true;
                }
                continue;
            }
        }
        let skip = if body { marker_len(line) } else { 0 };
        let rest = &line[skip..];
        if body {
            if let Some((ch, len)) = fence {
                if let Some((c, run, tail)) = fence_of(rest)
                    && c == ch
                    && run >= len
                    && tail.trim().is_empty()
                {
                    fence = None;
                }
                start = true;
                continue;
            }
            if let Some((c, run, tail)) = fence_of(rest)
                && (c == '~' || !tail.contains('`'))
            {
                fence = Some((c, run));
                start = true;
                continue;
            }
            if (!trimmed.is_empty() && lines.get(n + 1).is_some_and(|l| is_setext_rule(l)))
                || is_atx_heading(rest)
                || trimmed.is_empty()
                || trimmed.starts_with("!!!")
                || trimmed.starts_with("???")
            {
                start = true;
                continue;
            }
        }
        let cleaned = blank_spans(&blank_anchor_links(line));
        if body && is_bold_only(&cleaned[skip..]) {
            start = true;
            continue;
        }
        if skip > 0 {
            start = true;
        }
        scan_line(&cleaned, skip, base, &mut start, &mut out);
    }
    out
}

/// Characters that belong to a word: letters, digits and combining marks.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(
            c as u32,
            0x0300..=0x036f | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff | 0x20d0..=0x20ff | 0xfe20..=0xfe2f
        )
}

fn scan_line(line: &str, from: usize, base: usize, start: &mut bool, out: &mut Vec<Token>) {
    let mut words: Vec<Range<usize>> = Vec::new();
    let mut begin: Option<usize> = None;
    for (i, c) in line.char_indices().filter(|(i, _)| *i >= from) {
        match (is_word_char(c), begin) {
            (true, None) => begin = Some(i),
            (false, Some(s)) => {
                words.push(s..i);
                begin = None;
            }
            _ => {}
        }
    }
    if let Some(s) = begin {
        words.push(s..line.len());
    }
    for (n, w) in words.iter().enumerate() {
        let word = &line[w.clone()];
        let joined = n > 0 && &line[words[n - 1].end..w.start] == " ";
        let shape = shape_of(word);
        out.push(Token {
            range: base + w.start..base + w.end,
            shape,
            key: word.to_ascii_lowercase(),
            starts: *start,
            joined,
        });
        let last = n + 1 == words.len();
        let gap = if last {
            &line[w.end..]
        } else {
            &line[w.end..words[n + 1].start]
        };
        *start = ends_sentence(gap, last);
    }
}
