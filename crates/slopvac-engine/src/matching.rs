//! Helpers for lexical matches: allowlist and all-caps tests, message interpolation and the
//! replacement a substitution rule names. Ports of `slopvac.engine`.

use std::sync::{LazyLock, OnceLock};

use slopvac_rules::LoadedRule;
use slopvac_text::Pattern;

/// Function words that a pattern captures as context around the word it judges.
const SCAFFOLD_WORDS: &[&str] = &[
    "is", "are", "was", "were", "be", "been", "being", "am", "the", "a", "an", "this", "that",
    "these", "those", "and", "or", "not", "to", "of", "in", "on", "for", "with", "as", "at", "it",
    "its", "if", "then", "by", "from",
];

/// Two capitals at least and no lower-case letter.
fn shouts(word: &str) -> bool {
    !word.bytes().any(|b| b.is_ascii_lowercase())
        && word.bytes().filter(u8::is_ascii_uppercase).count() >= 2
}

/// Whether a match is written in capitals.
///
/// An all-caps token in technical prose is a normative keyword (`MUST`), an identifier
/// (`DATABASE_URL`), an initialism or a safety marker. No prose rule is about those, so a
/// match like this is exempt unless the rule sets `match_all_caps`. A short match is exempt
/// when its capitalised word carries the rule and the rest is function-word scaffolding.
pub(crate) fn is_all_caps(matched: &str) -> bool {
    if shouts(matched) {
        return true;
    }
    let mut words: Vec<&str> = Vec::new();
    let mut rest = matched;
    while let Some(at) = rest.find(|c: char| c.is_ascii_alphabetic()) {
        let tail = &rest[at..];
        let len = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')))
            .unwrap_or(tail.len());
        words.push(&tail[..len]);
        rest = &tail[len..];
    }
    if words.is_empty() || words.len() > 4 || !words.iter().any(|w| shouts(w)) {
        return false;
    }
    words
        .iter()
        .all(|w| SCAFFOLD_WORDS.contains(w) || shouts(w))
}

/// Whether the match is an allowlist entry, or lies inside an occurrence of a multi-word
/// entry in `text`.
pub(crate) fn allowlisted(rule: &LoadedRule, text: &str, range: &std::ops::Range<usize>) -> bool {
    let allowlist = rule.rule().allowlist();
    if allowlist.is_empty() {
        return false;
    }
    let matched = text[range.clone()].to_lowercase();
    if allowlist
        .iter()
        .any(|a| a.as_str().to_lowercase() == matched)
    {
        return true;
    }
    // Lower-case one character at a time and keep, for each byte of the result, the byte of
    // `text` it came from. A character whose lower case has another length (U+0130) then
    // shifts nothing else.
    let mut per_char = String::with_capacity(text.len());
    let mut origin: Vec<usize> = Vec::with_capacity(text.len() + 1);
    for (at, c) in text.char_indices() {
        let before = per_char.len();
        per_char.extend(c.to_lowercase());
        origin.resize(origin.len() + per_char.len() - before, at);
    }
    origin.push(text.len());
    // Whole-string lower case is what the allowlist entries went through, and it picks the
    // final sigma by context. It only changes a sigma into another two-byte letter, so the
    // byte map still holds; if it ever did not, the per-character text is the safe answer.
    let whole = text.to_lowercase();
    let lowered = if whole.len() == per_char.len() {
        whole
    } else {
        per_char
    };
    allowlist
        .iter()
        .map(|a| a.as_str().to_lowercase())
        .filter(|a| a.contains(' '))
        .any(|phrase| {
            lowered.match_indices(&phrase).any(|(at, found)| {
                origin[at] <= range.start && origin[at + found.len()] >= range.end
            })
        })
}

/// Fill `{match}` and `{replacement}` in a rule message.
///
/// Messages are data, and their placeholders drifted: some say `{value}` and `{limit}`.
/// Both vocabularies resolve for both kinds of rule. An unknown placeholder stays as
/// written. A stray brace returns the template unchanged, so a cosmetic mismatch never
/// kills a run.
pub(crate) fn format_message(template: &str, matched: &str, replacement: &str) -> String {
    let fields: [(&str, &str); 8] = [
        ("match", matched),
        ("value", matched),
        ("count", matched),
        ("actual", matched),
        ("replacement", replacement),
        ("limit", replacement),
        ("max", replacement),
        ("threshold", replacement),
    ];
    let mut out = String::with_capacity(template.len());
    let mut chars = template.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '{' if chars.peek().is_some_and(|&(_, n)| n == '{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek().is_some_and(|&(_, n)| n == '}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let Some(close) = template[i + 1..].find('}') else {
                    return template.to_owned();
                };
                let name = &template[i + 1..i + 1 + close];
                if name.contains('{') {
                    return template.to_owned();
                }
                if let Some((_, v)) = fields.iter().find(|(k, _)| *k == name) {
                    out.push_str(v);
                } else {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                }
                while chars.next_if(|&(j, _)| j <= i + 1 + close).is_some() {}
            }
            '}' => return template.to_owned(),
            _ => out.push(c),
        }
    }
    out
}

/// Whether a replacement names one safe automatic edit.
pub(crate) fn is_single_replacement(replacement: &str) -> bool {
    !replacement.is_empty() && !replacement.contains([',', '|'])
}

