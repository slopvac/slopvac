//! Tokenizer and sentence splitter, after prose v3.3.1's `tokenize` package and
//! Vale's handling of Markdown prose.
//!
//! Every token is a byte range into the unmodified input, so offsets never
//! drift when a word is split or when inline code is replaced.

use std::collections::HashMap;
use std::ops::Range;

/// Deterministic work counting for the linearity tests. In test builds `tick`
/// adds to a per-thread counter; elsewhere it compiles to nothing.
#[cfg(test)]
pub(crate) mod work {
    use std::cell::Cell;

    thread_local! {
        static WORK: Cell<u64> = const { Cell::new(0) };
    }

    pub(crate) fn tick(units: usize) {
        WORK.with(|w| w.set(w.get() + units as u64));
    }

    /// Returns the work done since the last call and resets the counter.
    pub(crate) fn take() -> u64 {
        WORK.with(|w| w.replace(0))
    }
}

#[cfg(not(test))]
mod work {
    #[inline(always)]
    pub(super) const fn tick(_units: usize) {}
}

/// One token of the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Token {
    /// Byte range in the original input.
    pub(crate) span: Range<usize>,
    /// What the tagger sees: the text, with each inline code span replaced by
    /// a run of `*`, one per character, as Vale does.
    pub(crate) word: String,
    /// The token is nothing but an inline code span. It conditions its
    /// neighbours' tags but is not reported.
    pub(crate) placeholder: bool,
}

/// A stretch of the working text and the input range it stands for.
struct Segment {
    work: Range<usize>,
    orig: Range<usize>,
    code: bool,
}

/// An inline code span: where its opening run starts, the text between the
/// runs, the offset just past its closing run, and how many `*` stand in for it.
struct CodeSpan {
    open: usize,
    #[cfg_attr(not(test), allow(dead_code))] // only the reference implementation in tests reads it
    inner: Range<usize>,
    end: usize,
    stars: usize,
}

/// Splits one whitespace-free chunk (starting at byte `base` of the working
/// text) into token ranges.
type SplitFn = fn(&str, usize, &mut Vec<Range<usize>>);

/// Splits `text` into tokens.
///
/// Inline code spans become runs of `*` before splitting, so that they keep
/// their place in a sentence for the tagger, as in Vale. Whitespace separates
/// chunks; each chunk is then split like prose's `Tokenizer.Tokenize`: prefix
/// and suffix punctuation peel off, and contractions split (`don't` becomes
/// `do` and `n't`). Hyphenated words stay whole, which matches Vale.
///
/// Every pass is linear in the length of `text`.
pub(crate) fn tokenize(text: &str) -> Vec<Token> {
    assemble(text, &find_code_spans(text), split_span)
}

/// Like [`tokenize`], with the code spans given instead of found: each
/// `(range, stars)` stands for a code span that covers `range` of `text` and
/// shows as `stars` asterisks. Backticks in `text` are ordinary characters.
/// The ranges must ascend, not overlap, and sit on character boundaries.
pub(crate) fn tokenize_masked(text: &str, masks: &[(Range<usize>, usize)]) -> Vec<Token> {
    let spans: Vec<CodeSpan> = masks
        .iter()
        .map(|(range, stars)| CodeSpan {
            open: range.start,
            inner: range.clone(),
            end: range.end,
            stars: (*stars).max(1),
        })
        .collect();
    assemble(text, &spans, split_span)
}

