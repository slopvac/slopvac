//! Compile a rule's `match` block into a PCRE2 matcher.
//!
//! The 2.x Python engine is the reference (`engine.rule_pattern`, `_bounded`,
//! `build_substitution_pattern`): the shared boundaries `(?<![\w-])…(?![\w-])` wrap token lists
//! and the substitution keys that carry none of their own, and keys keep 2.x's meaning (regex
//! when they hold a metacharacter, a literal otherwise).

use std::cmp::Reverse;
use std::ops::Range;

use slopvac_model::NonEmptyString;
use slopvac_text::{Pattern, TextError};

use crate::model::{Match, MatchKind, PatternMatch, SubstitutionMatch, TokensMatch};

/// Values the loader substitutes into patterns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompileOptions {
    /// The locale allow list behind the `allowed-words` placeholder source.
    pub allowed_words: Vec<String>,
}

/// A lexical rule's pattern did not compile.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message} (pattern: {source_text})")]
pub struct CompileError {
    message: String,
    source_text: String,
}

/// The compiled form of a `tokens`, `pattern` or `substitution` match.
#[derive(Debug)]
pub struct LexicalMatcher {
    kind: MatchKind,
    source: String,
    pattern: Pattern,
    ignore_case: bool,
    match_all_caps: bool,
}

impl LexicalMatcher {
    /// Which of the three lexical kinds this came from.
    #[must_use]
    pub const fn kind(&self) -> MatchKind {
        self.kind
    }

    /// The PCRE2 source that was compiled, after boundaries and placeholders.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Size in bytes of the compiled pattern, as PCRE2 reports it.
    #[must_use]
    pub fn compiled_size(&self) -> usize {
        self.pattern.compiled_size()
    }

    /// Whether matching ignores case.
    #[must_use]
    pub const fn ignore_case(&self) -> bool {
        self.ignore_case
    }

    /// Whether a match written entirely in capitals still counts.
    #[must_use]
    pub const fn match_all_caps(&self) -> bool {
        self.match_all_caps
    }

    /// Byte ranges of every non-overlapping match, left to right.
    ///
    /// # Errors
    /// A match limit was exhausted.
    pub fn find_all(&self, text: &str) -> Result<Vec<Range<usize>>, TextError> {
        self.pattern.find_all(text)
    }

    /// Whether the pattern matches anywhere in `text`.
    ///
    /// # Errors
    /// A match limit was exhausted.
    pub fn is_match(&self, text: &str) -> Result<bool, TextError> {
        self.pattern.is_match(text)
    }
}

/// A lexical match with its pattern text built and nothing compiled yet.
pub(crate) struct Planned {
    kind: MatchKind,
    source: String,
    ignore_case: bool,
    match_all_caps: bool,
}

impl Planned {
    /// Bytes of the pattern that [`Planned::compile`] will compile.
    pub(crate) fn source_len(&self) -> usize {
        self.source.len()
    }

    /// Compile the pattern.
    pub(crate) fn compile(self) -> Result<LexicalMatcher, CompileError> {
        let Self {
            kind,
            source,
            ignore_case,
            match_all_caps,
        } = self;
        let pattern = Pattern::new(&source, ignore_case).map_err(|e| CompileError {
            message: e.to_string(),
            source_text: source.clone(),
        })?;
        Ok(LexicalMatcher {
            kind,
            source,
            pattern,
            ignore_case,
            match_all_caps,
        })
    }
}

/// Compile the lexical kinds; every other kind has nothing to compile.
#[cfg(test)]
pub(crate) fn compile(
    m: &Match,
    options: &CompileOptions,
) -> Result<Option<LexicalMatcher>, CompileError> {
    plan(m, options).map(Planned::compile).transpose()
}

/// Build the pattern text of the lexical kinds; every other kind has nothing to compile.
pub(crate) fn plan(m: &Match, options: &CompileOptions) -> Option<Planned> {
    let (kind, source, ignore_case, match_all_caps) = match m {
        Match::Tokens(t) => (
            MatchKind::Tokens,
            tokens_source(t),
            t.ignore_case,
            t.match_all_caps,
        ),
        Match::Pattern(p) => (
            MatchKind::Pattern,
            pattern_source(p, options),
            p.ignore_case,
            p.match_all_caps,
        ),
        Match::Substitution(s) => (
            MatchKind::Substitution,
            substitution_source(s),
            s.ignore_case,
            s.match_all_caps,
        ),
        _ => return None,
    };
    Some(Planned {
        kind,
        source,
        ignore_case,
        match_all_caps,
    })
}

