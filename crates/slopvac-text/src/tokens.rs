//! Word counting per ASD-STE100 rules 8.4 to 8.7. A port of the Python reference
//! (`analyze._ste_tokens`); see `docs/metrics.md` for the contract.

use std::sync::LazyLock;

use slopvac_pcre2::Regex;

use crate::lexicon::IMPERATIVE_VERBS;
use crate::pattern::{TextError, find_all, re, re_i, replace_all};
use crate::pyrules::{lower, py_isspace, py_strip};

pub(crate) const SENTINEL: char = '\0';

const NUMBER: &str = r"[+-]?\d+(?:[.,]\d+)*(?:\.\d+)*";
const UNIT: &str = concat!(
    r"(?:",
    r"°[CF]?|K|",
    r"[numkKMGTP]?(?:m|g|s|V|W|J|N|Pa|Hz|B|bps|bit|byte|bytes|",
    r"[bB]|[iI]?B)|",
    r"ms|us|ns|ps|min|mins|second|seconds|minute|minutes|h|hr|hrs|hour|hours|",
    r"d|day|days|wk|wks|week|weeks|mo|month|months|yr|yrs|year|years|",
    r"%|px|em|rem|pt|dpi|rpm|",
    r"KiB|MiB|GiB|TiB|PiB|kB|MB|GB|TB|PB|",
    r"mm|cm|km|in|ft|yd|mi|",
    r"mg|kg|lb|lbs|oz|",
    r"mL|L|gal|",
    r"deg|degC|degF|rad|",
    r"req/s|ops/s|qps|rps|",
    r"USD|EUR|GBP",
    r")(?:\^?-?\d)?"
);

pub(crate) static WORDLIKE: LazyLock<Regex> = LazyLock::new(|| re(r"[\p{L}\p{Nd}]"));
static WORD_CHAR: LazyLock<Regex> = LazyLock::new(|| re(r"\A[\p{L}\p{Nd}\p{M}]\z"));
static MARK: LazyLock<Regex> = LazyLock::new(|| re(r"\A\p{M}\z"));

pub(crate) static CODE_SPAN: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?<!`)`{1,}(?P<body>[^`\n]*?)`{1,}(?!`)"));
// Path characters and the characters that may end a path. Both go into the pattern text.
macro_rules! path_class {
    () => {
        r"[\p{L}\p{Nd}_.-]"
    };
}
macro_rules! path_end {
    () => {
        r"[\p{L}\p{Nd}_]"
    };
}

// A path match that fails at one start fails at every start inside the same chain of
// path characters and slashes, because each later start sees a suffix of the same text.
// Without a rule for that, each start rescans the chain, so a long run such as `.....`
// followed by `/` costs quadratic time and no PCRE2 limit stops it. This branch runs
// only when the path match fails. It reads the rest of the chain without backtracking,
// then `(*SKIP)` moves the next start to where the branch stopped. It stops five
// characters short of the end of the last run, so the matcher still tries a `https://` or
// `ftp://` that starts inside the run. The URL alternative has no lookbehind and needs those
// characters.
macro_rules! skip_failed_chain {
    () => {
        concat!(
            r"(?:",
            path_class!(),
            r"+/)*+",
            r"(?:",
            path_class!(),
            r"(?=",
            path_class!(),
            r"{5}))*+",
            r"(*SKIP)(*FAIL)"
        )
    };
}

static URL_OR_PATH_SRC: &str = concat!(
    r"(?:https?://|ftp://)[^\s<>]+|",
    r"(?<!\w)/(?:(?:",
    path_class!(),
    r"+/)+",
    path_class!(),
    r"*",
    path_end!(),
    r"|",
    skip_failed_chain!(),
    r")|",
    r"(?<!\w)(?:(?:\./)?(?:",
    path_class!(),
    r"+/)+",
    path_class!(),
    r"*",
    path_end!(),
    r"|",
    skip_failed_chain!(),
    r")"
);
pub(crate) static URL_OR_PATH: LazyLock<Regex> = LazyLock::new(|| re(URL_OR_PATH_SRC));
static FLAG_OR_ENV: LazyLock<Regex> =
    LazyLock::new(|| re(r"--[A-Za-z][A-Za-z0-9-]*|\$[A-Z][A-Z0-9_]*"));
pub(crate) static IDENTIFIER: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?<!\w)[\p{L}_][\p{L}\p{Nd}_]*(?:(?:\.|::)[\p{L}\p{Nd}_]+)+(?!\w)"));
static MIXED_IDENTIFIER: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"(?<!\w)(?=[A-Za-z0-9_]*[A-Za-z])(?=[A-Za-z0-9_]*\d)",
        r"[A-Za-z][A-Za-z0-9_]*\d[A-Za-z0-9_]*(?!\w)"
    ))
});
pub(crate) static QUOTED_SPAN: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r#"(?<![\w'])"[^"\n]{1,200}"(?!\w)|"#,
        r"(?<![\w'])'[^'\n]{2,200}'(?!\w)|",
        r"(?<!\w)“[^”\n]{1,200}”(?!\w)|(?<!\w)‘[^’\n]{1,200}’(?!\w)"
    ))
});
pub(crate) static PAREN_SPAN: LazyLock<Regex> = LazyLock::new(|| re(r"\([^()\n]{1,200}\)"));