/// Finds the inline code spans of `text`, left to right.
///
/// A span opened by a run of N backticks closes at the next run of exactly N;
/// without one, the run is literal text (`CommonMark`). The next run of each
/// length is looked up from a table built in one backward pass, so unclosed
/// openers cost nothing extra.
fn find_code_spans(text: &str) -> Vec<CodeSpan> {
    let bytes = text.as_bytes();
    let mut runs: Vec<Range<usize>> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        work::tick(1);
        if bytes[i] == b'`' {
            let len = bytes[i..].iter().take_while(|&&b| b == b'`').count();
            work::tick(len);
            runs.push(i..i + len);
            i += len;
        } else {
            i += 1;
        }
    }
    let mut next_same: Vec<Option<usize>> = vec![None; runs.len()];
    let mut latest: HashMap<usize, usize> = HashMap::new();
    for (idx, run) in runs.iter().enumerate().rev() {
        work::tick(1);
        next_same[idx] = latest.insert(run.len(), idx);
    }
    let mut spans = Vec::new();
    let mut idx = 0;
    while idx < runs.len() {
        work::tick(1);
        if let Some(close) = next_same[idx] {
            spans.push(CodeSpan {
                open: runs[idx].start,
                inner: runs[idx].end..runs[close].start,
                end: runs[close].end,
                stars: text[runs[idx].end..runs[close].start].chars().count(),
            });
            idx = close + 1;
        } else {
            idx += 1;
        }
    }
    spans
}

/// Replaces the code spans with star runs, splits the result into chunks and
/// each chunk with `split`, and maps the pieces back to byte ranges of `text`.
fn assemble(text: &str, code_spans: &[CodeSpan], split: SplitFn) -> Vec<Token> {
    let mut work = String::with_capacity(text.len());
    let mut segments: Vec<Segment> = Vec::new();
    let mut plain_start = 0;
    for span in code_spans {
        push_plain(text, plain_start..span.open, &mut work, &mut segments);
        let start = work.len();
        self::work::tick(span.stars);
        work.extend(std::iter::repeat_n('*', span.stars));
        segments.push(Segment {
            work: start..work.len(),
            orig: span.open..span.end,
            code: true,
        });
        plain_start = span.end;
    }
    push_plain(text, plain_start..text.len(), &mut work, &mut segments);

    let mut spans = Vec::new();
    let mut chunk_start: Option<usize> = None;
    for (pos, c) in work.char_indices() {
        self::work::tick(1);
        // Rust's White_Space set equals Go's unicode.IsSpace (checked over all
        // code points against Go 1.27.4), which prose splits on.
        if c.is_whitespace() {
            flush(&work, &mut chunk_start, pos, &mut spans, split);
        } else if chunk_start.is_none() {
            chunk_start = Some(pos);
        }
    }
    flush(&work, &mut chunk_start, work.len(), &mut spans, split);

    spans
        .into_iter()
        .map(|r| {
            let first = segment_at(&segments, r.start);
            let last = segment_at(&segments, r.end - 1);
            let start = if first.code {
                first.orig.start
            } else {
                first.orig.start + (r.start - first.work.start)
            };
            let end = if last.code {
                last.orig.end
            } else {
                last.orig.start + (r.end - last.work.start)
            };
            Token {
                span: start..end,
                word: work[r].to_owned(),
                placeholder: first.code && std::ptr::eq(first, last),
            }
        })
        .collect()
}

fn push_plain(text: &str, orig: Range<usize>, work: &mut String, segments: &mut Vec<Segment>) {
    if orig.is_empty() {
        return;
    }
    let start = work.len();
    self::work::tick(orig.len());
    work.push_str(&text[orig.clone()]);
    segments.push(Segment {
        work: start..work.len(),
        orig,
        code: false,
    });
}

fn segment_at(segments: &[Segment], work_offset: usize) -> &Segment {
    let idx = segments.partition_point(|s| s.work.end <= work_offset);
    &segments[idx]
}

fn flush(
    work: &str,
    start: &mut Option<usize>,
    end: usize,
    out: &mut Vec<Range<usize>>,
    split: SplitFn,
) {
    if let Some(s) = start.take() {
        split(&work[s..end], s, out);
    }
}

/// Contraction endings that split off the rest of a word; ASCII case is
/// ignored. Both apostrophe forms are listed because the input is never
/// rewritten.
const CONTRACTIONS: [&str; 10] = [
    "'ll",
    "'s",
    "'re",
    "'m",
    "n't",
    "\u{2019}ll",
    "\u{2019}s",
    "\u{2019}re",
    "\u{2019}m",
    "n\u{2019}t",
];