/// Escape every ASCII character that is not a letter, digit or underscore.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        if c.is_ascii() && !c.is_ascii_alphanumeric() && c != '_' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// One word-edge pair around a group of alternatives.
fn bounded(alternatives: &[String]) -> String {
    format!(r"(?<![\w-])(?:{})(?![\w-])", alternatives.join("|"))
}

fn tokens_source(t: &TokensMatch) -> String {
    let mut tokens: Vec<&str> = t.tokens.iter().map(NonEmptyString::as_str).collect();
    // Longest first, so "in order to" wins over "order". The sort is stable, as Python's is.
    tokens.sort_by_key(|s| Reverse(s.chars().count()));
    let alternatives: Vec<String> = tokens.into_iter().map(escape).collect();
    bounded(&alternatives)
}

fn has_own_boundary(key: &str) -> bool {
    [r"\b", r"\B", "(?<", "(?=", "(?!"]
        .iter()
        .any(|m| key.contains(m))
        || key.starts_with('^')
        || key.ends_with('$')
}

fn has_metacharacter(key: &str) -> bool {
    key.contains([
        '\\', '(', ')', '{', '}', '[', ']', '|', '?', '*', '+', '^', '$',
    ])
}

/// One alternation over every key, longest first. Keys that need the default word-edge
/// guard share one pair per run; a key with its own boundary ends the run, so the
/// longest-first order is kept.
fn substitution_source(s: &SubstitutionMatch) -> String {
    let mut keys: Vec<&str> = s
        .substitutions
        .entries()
        .iter()
        .map(|(k, _)| k.as_str())
        .collect();
    keys.sort_by_key(|k| Reverse(k.chars().count()));
    let mut parts: Vec<String> = Vec::new();
    let mut run: Vec<String> = Vec::new();
    for key in keys {
        if has_own_boundary(key) {
            if !run.is_empty() {
                parts.push(bounded(&std::mem::take(&mut run)));
            }
            parts.push(format!("(?:{key})"));
        } else if has_metacharacter(key) {
            run.push(format!("(?:{key})"));
        } else {
            run.push(escape(key));
        }
    }
    if !run.is_empty() {
        parts.push(bounded(&run));
    }
    parts.join("|")
}

/// Replace each placeholder with an alternation of the allowed words: single ASCII alphabetic
/// words, escaped, sorted without regard to case. With none, the placeholder becomes `(?!)`.
fn pattern_source(p: &PatternMatch, options: &CompileOptions) -> String {
    let mut source = p.pattern.as_str().to_owned();
    let Some(placeholders) = &p.placeholders else {
        return source;
    };
    let mut words: Vec<&str> = options
        .allowed_words
        .iter()
        .map(String::as_str)
        .filter(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_alphabetic()))
        .collect();
    words.sort_by_key(|w| w.to_ascii_lowercase());
    words.dedup();
    let alternation = if words.is_empty() {
        "(?!)".to_owned()
    } else {
        words.into_iter().map(escape).collect::<Vec<_>>().join("|")
    };
    for name in placeholders.keys() {
        source = source.replace(name.as_str(), &alternation);
    }
    source
}

#[cfg(test)]
mod tests {
    #![allow(clippy::single_range_in_vec_init)]

    use super::*;
    use serde_json::json;

    fn matcher(m: serde_json::Value) -> LexicalMatcher {
        let m: Match = serde_json::from_value(m).unwrap();
        compile(&m, &CompileOptions::default()).unwrap().unwrap()
    }

