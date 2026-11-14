//! Tokenizer and sentence splitter, after prose v3.3.1's `tokenize` package and
//! Vale's handling of Markdown prose.
//!
//! Every token is a byte range into the unmodified input, so offsets never
//! drift when a word is split or when inline code is replaced.

use std::ops::Range;

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

/// Splits `text` into tokens.
///
/// Inline code spans become runs of `*` before splitting, so that they keep
/// their place in a sentence for the tagger, as in Vale. Whitespace separates
/// chunks; each chunk is then split like prose's `Tokenizer.Tokenize`: prefix
/// and suffix punctuation peel off, and contractions split (`don't` becomes
/// `do` and `n't`). Hyphenated words stay whole, which matches Vale.
pub(crate) fn tokenize(text: &str) -> Vec<Token> {
    let mut work = String::with_capacity(text.len());
    let mut segments: Vec<Segment> = Vec::new();
    let mut plain_start = 0;
    let mut i = 0;
    while i < text.len() {
        if text.as_bytes()[i] != b'`' {
            i += 1;
        } else if let Some((inner, end)) = code_span(text, i) {
            push_plain(text, plain_start..i, &mut work, &mut segments);
            let start = work.len();
            work.extend(text[inner].chars().map(|_| '*'));
            segments.push(Segment {
                work: start..work.len(),
                orig: i..end,
                code: true,
            });
            plain_start = end;
            i = end;
        } else {
            // Unclosed: the whole run of backticks is literal text.
            i += text.as_bytes()[i..]
                .iter()
                .take_while(|&&b| b == b'`')
                .count();
        }
    }
    push_plain(text, plain_start..text.len(), &mut work, &mut segments);

    let mut spans = Vec::new();
    let mut chunk_start: Option<usize> = None;
    for (pos, c) in work.char_indices() {
        if c.is_whitespace() {
            flush(&work, &mut chunk_start, pos, &mut spans);
        } else if chunk_start.is_none() {
            chunk_start = Some(pos);
        }
    }
    flush(&work, &mut chunk_start, work.len(), &mut spans);

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

fn flush(work: &str, start: &mut Option<usize>, end: usize, out: &mut Vec<Range<usize>>) {
    if let Some(s) = start.take() {
        split_span(&work[s..end], s, out);
    }
}

/// If a code span opens at `open` (a backtick), returns its inner range and
/// the offset just past its closing run. A span opened by a run of N backticks
/// closes at the next run of exactly N; without one, the backticks are literal
/// text (`CommonMark`).
fn code_span(text: &str, open: usize) -> Option<(Range<usize>, usize)> {
    let bytes = text.as_bytes();
    let run = bytes[open..].iter().take_while(|&&b| b == b'`').count();
    let mut i = open + run;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let close = bytes[i..].iter().take_while(|&&b| b == b'`').count();
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
fn split_span(chunk: &str, base: usize, out: &mut Vec<Range<usize>>) {
    let mut span = chunk;
    let mut base = base;
    // Suffixes peel off the end, so they are collected in reverse.
    let mut suffixes: Vec<Range<usize>> = Vec::new();
    while !span.is_empty() {
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

/// Byte offset of the earliest contraction ending after the first byte.
fn contraction_at(span: &str) -> Option<usize> {
    let bytes = span.as_bytes();
    CONTRACTIONS
        .iter()
        .filter_map(|c| {
            let c = c.as_bytes();
            (1..=bytes.len().checked_sub(c.len())?)
                .find(|&i| bytes[i..i + c.len()].eq_ignore_ascii_case(c))
        })
        .min()
}

/// Spans prose keeps whole: dotted abbreviations (`e.g.`, `U.S.`) and short
/// capitalized ones (`Dr.`, `Mr.`).
/// Go: `^(?:[A-Za-z]\.){2,}$|^[A-Z][a-z]{1,2}\.$`.
fn is_special(span: &str) -> bool {
    let b = span.as_bytes();
    let dotted = b.len() >= 4
        && b.len().is_multiple_of(2)
        && b.as_chunks::<2>()
            .0
            .iter()
            .all(|p| p[0].is_ascii_alphabetic() && p[1] == b'.');
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
    word.chars().next().is_some_and(char::is_lowercase)
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