/// Punctuation that peels off the end of a chunk (prose v3.3.1
/// `defaultSuffixes`).
const SUFFIXES: [&str; 31] = [
    ",", ")", "\"", "]", "!", ";", ".", "?", ":", "'", "\u{201d}", "\u{2019}", "\u{bb}",
    "\u{201c}", "\u{2018}", "\u{203a}", "\u{2026}", "\u{3002}", "\u{ff0c}", "\u{3001}", "\u{ff01}",
    "\u{ff1f}", "\u{ff1a}", "\u{ff1b}", "\u{ff09}", "\u{300d}", "\u{300f}", "\u{3011}", "\u{300b}",
    "\u{3009}", "\u{3015}",
];

/// Punctuation that peels off the front of a chunk (prose v3.3.1
/// `defaultPrefixes`).
const PREFIXES: [&str; 20] = [
    "$", "(", "\"", "[", "\u{201c}", "\u{2018}", "\u{ab}", "\u{201e}", "\u{201d}", "\u{201a}",
    "\u{2039}", "\u{bf}", "\u{a1}", "\u{ff08}", "\u{300c}", "\u{300e}", "\u{3010}", "\u{300a}",
    "\u{3008}", "\u{3014}",
];

/// Splits one whitespace-free chunk, which starts at byte `base` of the input.
///
/// Prefixes peel off the front, then contractions split, then suffixes peel
/// off the end, one step at a time, as in prose. Contraction positions are
/// found once for the chunk; `next` only moves forward because the span only
/// shrinks, so the whole chunk costs a constant number of steps per byte.
fn split_span(chunk: &str, base: usize, out: &mut Vec<Range<usize>>) {
    let occurrences = contraction_occurrences(chunk);
    let mut next = 0;
    // The span is chunk[lo..hi]; `base` is the input offset of chunk[0].
    let (mut lo, mut hi) = (0, chunk.len());
    // Suffixes peel off the end, so they are collected in reverse.
    let mut suffixes: Vec<Range<usize>> = Vec::new();
    while lo < hi {
        work::tick(1 + PREFIXES.len() + SUFFIXES.len());
        let span = &chunk[lo..hi];
        if is_special(span) {
            break;
        }
        if let Some(p) = PREFIXES
            .iter()
            .find(|p| span.starts_with(**p) && span.len() > p.len())
        {
            out.push(base + lo..base + lo + p.len());
            lo += p.len();
            continue;
        }
        // The earliest contraction that starts after the span's first byte and
        // ends inside it. One that fails either test now fails it for good.
        while next < occurrences.len()
            && (occurrences[next].start <= lo || occurrences[next].end > hi)
        {
            work::tick(1);
            next += 1;
        }
        if let Some(occurrence) = occurrences.get(next) {
            out.push(base + lo..base + occurrence.start);
            lo = occurrence.start;
            continue;
        }
        if let Some(s) = SUFFIXES
            .iter()
            .find(|s| span.ends_with(**s) && span.len() > s.len())
        {
            suffixes.push(base + hi - s.len()..base + hi);
            hi -= s.len();
            continue;
        }
        break;
    }
    if lo < hi {
        out.push(base + lo..base + hi);
    }
    out.extend(suffixes.into_iter().rev());
}

/// Every place in `chunk` where a contraction ending occurs (ASCII case
/// ignored), in order of start. Two endings never start at the same byte.
fn contraction_occurrences(chunk: &str) -> Vec<Range<usize>> {
    let bytes = chunk.as_bytes();
    work::tick(bytes.len() * CONTRACTIONS.len());
    let mut found = Vec::new();
    for start in 1..bytes.len() {
        for ending in CONTRACTIONS {
            let ending = ending.as_bytes();
            if bytes[start..].len() >= ending.len()
                && bytes[start..start + ending.len()].eq_ignore_ascii_case(ending)
            {
                found.push(start..start + ending.len());
                break;
            }
        }
    }
    found
}