/// A leading step or paragraph number. Rule 8.6: it is not counted.
pub(crate) static STEP_NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    re_i(concat!(
        r"^\s*(?:(?:step\s+\d+(?:\.\d+)*[.:)]?)|",
        r"(?:\([A-Za-z0-9ivxIVX]+\)|(?:[A-Za-z]|[ivxIVX]+|\d+)[.)])|",
        r"(?:\d+(?:\.\d+)+\.?)\s*)(?=\s|$)"
    ))
});

static ABBREVIATION_NUMBER: LazyLock<Regex> =
    LazyLock::new(|| re_i(r"(?<!\w)(?:no|number|fig|figure|sec|section|ref)\.\s+\d+(?!\w)"));
static ABBREVIATION: LazyLock<Regex> = LazyLock::new(|| re(r"(?<!\w)(?:[A-Za-z]\.){2,}(?!\w)"));
static NUMBER_WITH_UNIT: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"(?<!\w){NUMBER}(?:\s+(?:degrees?\s+(?:Celsius|Fahrenheit)|degrees?))?(?:\s+(?:{UNIT}))?(?!\w)"
    ))
});
static PROPER_NAME: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"(?<!\w)(?:[A-Z][A-Za-z0-9'’-]*|of|and|for|the)",
        r"(?:\s+(?:[A-Z][A-Za-z0-9'’-]*|of|and|for|the)){1,}(?!\w)"
    ))
});
static CAPITALISED: LazyLock<Regex> = LazyLock::new(|| re(r"\b[A-Z][A-Za-z0-9'’-]*"));

const TOKEN_JOINERS: [char; 9] = ['\'', '’', 'ʼ', '＇', '-', '‐', '‑', '﹣', '－'];

fn is_word(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphanumeric();
    }
    let mut buf = [0u8; 4];
    WORD_CHAR.is_match(c.encode_utf8(&mut buf)).unwrap_or(false)
}

fn is_mark(c: char) -> bool {
    if c.is_ascii() {
        return false;
    }
    let mut buf = [0u8; 4];
    MARK.is_match(c.encode_utf8(&mut buf)).unwrap_or(false)
}

fn collapse(text: &str, patterns: &[&Regex]) -> Result<String, TextError> {
    let mut text = text.to_owned();
    for pattern in patterns {
        text = replace_all(pattern, &text, |_, _| Ok(format!(" {SENTINEL} ")))?;
    }
    Ok(text)
}

/// Phase 4: a run of two or more capitalised tokens is one word. The first two
/// capitalised tokens of the text are not a name, because sentence case explains them.
fn collapse_proper_names(text: &str) -> Result<String, TextError> {
    replace_all(&PROPER_NAME, text, |start, end| {
        let matched = &text[start..end];
        let first = lower(
            matched
                .split(py_isspace)
                .find(|part| !part.is_empty())
                .unwrap_or(""),
        );
        if IMPERATIVE_VERBS.contains(&first.as_str()) {
            return Ok(matched.to_owned());
        }
        let capitalised = find_all(&CAPITALISED, matched)?.len();
        if capitalised < 2 || (capitalised == 2 && py_strip(&text[..start]).is_empty()) {
            return Ok(matched.to_owned());
        }
        Ok(format!(" {SENTINEL} "))
    })
}

/// Remove the leading step number, once.
pub(crate) fn strip_step_number(text: &str) -> Result<&str, TextError> {
    Ok(match STEP_NUMBER.find(text)? {
        Some(m) => &text[m.end..],
        None => text,
    })
}

/// The canonical token stream for `text`.
pub fn tokens(text: &str) -> Result<Vec<String>, TextError> {
    stepless_tokens(strip_step_number(text)?)
}

pub(crate) fn stepless_tokens(text: &str) -> Result<Vec<String>, TextError> {
    let text = collapse(
        text,
        &[
            &CODE_SPAN,
            &URL_OR_PATH,
            &FLAG_OR_ENV,
            &IDENTIFIER,
            &MIXED_IDENTIFIER,
        ],
    )?;
    let text = collapse(&text, &[&ABBREVIATION_NUMBER, &QUOTED_SPAN])?;
    let text = collapse_proper_names(&text)?;
    let text = collapse(&text, &[&PAREN_SPAN])?;
    let text = collapse(&text, &[&NUMBER_WITH_UNIT, &ABBREVIATION])?;
    Ok(split_words(&text))
}

/// The words of `text` with no collapsing: the same character rules that end the token
/// stream. A joiner (hyphen or apostrophe) stays only with a word character directly on
/// both sides, so `well-known` is one word and `a--b` is two.
#[must_use]
pub fn joined_words(text: &str) -> Vec<String> {
    split_words(text)
        .into_iter()
        .filter(|w| !w.starts_with(SENTINEL))
        .collect()
}