    fn spans<'a>(m: &LexicalMatcher, text: &'a str) -> Vec<&'a str> {
        m.find_all(text)
            .unwrap()
            .into_iter()
            .map(|r| &text[r])
            .collect()
    }

    #[test]
    fn tokens_match_whole_words_only() {
        let m = matcher(json!({"kind": "tokens", "tokens": ["order", "in order to"]}));
        assert_eq!(
            spans(&m, "In order to sort, order it; reorder, in-order."),
            ["In order to", "order"]
        );
    }

    #[test]
    fn tokens_keep_their_punctuation_literal() {
        let m = matcher(json!({"kind": "tokens", "tokens": ["a.b", "c+d"]}));
        assert_eq!(spans(&m, "a.b axb c+d ccd"), ["a.b", "c+d"]);
    }

    #[test]
    fn substitution_keys_with_dots_are_literal_and_bounded() {
        let m = matcher(json!({"kind": "substitution", "substitutions": {
            "i.e.": "that is", "e.g.": "for example", "whitelist": "allowlist"
        }}));
        // "iXeX" would match if the dots were wildcards; "whitelisted" and "anti-whitelist"
        // would match without the word-and-hyphen edges.
        assert_eq!(
            spans(
                &m,
                "Use i.e. or e.g. here, not iXeX, whitelisted or anti-whitelist, but a whitelist."
            ),
            ["i.e.", "e.g.", "whitelist"]
        );
    }

    #[test]
    fn substitution_keys_that_are_regex_are_regex() {
        let m = matcher(json!({"kind": "substitution", "substitutions": {
            "white-?list(?:ed)?": "allow"
        }}));
        assert_eq!(
            spans(&m, "whitelisted and white-list and whitelists"),
            ["whitelisted", "white-list"]
        );
    }

    #[test]
    fn a_key_with_its_own_boundary_gets_no_shared_edge() {
        let m = matcher(json!({"kind": "substitution", "substitutions": {
            r"\bfeatures(?=\s+the)": "has", "plain": "p"
        }}));
        assert_eq!(
            spans(&m, "It features the car; plainly plain"),
            ["features", "plain"]
        );
    }

    #[test]
    fn a_longer_substitution_key_wins_over_a_prefix() {
        let m = matcher(json!({"kind": "substitution", "substitutions": {
            "order": "o", "in order to": "to"
        }}));
        assert_eq!(spans(&m, "in order to"), ["in order to"]);
    }

    #[test]
    fn ignore_case_is_on_unless_turned_off() {
        let on = matcher(json!({"kind": "tokens", "tokens": ["Foo"]}));
        let off = matcher(json!({"kind": "tokens", "tokens": ["Foo"], "ignore_case": false}));
        assert_eq!(spans(&on, "foo FOO"), ["foo", "FOO"]);
        assert_eq!(spans(&off, "foo FOO Foo"), ["Foo"]);
    }

    #[test]
    fn a_placeholder_becomes_the_allowed_words_sorted_without_case() {
        let m: Match = serde_json::from_value(json!({
            "kind": "pattern", "pattern": r"\b(?!(?:ALLOWED)\b)[A-Z][a-z]+",
            "placeholders": {"ALLOWED": "allowed-words"}, "ignore_case": false
        }))
        .unwrap();
        let words = ["Zeta", "Alpha", "not a word", "Beta1", "beta"];
        let options = CompileOptions {
            allowed_words: words.map(String::from).to_vec(),
        };
        let c = compile(&m, &options).unwrap().unwrap();
        assert!(
            c.source().contains(r"(?!(?:Alpha|beta|Zeta)\b)"),
            "{}",
            c.source()
        );
        // "Beta" is not exempt: only the lowercase "beta" is on the list.
        assert_eq!(
            c.find_all("Alpha Gamma Zeta Beta").unwrap(),
            [6..11, 17..21]
        );
    }

    #[test]
    fn a_placeholder_without_words_never_matches() {
        let m: Match = serde_json::from_value(json!({
            "kind": "pattern", "pattern": r"(?!ALLOWED)Foo",
            "placeholders": {"ALLOWED": "allowed-words"}
        }))
        .unwrap();
        let c = compile(&m, &CompileOptions::default()).unwrap().unwrap();
        assert_eq!(c.find_all("Foo").unwrap(), [0..3]);
    }

    #[test]
    fn a_pattern_that_does_not_compile_is_an_error() {
        let m: Match = serde_json::from_value(json!({"kind": "pattern", "pattern": "(a"})).unwrap();
        assert!(compile(&m, &CompileOptions::default()).is_err());
    }

    #[test]
    fn a_lookbehind_without_a_bound_is_rejected() {
        let m: Match =
            serde_json::from_value(json!({"kind": "pattern", "pattern": "(?<=a+)b"})).unwrap();
        assert!(compile(&m, &CompileOptions::default()).is_err());
    }

    #[test]
    fn other_kinds_have_no_matcher() {
        let m: Match =
            serde_json::from_value(json!({"kind": "judgement", "question": "q?"})).unwrap();
        assert!(compile(&m, &CompileOptions::default()).unwrap().is_none());
    }
}