/// Spans prose keeps whole: dotted abbreviations (`e.g.`, `U.S.`) and short
/// capitalized ones (`Dr.`, `Mr.`).
/// Go: `^(?:[A-Za-z]\.){2,}$|^[A-Z][a-z]{1,2}\.$`.
fn is_special(span: &str) -> bool {
    let b = span.as_bytes();
    work::tick(1);
    let pair = |p: &[u8]| p[0].is_ascii_alphabetic() && p[1] == b'.';
    // Checking both ends first rejects most spans in constant time, so
    // peeling a long run of punctuation does not rescan the middle each step.
    let dotted = b.len() >= 4
        && b.len().is_multiple_of(2)
        && pair(&b[..2])
        && pair(&b[b.len() - 2..])
        && b.as_chunks::<2>().0.iter().all(|p| {
            work::tick(1);
            pair(p)
        });
    let title = (3..=4).contains(&b.len())
        && b[0].is_ascii_uppercase()
        && b[b.len() - 1] == b'.'
        && b[1..b.len() - 1].iter().all(u8::is_ascii_lowercase);
    dotted || title
}

/// Groups `tokens` into sentences, as ranges of token indexes.
///
/// A sentence ends after `.`, `!` or `?` (plus any closing quotes or brackets
/// attached to it) when the next token does not start with a lowercase letter,
/// and at every blank line of `text`.
pub(crate) fn sentences(text: &str, tokens: &[Token]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < tokens.len() {
        work::tick(1);
        let mut end = i + 1;
        let mut boundary = false;
        if !tokens[i].placeholder
            && is_terminal(&tokens[i].word)
            && !follows_abbreviation(tokens, i)
        {
            while end < tokens.len()
                && tokens[end].span.start == tokens[end - 1].span.end
                && is_closer(&tokens[end].word)
            {
                work::tick(1);
                end += 1;
            }
            boundary = end == tokens.len() || {
                let next = &tokens[end].word;
                !starts_lowercase(next) && !is_closer(next)
            };
        }
        if end < tokens.len()
            && blank_line_between(&text[tokens[end - 1].span.end..tokens[end].span.start])
        {
            boundary = true;
        }
        if boundary {
            out.push(start..end);
            start = end;
        }
        i = end;
    }
    if start < tokens.len() {
        out.push(start..tokens.len());
    }
    out
}

fn blank_line_between(gap: &str) -> bool {
    work::tick(gap.len());
    gap.bytes().filter(|&b| b == b'\n').count() >= 2
}

fn is_terminal(word: &str) -> bool {
    matches!(word, "." | "!" | "?")
}

fn is_closer(word: &str) -> bool {
    word.chars().all(|c| {
        matches!(
            c,
            '"' | '\'' | ')' | ']' | '\u{201d}' | '\u{2019}' | '.' | '!' | '?'
        )
    })
}

fn starts_lowercase(word: &str) -> bool {
    word.chars().next().is_some_and(crate::go_unicode::is_lower)
}

/// A period after one of these does not end the sentence.
const ABBREVIATIONS: [&str; 8] = ["vs", "cf", "fig", "approx", "eg", "ie", "ca", "resp"];

