//! The `capitalization` rule kind. Port of the 2.x `title-case-heading` logic (issue #257).
//!
//! In the sentence style a text is reported when it is written in title case: it reads as a
//! run of capitalised words, small words, numbers and punctuation, and holds at least one
//! plain capitalised word (`Server`) after the first word. A word that is an acronym (`SDK`),
//! CamelCase (`EventLog`), mixed case (`gRPC`, `macOS`), holds a digit (`Python3`) or is inline
//! code does not count, and neither does a word on the allowed list. A text with any other
//! kind of word is not a title-case run, so nothing is reported.
//!
//! The 2.x rule was one pattern. The grammar below is that pattern read token by token.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::LazyLock;

use slopvac_rules::{CapitalizationMatch, CapitalizationStyle};
use slopvac_text::{Pattern, TextError};

/// Words a title leaves in lower case.
const SMALL_WORDS: &[&str] = &[
    "a", "an", "and", "as", "at", "but", "by", "for", "from", "in", "into", "nor", "of", "on",
    "or", "the", "to", "up", "via", "with", "without", "your", "you", "is", "are", "was", "were",
    "than",
];

/// Punctuation that may follow a word.
const TRAILING: &[char] = &[',', ':', ';', '.', '!', '?', ')'];
/// Punctuation that may stand alone as a token.
const LONE: &[char] = &[',', ':', ';', '.', '!', '?', '&', '(', ')', '/', '+', '-'];

/// Whether the engine runs a capitalization style.
pub(crate) const fn runs(style: CapitalizationStyle) -> bool {
    matches!(style, CapitalizationStyle::Sentence)
}

/// A sentence-case check with its allowed words.
pub(crate) struct SentenceCase {
    /// Lower-cased words that do not count as capitalised.
    allowed: HashSet<String>,
}

impl SentenceCase {
    /// A check that exempts the rule's own allowed words and `extra`, ignoring case.
    pub(crate) fn new(rule: &CapitalizationMatch, extra: &[String]) -> Self {
        let own = rule
            .allowed_words
            .iter()
            .flat_map(|words| words.iter().map(slopvac_model::NonEmptyString::as_str));
        let allowed = own
            .chain(extra.iter().map(String::as_str))
            .map(str::to_lowercase)
            .collect();
        Self { allowed }
    }

    /// The range of `text` to report: all of it when it is a title-case run.
    ///
    /// # Errors
    /// A pattern exhausted its match limit.
    pub(crate) fn find(&self, text: &str) -> Result<Option<Range<usize>>, TextError> {
        if text.starts_with([' ', '\t']) || text.contains('\n') {
            return Ok(None);
        }
        for token in grammar_tokens(text) {
            let Some(token) = token else {
                return Ok(None);
            };
            if !is_run_token(token)? {
                return Ok(None);
            }
        }
        // The regex counts capitalised words anywhere in the text, code spans included.
        if NAME.find_all(text)?.len() < 2 {
            return Ok(None);
        }
        let plain_after_first = text
            .split([' ', '\t'])
            .filter(|t| !t.is_empty())
            .skip(1)
            .any(|t| self.is_plain(t));
        Ok(plain_after_first.then_some(0..text.len()))
    }

    /// Whether the token is `Word` with optional trailing punctuation, and not allowed.
    fn is_plain(&self, token: &str) -> bool {
        let word = token.trim_end_matches(TRAILING);
        let mut chars = word.chars();
        chars.next().is_some_and(|c| c.is_ascii_uppercase())
            && word.len() > 1
            && chars.all(|c| c.is_ascii_lowercase())
            && !self.allowed.contains(&word.to_lowercase())
    }
}

/// `[A-Z][A-Za-z0-9]*(?:[-'’][A-Za-z0-9]+)*` as a word.
static NAME: LazyLock<Pattern> = LazyLock::new(|| {
    Pattern::new(r"\b[A-Z][A-Za-z0-9]*(?:[-'’][A-Za-z0-9]+)*\b", false)
        .expect("the name pattern compiles")
});
/// A number, with Unicode decimal digits as in the 2.x regex.
static NUMBER: LazyLock<Pattern> = LazyLock::new(|| {
    Pattern::new(r"\A\d+(?:\.\d+)*\z", false).expect("the number pattern compiles")
});

/// The tokens of the run, split at spaces and tabs except inside a backtick span. A token
/// that cannot be cut, such as an unclosed span, comes out as `None`.
fn grammar_tokens(text: &str) -> impl Iterator<Item = Option<&str>> {
    let mut rest = text;
    std::iter::from_fn(move || {
        rest = rest.trim_start_matches([' ', '\t']);
        if rest.is_empty() {
            return None;
        }
        let mut end = if let Some(inner) = rest.strip_prefix('`') {
            // The span ends at the next backtick and holds at least one character.
            match inner.find('`') {
                Some(close) if close > 0 => close + 2,
                _ => {
                    rest = "";
                    return Some(None);
                }
            }
        } else {
            0
        };
        end += rest[end..].find([' ', '\t']).unwrap_or(rest.len() - end);
        let (token, tail) = rest.split_at(end);
        rest = tail;
        Some(Some(token))
    })
}

/// Whether the token is a word, small word, number, code span or punctuation mark, with
/// optional trailing punctuation.
fn is_run_token(token: &str) -> Result<bool, TextError> {
    // The word may be followed by punctuation, so every split of the token is a candidate.
    for at in (1..=token.len()).filter(|&at| token.is_char_boundary(at)) {
        if token[at..].chars().all(|c| TRAILING.contains(&c)) && is_body(&token[..at])? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_body(word: &str) -> Result<bool, TextError> {
    Ok(is_name(word)
        || SMALL_WORDS.contains(&word)
        || is_code(word)
        || (word.len() == 1 && word.starts_with(LONE))
        || NUMBER.is_match(word)?)
}

/// `[A-Z][A-Za-z0-9]*(?:[-'’][A-Za-z0-9]+)*`
fn is_name(word: &str) -> bool {
    let mut chars = word.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_uppercase()) {
        return false;
    }
    let mut after_joiner = false;
    for c in chars {
        if c.is_ascii_alphanumeric() {
            after_joiner = false;
        } else if matches!(c, '-' | '\'' | '\u{2019}') && !after_joiner {
            after_joiner = true;
        } else {
            return false;
        }
    }
    !after_joiner
}

/// A code span: backticks around at least one character that is not a backtick.
fn is_code(word: &str) -> bool {
    word.len() > 2
        && word.starts_with('`')
        && word.ends_with('`')
        && !word[1..word.len() - 1].contains('`')
}