fn split_words(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut current = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c == SENTINEL {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            out.push(SENTINEL.to_string());
            continue;
        }
        if is_word(c) && (!is_mark(c) || !current.is_empty()) {
            current.push(c);
            continue;
        }
        if TOKEN_JOINERS.contains(&c) {
            let prev = i.checked_sub(1).map(|p| chars[p]);
            let next = chars.get(i + 1).copied();
            if !current.is_empty() && prev.is_some_and(is_word) && next.is_some_and(is_word) {
                current.push(c);
                continue;
            }
        }
        if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Count words per the ordered phases in `docs/metrics.md`.
pub fn count_words(text: &str) -> Result<usize, TextError> {
    Ok(tokens(text)?.len())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;
    use slopvac_pcre2::{Limits, Options, Regex};

    use super::{URL_OR_PATH, URL_OR_PATH_SRC, find_all, tokens};
    use crate::rewrite::compat;

    /// The pattern before it learned to skip a failed chain. It is the reference for
    /// "the same matches".
    const OLD_SRC: &str = concat!(
        r"(?:https?://|ftp://)[^\s<>]+|(?<!\w)/(?:[\p{L}\p{Nd}_.-]+/)+[\p{L}\p{Nd}_.-]*[\p{L}\p{Nd}_]|",
        r"(?<!\w)(?:\./)?(?:[\p{L}\p{Nd}_.-]+/)+[\p{L}\p{Nd}_.-]*[\p{L}\p{Nd}_]"
    );

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn compile(src: &str) -> Result<Regex, slopvac_pcre2::CompileError> {
        Regex::with_limits(&compat(src), Options::new().ucp(true), Limits::POLICY)
    }

    fn matched<'a>(text: &'a str, ranges: &[(usize, usize)]) -> Vec<&'a str> {
        ranges.iter().map(|&(s, e)| &text[s..e]).collect()
    }

    #[rstest]
    #[case::url("see https://example.com/a?b=1 now", &["https://example.com/a?b=1"])]
    #[case::ftp("get ftp://host/file now", &["ftp://host/file"])]
    #[case::absolute("open /usr/local/bin now", &["/usr/local/bin"])]
    #[case::dot_relative("edit ./src/main.rs please", &["./src/main.rs"])]
    #[case::relative("edit src/lib.rs", &["src/lib.rs"])]
    #[case::alternatives("and/or", &["and/or"])]
    #[case::trailing_slash_dropped("see src/lib/ now", &["src/lib"])]
    #[case::url_inside_failed_chain("a.http://x", &["http://x"])]
    #[case::chain_without_word_character("./././", &[])]
    #[case::start_after_mark("e\u{301}.foo/bar", &["foo/bar"])]
    #[case::double_slash_restarts("a//b/c", &["/b/c"])]
    #[case::plain_words("no paths here", &[])]
    fn url_or_path_matches(#[case] text: &str, #[case] expected: &[&str]) -> TestResult {
        let ranges = find_all(&URL_OR_PATH, text)?;
        assert_eq!(matched(text, &ranges), expected);
        Ok(())
    }

    const PIECES: &[&str] = &[
        "a", "Z", "1", "_", ".", "-", "/", "//", "./", "http", "https", "ftp", ":", "://", " ",
        "<", "\u{301}", "é", "x",
    ];

    proptest! {
        #[test]
        fn new_pattern_matches_what_the_old_one_did(
            pieces in proptest::collection::vec(proptest::sample::select(PIECES), 0..40)
        ) {
            let text = pieces.concat();
            let old = compile(OLD_SRC)?.find_all(&text)?;
            let new = compile(URL_OR_PATH_SRC)?.find_all(&text)?;
            prop_assert_eq!(old, new, "text {:?}", text);
        }
    }

    /// Scaling guard. The old pattern spent its time scanning, outside PCRE2's counted
    /// match calls, so the match limit stayed silent. `slopvac-pcre2` has no call
    /// counter, so the guard is a size. The old pattern took 0.33 s at 20 000 characters
    /// on a release build and four times that at 40 000, which is many minutes at this
    /// length. The fixed pattern finishes in well under a second. A regression shows as a
    /// test that nextest terminates for running too long. No assertion reads a clock.
    const N: usize = 1_000_000;

    #[rstest]
    #[case::dots(".".repeat(N) + "/")]
    #[case::letter_dot_pairs("a.".repeat(N / 2) + "/")]
    #[case::dot_slash_pairs("./".repeat(N / 2))]
    #[case::absolute_dot_slash_pairs(format!("/{}", "./".repeat(N / 2)))]
    #[case::hyphen_run_before_url("-".repeat(N) + "http://x")]
    fn long_punctuation_runs_finish(#[case] text: String) -> TestResult {
        find_all(&URL_OR_PATH, &text)?;
        Ok(())
    }

    #[test]
    fn tokenizer_counts_a_long_run_as_plain_text() -> TestResult {
        let text = ".".repeat(N) + "/";
        assert_eq!(tokens(&text)?, Vec::<String>::new());
        Ok(())
    }
}