fn follows_abbreviation(tokens: &[Token], i: usize) -> bool {
    tokens[i].word == "."
        && i > 0
        && tokens[i - 1].span.end == tokens[i].span.start
        && ABBREVIATIONS
            .iter()
            .any(|a| tokens[i - 1].word.eq_ignore_ascii_case(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        tokenize(text).into_iter().map(|t| t.word).collect()
    }

    #[test]
    fn splits_punctuation_and_contractions() {
        assert_eq!(
            words("(Don't) run it, \"now\"."),
            [
                "(", "Do", "n't", ")", "run", "it", ",", "\"", "now", "\"", "."
            ]
        );
    }

    #[test]
    fn keeps_hyphenated_words_and_abbreviations_whole() {
        assert_eq!(
            words("A non-interactive run, e.g. Dr. Smith."),
            [
                "A",
                "non-interactive",
                "run",
                ",",
                "e.g.",
                "Dr.",
                "Smith",
                "."
            ]
        );
    }

    #[test]
    fn inline_code_becomes_a_star_placeholder_per_character() {
        let tokens = tokenize("Run `kubectl` now, or ``a ` b``.");
        let got: Vec<(&str, bool)> = tokens
            .iter()
            .map(|t| (t.word.as_str(), t.placeholder))
            .collect();
        assert_eq!(
            got,
            [
                ("Run", false),
                ("*******", true),
                ("now", false),
                (",", false),
                ("or", false),
                ("*****", true),
                (".", false),
            ]
        );
    }

    #[test]
    fn placeholder_span_covers_the_whole_code_span() {
        let text = "Use `ab` here";
        let tokens = tokenize(text);
        assert_eq!(&text[tokens[1].span.clone()], "`ab`");
    }

    #[test]
    fn unclosed_backtick_is_text() {
        assert_eq!(words("a ` b"), ["a", "`", "b"]);
    }

    #[test]
    fn spans_are_utf8_byte_offsets() {
        let text = "Caf\u{e9} \u{201c}na\u{ef}ve\u{201d} idea";
        let got: Vec<(&str, usize)> = tokenize(text)
            .iter()
            .map(|t| (&text[t.span.clone()], t.span.start))
            .collect();
        assert_eq!(
            got,
            [
                ("Caf\u{e9}", 0),
                ("\u{201c}", 6),
                ("na\u{ef}ve", 9),
                ("\u{201d}", 15),
                ("idea", 19)
            ]
        );
    }

    #[test]
    fn spans_stay_exact_after_multibyte_text_in_and_around_code() {
        let text = "\u{e9}t\u{e9} `\u{e9}\u{e9}` d\u{e9}j\u{e0}";
        let got: Vec<&str> = tokenize(text)
            .iter()
            .map(|t| &text[t.span.clone()])
            .collect();
        assert_eq!(got, ["\u{e9}t\u{e9}", "`\u{e9}\u{e9}`", "d\u{e9}j\u{e0}"]);
    }

    #[test]
    fn sentences_split_after_terminals_not_before_lowercase() {
        let text = "Stop. Go now! (See vs. this.) Ok? yes";
        let toks = tokenize(text);
        let sents: Vec<Vec<&str>> = sentences(text, &toks)
            .into_iter()
            .map(|r| toks[r].iter().map(|t| t.word.as_str()).collect())
            .collect();
        assert_eq!(
            sents,
            [
                vec!["Stop", "."],
                vec!["Go", "now", "!"],
                vec!["(", "See", "vs", ".", "this", ".", ")"],
                vec!["Ok", "?", "yes"],
            ]
        );
    }

    #[test]
    fn blank_line_ends_a_sentence() {
        let text = "Title\n\nBody text";
        let toks = tokenize(text);
        assert_eq!(sentences(text, &toks), [0..1, 1..3]);
    }
}

/// The quadratic implementation this module replaced, kept as an oracle: the
/// linear code must produce the same tokens.
#[cfg(test)]
mod oracle {
    use super::*;

    fn code_span(text: &str, open: usize) -> Option<(Range<usize>, usize)> {
        let bytes = text.as_bytes();
        let run = bytes[open..].iter().take_while(|&&b| b == b'`').count();
        let mut i = open + run;
        while i < bytes.len() {
            work::tick(1);
            if bytes[i] == b'`' {
                let close = bytes[i..].iter().take_while(|&&b| b == b'`').count();
                work::tick(close);
                if close == run {
                    return Some((open + run..i, i + close));
                }
                i += close;
            } else {
                i += 1;
            }
        }
        None
    }

    pub(super) fn find_code_spans(text: &str) -> Vec<CodeSpan> {
        let mut spans = Vec::new();
        let mut i = 0;
        while i < text.len() {
            work::tick(1);
            if text.as_bytes()[i] != b'`' {
                i += 1;
            } else if let Some((inner, end)) = code_span(text, i) {
                spans.push(CodeSpan {
                    open: i,
                    stars: text[inner.clone()].chars().count(),
                    inner,
                    end,
                });
                i = end;
            } else {
                i += text.as_bytes()[i..]
                    .iter()
                    .take_while(|&&b| b == b'`')
                    .count();
            }
        }
        spans
    }

    fn contraction_at(span: &str) -> Option<usize> {
        let bytes = span.as_bytes();
        CONTRACTIONS
            .iter()
            .filter_map(|c| {
                let c = c.as_bytes();
                (1..=bytes.len().checked_sub(c.len())?).find(|&i| {
                    work::tick(1);
                    bytes[i..i + c.len()].eq_ignore_ascii_case(c)
                })
            })
            .min()
    }

    fn is_special(span: &str) -> bool {
        let b = span.as_bytes();
        let dotted = b.len() >= 4
            && b.len().is_multiple_of(2)
            && b.as_chunks::<2>().0.iter().all(|p| {
                work::tick(1);
                p[0].is_ascii_alphabetic() && p[1] == b'.'
            });
        let title = (3..=4).contains(&b.len())
            && b[0].is_ascii_uppercase()
            && b[b.len() - 1] == b'.'
            && b[1..b.len() - 1].iter().all(u8::is_ascii_lowercase);
        dotted || title
    }

    pub(super) fn split_span(chunk: &str, base: usize, out: &mut Vec<Range<usize>>) {
        let mut span = chunk;
        let mut base = base;
        let mut suffixes: Vec<Range<usize>> = Vec::new();
        while !span.is_empty() {
            work::tick(1 + PREFIXES.len() + SUFFIXES.len());
            if is_special(span) {
                break;
            }
            if let Some(p) = PREFIXES
                .iter()
                .find(|p| span.starts_with(**p) && span.len() > p.len())
            {
                out.push(base..base + p.len());
                span = &span[p.len()..];
                base += p.len();
                continue;
            }
            if let Some(idx) = contraction_at(span) {
                out.push(base..base + idx);
                span = &span[idx..];
                base += idx;
                continue;
            }
            if let Some(s) = SUFFIXES
                .iter()
                .find(|s| span.ends_with(**s) && span.len() > s.len())
            {
                let cut = span.len() - s.len();
                suffixes.push(base + cut..base + span.len());
                span = &span[..cut];
                continue;
            }
            break;
        }
        if !span.is_empty() {
            out.push(base..base + span.len());
        }
        out.extend(suffixes.into_iter().rev());
    }

    pub(super) fn tokenize(text: &str) -> Vec<Token> {
        assemble(text, &find_code_spans(text), split_span)
    }
}

#[cfg(test)]
mod differential_tests {
    use super::*;

    /// A small deterministic generator (xorshift64*), so failures reproduce.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }

        fn below(&mut self, n: usize) -> usize {
            usize::try_from(self.next() % u64::try_from(n).unwrap()).unwrap()
        }
    }

    /// Pieces that exercise every affix, contraction, code-span and
    /// abbreviation rule, including overlapping and unclosed cases.
    const PIECES: [&str; 44] = [
        "a",
        "B",
        "n",
        "t",
        "'",
        "'s",
        "'LL",
        "n't",
        "N'T",
        "\u{2019}",
        "\u{2019}re",
        "n\u{2019}t",
        "m",
        "e.g.",
        "U.S.",
        "Dr.",
        "x.y.",
        " ",
        " ",
        "\n",
        "\n\n",
        ".",
        ",",
        "!",
        "?",
        ":",
        ";",
        "(",
        ")",
        "[",
        "]",
        "\"",
        "$",
        "\u{201c}",
        "\u{201d}",
        "\u{2039}",
        "-",
        "`",
        "``",
        "```",
        "*",
        "\u{e9}",
        "\u{ff08}",
        "\u{3002}",
    ];

    fn random_text(rng: &mut Rng) -> String {
        let n = rng.below(40);
        (0..n).map(|_| PIECES[rng.below(PIECES.len())]).collect()
    }

    #[test]
    fn linear_tokenizer_matches_the_quadratic_oracle_on_random_text() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for _ in 0..40_000 {
            let text = random_text(&mut rng);
            assert_eq!(tokenize(&text), oracle::tokenize(&text), "{text:?}");
        }
    }

    #[test]
    fn linear_tokenizer_matches_the_quadratic_oracle_on_the_corpus_slice() {
        let corpus = include_str!("../tests/fixtures/corpus.md");
        assert_eq!(tokenize(corpus), oracle::tokenize(corpus));
        for line in corpus.lines() {
            assert_eq!(tokenize(line), oracle::tokenize(line), "{line:?}");
        }
    }

    #[test]
    fn code_span_lookup_matches_the_oracle() {
        let mut rng = Rng(7);
        for _ in 0..20_000 {
            let text = random_text(&mut rng);
            let new: Vec<_> = find_code_spans(&text)
                .into_iter()
                .map(|s| (s.open, s.inner, s.end))
                .collect();
            let old: Vec<_> = oracle::find_code_spans(&text)
                .into_iter()
                .map(|s| (s.open, s.inner, s.end))
                .collect();
            assert_eq!(new, old, "{text:?}");
        }
    }
}

