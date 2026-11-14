//! Word counting per ASD-STE100 rules 8.4 to 8.7. A port of the Python reference
//! (`analyze._ste_tokens`); see `docs/metrics.md` for the contract.

use std::sync::LazyLock;

use pcre2::bytes::Regex;

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
static URL_OR_PATH_SRC: &str = concat!(
    r"(?:https?://|ftp://)[^\s<>]+|(?<!\w)/(?:[\p{L}\p{Nd}_.-]+/)+[\p{L}\p{Nd}_.-]*[\p{L}\p{Nd}_]|",
    r"(?<!\w)(?:\./)?(?:[\p{L}\p{Nd}_.-]+/)+[\p{L}\p{Nd}_.-]*[\p{L}\p{Nd}_]"
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
    WORD_CHAR
        .is_match(c.encode_utf8(&mut buf).as_bytes())
        .unwrap_or(false)
}

fn is_mark(c: char) -> bool {
    if c.is_ascii() {
        return false;
    }
    let mut buf = [0u8; 4];
    MARK.is_match(c.encode_utf8(&mut buf).as_bytes())
        .unwrap_or(false)
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
    Ok(match STEP_NUMBER.find(text.as_bytes())? {
        Some(m) => &text[m.end()..],
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
    Ok(out)
}

/// Count words per the ordered phases in `docs/metrics.md`.
pub fn count_words(text: &str) -> Result<usize, TextError> {
    Ok(tokens(text)?.len())
}