// A trailing lookahead cannot be satisfied by the matched span alone: the key
// `\b(?:e\.g\.)(?=[\s,])` matches "e.g." inside a sentence but not the bare string, because
// the space it looks ahead for is outside the span. Stripping the assertions makes the
// reverse lookup work. One level of grouping inside an assertion is allowed.
const ASSERTION_BODY: &str = r"(?:\?=|\?!|\?<=|\?<!)(?:[^()]|\([^()]*\))*";
static TRAILING_ASSERTION: LazyLock<Pattern> = LazyLock::new(|| {
    Pattern::new(&format!(r"\({ASSERTION_BODY}\)\z"), false).expect("assertion pattern compiles")
});
static LEADING_ASSERTION: LazyLock<Pattern> = LazyLock::new(|| {
    Pattern::new(&format!(r"\A\({ASSERTION_BODY}\)"), false).expect("assertion pattern compiles")
});

fn strip(pattern: &Pattern, key: &str) -> String {
    match pattern.find_all(key) {
        Ok(found) if !found.is_empty() => {
            let mut out = String::from(key);
            for r in found.iter().rev() {
                out.replace_range(r.clone(), "");
            }
            out
        }
        _ => key.to_owned(),
    }
}

fn matches_somewhere(pattern: Option<&Pattern>, text: &str) -> bool {
    pattern.is_some_and(|p| p.find_all(text).is_ok_and(|found| !found.is_empty()))
}

/// One substitution key, with the patterns that find its replacement from a matched span.
struct Key {
    replacement: String,
    lowered: String,
    /// Matches the whole span.
    full: Option<Pattern>,
    /// The same with the leading and trailing assertions removed; `None` when there are none.
    full_stripped: Option<Pattern>,
    /// Matches somewhere inside the span.
    inside: Option<Pattern>,
}

/// The reverse lookup of a substitution rule: from the text a match found to the
/// replacement its key names. A finding that reports no replacement tells the writer
/// nothing, so every key must be able to name its fix.
pub(crate) struct Replacements {
    /// Keys longest first.
    keys: Vec<Key>,
}

impl Replacements {
    fn new(rule: &LoadedRule) -> Self {
        let slopvac_rules::Match::Substitution(m) = &rule.rule().match_ else {
            return Self { keys: Vec::new() };
        };
        let mut entries: Vec<_> = m.substitutions.entries().iter().collect();
        entries.sort_by_key(|(k, _)| std::cmp::Reverse(k.as_str().chars().count()));
        let keys = entries
            .into_iter()
            .map(|(key, replacement)| {
                let key = key.as_str();
                let stripped = strip(&LEADING_ASSERTION, &strip(&TRAILING_ASSERTION, key));
                let anchored = |probe: &str| Pattern::new(&format!(r"\A(?:{probe})\z"), true).ok();
                Key {
                    replacement: replacement.clone(),
                    lowered: key.to_lowercase(),
                    full: anchored(key),
                    full_stripped: if stripped == key {
                        None
                    } else {
                        anchored(&stripped)
                    },
                    inside: Pattern::new(&stripped, true).ok(),
                }
            })
            .collect();
        Self { keys }
    }

    /// The replacement for `matched`, when some key names one.
    pub(crate) fn find(&self, matched: &str) -> Option<&str> {
        let lowered = matched.to_lowercase();
        if let Some(k) = self.keys.iter().rev().find(|k| k.lowered == lowered) {
            return Some(&k.replacement);
        }
        for stripped in [false, true] {
            for k in &self.keys {
                let pattern = if stripped { &k.full_stripped } else { &k.full };
                if matches_somewhere(pattern.as_ref(), matched) {
                    return Some(&k.replacement);
                }
            }
        }
        self.keys
            .iter()
            .find(|k| matches_somewhere(k.inside.as_ref(), matched))
            .map(|k| k.replacement.as_str())
    }
}

/// Replacements for one rule, built on first use.
#[derive(Default)]
pub(crate) struct LazyReplacements(OnceLock<Replacements>);

impl LazyReplacements {
    pub(crate) fn get(&self, rule: &LoadedRule) -> &Replacements {
        self.0.get_or_init(|| Replacements::new(rule))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_caps_covers_keywords_identifiers_and_scaffolded_matches() {
        for exempt in [
            "MUST",
            "DATABASE_URL",
            "TLS1.3",
            "RFC 2119",
            "is IMPORTANT",
            "ENSURE the",
        ] {
            assert!(is_all_caps(exempt), "{exempt}");
        }
        for reported in [
            "Ensure",
            "A",
            "is important",
            "The IMPORTANT build of release",
            "",
        ] {
            assert!(!is_all_caps(reported), "{reported}");
        }
    }

    #[test]
    fn messages_fill_both_placeholder_vocabularies() {
        assert_eq!(
            format_message("use {replacement} for {match} ({limit}/{value})", "a", "b"),
            "use b for a (b/a)"
        );
    }

    #[test]
    fn messages_keep_unknown_placeholders_and_unescape_doubled_braces() {
        assert_eq!(format_message("{other} {{x}}", "a", "b"), "{other} {x}");
    }

    #[test]
    fn messages_with_a_stray_brace_come_back_unchanged() {
        assert_eq!(
            format_message("close } here {match}", "a", "b"),
            "close } here {match}"
        );
        assert_eq!(format_message("open { here", "a", "b"), "open { here");
    }

    #[test]
    fn a_replacement_with_a_comma_or_pipe_is_not_single() {
        assert!(is_single_replacement("use"));
        assert!(!is_single_replacement("use, or this"));
        assert!(!is_single_replacement("a|b"));
        assert!(!is_single_replacement(""));
    }
}