#[cfg(test)]
#[path = "../tests/common/adversarial.rs"]
mod adversarial;

#[cfg(test)]
mod work_tests {
    use super::*;

    fn linear_work(text: &str) -> u64 {
        work::take();
        let tokens = tokenize(text);
        let _ = sentences(text, &tokens);
        work::take()
    }

    fn oracle_work(text: &str) -> u64 {
        work::take();
        let tokens = oracle::tokenize(text);
        let _ = sentences(text, &tokens);
        work::take()
    }

    /// Work units allowed per input byte, and a constant allowance. The
    /// linear code spends at most 67 units per byte (peeling one punctuation
    /// character costs a table lookup of each affix); quadratic code passes
    /// this budget as soon as the input is a few thousand bytes.
    const PER_BYTE: u64 = 70;
    const ALLOWANCE: u64 = 1_000;

    fn budget(text: &str) -> u64 {
        PER_BYTE * text.len() as u64 + ALLOWANCE
    }

    /// Big enough for the unique-length backtick case to show its n^1.5 cost.
    fn size_for(name: &str, small: usize) -> usize {
        if name == "backtick runs" {
            200_000
        } else {
            small
        }
    }

    #[test]
    fn tokenizing_stays_within_a_linear_work_budget_on_adversarial_inputs() {
        for name in adversarial::NAMES {
            let n = size_for(name, 40_000);
            let text = adversarial::adversarial(name, n);
            let work = linear_work(&text);
            assert!(
                work <= budget(&text),
                "{name}: {work} units for {} bytes exceeds {}",
                text.len(),
                budget(&text)
            );
            // Doubling the input must not much more than double the work.
            let double = adversarial::adversarial(name, 2 * n);
            let doubled = linear_work(&double);
            assert!(
                doubled * 10 <= work * 22,
                "{name}: doubling the input took work from {work} to {doubled}"
            );
        }
    }

    /// The same budget rejects the quadratic implementation this module
    /// replaced, on every input that made it quadratic. This is what makes
    /// the gate above meaningful: reverting an optimisation fails it.
    #[test]
    fn the_budget_rejects_the_old_quadratic_code() {
        for name in [
            "bang run",
            "wrapped word",
            "contractions",
            "contraction tail",
            "dotted abbreviation tail",
            "backtick runs",
        ] {
            let n = size_for(name, 4_000);
            let text = adversarial::adversarial(name, n);
            let work = oracle_work(&text);
            assert!(
                work > budget(&text),
                "{name}: the old code used only {work} units for {} bytes (budget {})",
                text.len(),
                budget(&text)
            );
        }
    }
}
