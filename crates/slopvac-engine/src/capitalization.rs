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

use slopvac_rules::{CapitalizationMatch, CapitalizationStyle};

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
    pub(crate) fn find(&self, text: &str) -> Option<Range<usize>> {
        if text.starts_with([' ', '\t']) {
            return None;
        }
        let tokens: Vec<&str> = text.split([' ', '\t']).filter(|t| !t.is_empty()).collect();
        let mut capitalised = 0;
        for token in &tokens {
            if shape(token)? == Shape::Capitalised {
                capitalised += 1;
            }
        }
        let plain_after_first = tokens.iter().skip(1).any(|t| self.is_plain(t));
        (capitalised >= 2 && plain_after_first).then_some(0..text.len())
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

#[derive(PartialEq, Eq)]
enum Shape {
    /// Starts with a capital letter: a word, an acronym or an identifier.
    Capitalised,
    /// A small word, a number, code or punctuation.
    Other,
}

/// The shape of a token, or `None` when it is not part of a title-case run.
fn shape(token: &str) -> Option<Shape> {
    // The word may be followed by punctuation, so every split of the token is a candidate.
    (1..=token.len())
        .filter(|&at| token.is_char_boundary(at))
        .filter(|&at| token[at..].chars().all(|c| TRAILING.contains(&c)))
        .find_map(|at| body(&token[..at]))
}

fn body(word: &str) -> Option<Shape> {
    if is_name(word) {
        Some(Shape::Capitalised)
    } else if is_number(word)
        || SMALL_WORDS.contains(&word)
        || is_code(word)
        || (word.len() == 1 && word.starts_with(LONE))
    {
        Some(Shape::Other)
    } else {
        None
    }
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

/// `\d+(?:\.\d+)*`
fn is_number(word: &str) -> bool {
    word.split('.')
        .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// A code span that kept its backticks.
fn is_code(word: &str) -> bool {
    word.len() > 2
        && word.starts_with('`')
        && word.ends_with('`')
        && !word[1..word.len() - 1].contains(['`', '\n'])
}
