//! The term casing check.
//!
//! A term matches text case-insensitively on ASCII, at word boundaries. Text
//! that differs from the canonical spelling and from every declared variant is
//! a finding. Plurals (`GitHubs`) and possessives (`GitHub's`) need no
//! declaration. Matches inside paths, domains, handles and identifiers are
//! left alone, as are matches that overlap a caller-supplied skip range such
//! as a code span.

use std::collections::HashMap;
use std::ops::Range;

use crate::{TermEntry, TermsFile};

/// A match that differs from the canonical spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CasingFinding {
    /// Byte range of the matched term in the text, without any plural or possessive suffix.
    pub span: Range<usize>,
    /// The text found.
    pub found: String,
    /// The canonical spelling.
    pub expected: String,
}

#[derive(Debug, Clone)]
struct Surface {
    text: String,
    term: usize,
}

/// A compiled set of terms.
#[derive(Debug, Clone, Default)]
pub struct TermSet {
    terms: Vec<TermEntry>,
    /// Surfaces by lower-cased first byte, longest first.
    by_first: HashMap<u8, Vec<Surface>>,
}

impl TermSet {
    /// The terms that ship with slopvac. They are versioned with the binary.
    ///
    /// # Panics
    /// Never in a release: a unit test parses the bundled file.
    #[must_use]
    pub fn builtin() -> Self {
        Self::from_file(&TermsFile::builtin())
    }

    /// Compile terms. Later duplicates of a canonical spelling are ignored.
    #[must_use]
    pub fn new(entries: impl IntoIterator<Item = TermEntry>) -> Self {
        let mut terms: Vec<TermEntry> = Vec::new();
        for e in entries {
            if !e.term.is_empty() && !terms.iter().any(|t| t.term == e.term) {
                terms.push(e);
            }
        }
        let mut by_first: HashMap<u8, Vec<Surface>> = HashMap::new();
        for (i, t) in terms.iter().enumerate() {
            for s in std::iter::once(&t.term).chain(t.variants.iter()) {
                if let Some(b) = s.bytes().next() {
                    by_first
                        .entry(b.to_ascii_lowercase())
                        .or_default()
                        .push(Surface {
                            text: s.clone(),
                            term: i,
                        });
                }
            }
        }
        for list in by_first.values_mut() {
            list.sort_by_key(|s| std::cmp::Reverse(s.text.len()));
        }
        Self { terms, by_first }
    }

    /// Compile a parsed terms file.
    #[must_use]
    pub fn from_file(file: &TermsFile) -> Self {
        Self::new(file.terms.iter().cloned())
    }

    /// Number of terms.
    #[must_use]
    pub fn len(&self) -> usize {
        self.terms.len()
    }

    /// Whether the set has no terms.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Every accepted spelling: each canonical term followed by its variants, in file order.
    pub fn spellings(&self) -> impl Iterator<Item = &str> {
        self.terms.iter().flat_map(|t| {
            std::iter::once(t.term.as_str()).chain(t.variants.iter().map(String::as_str))
        })
    }

    /// Find case mismatches in `text`, in text order. Matches that overlap a
    /// range in `skip` are ignored.
    #[must_use]
    pub fn check_casing(&self, text: &str, skip: &[Range<usize>]) -> Vec<CasingFinding> {
        let bytes = text.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while let Some(&byte) = bytes.get(i) {
            if !text.is_char_boundary(i) {
                i += 1;
                continue;
            }
            let Some(candidates) = self.by_first.get(&byte.to_ascii_lowercase()) else {
                i += 1;
                continue;
            };
            let hit = candidates.iter().find(|s| {
                let end = i + s.text.len();
                let Some(window) = bytes.get(i..end) else {
                    return false;
                };
                let case_sensitive = self.terms.get(s.term).is_some_and(|t| t.case_sensitive);
                text.is_char_boundary(end)
                    && if case_sensitive {
                        window == s.text.as_bytes()
                    } else {
                        window.eq_ignore_ascii_case(s.text.as_bytes())
                    }
                    && left_ok(text, i)
                    && right_ok(text, end)
                    && token_ok(text, i, end)
            });
            let Some(s) = hit else {
                i += 1;
                continue;
            };
            let end = i + s.text.len();
            let (Some(term), Some(found)) = (self.terms.get(s.term), text.get(i..end)) else {
                i += 1;
                continue;
            };
            let accepted =
                term.ignore_case || found == term.term || term.variants.iter().any(|v| v == found);
            if !accepted && !skip.iter().any(|r| r.start < end && i < r.end) {
                out.push(CasingFinding {
                    span: i..end,
                    found: found.to_owned(),
                    expected: term.term.clone(),
                });
            }
            i = end;
        }
        out
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn left_ok(text: &str, at: usize) -> bool {
    text.get(..at).is_some_and(|before| {
        before
            .chars()
            .next_back()
            .is_none_or(|c| !is_word(c) && !"/@.\\#$~".contains(c))
    })
}

fn right_ok(text: &str, at: usize) -> bool {
    let Some(rest) = text.get(at..) else {
        return false;
    };
    let rest = strip_suffix_forms(rest);
    let mut chars = rest.chars();
    match chars.next() {
        Some(c) if is_word(c) || "/@\\".contains(c) => false,
        Some('.') => !chars.next().is_some_and(is_word),
        _ => true,
    }
}

/// Skip a plural or possessive ending, ignoring ASCII case, so it does not
/// count as part of the next word. The ending stays outside the reported span.
fn strip_suffix_forms(rest: &str) -> &str {
    for suffix in ["'s", "\u{2019}s", "s'", "s\u{2019}", "es", "s"] {
        if let (Some(head), Some(after)) = (rest.get(..suffix.len()), rest.get(suffix.len()..))
            && head.eq_ignore_ascii_case(suffix)
            && after.chars().next().is_none_or(|c| !is_word(c))
        {
            return after;
        }
    }
    rest
}

/// Reject a match inside a path, URL, domain, e-mail or handle. The whole
/// whitespace-delimited token decides, so `docs/my-github-repo` and
/// `@my-github` are skipped while `github-hosted` still matches.
fn token_ok(text: &str, start: usize, end: usize) -> bool {
    token_check(text, start, end).unwrap_or(false)
}

/// `token_ok` over offsets that may not lie on char boundaries: `None` when they do not.
fn token_check(text: &str, start: usize, end: usize) -> Option<bool> {
    let ts = text
        .get(..start)?
        .rfind(char::is_whitespace)
        .map_or(0, |p| {
            p + text
                .get(p..)
                .and_then(|t| t.chars().next())
                .map_or(1, char::len_utf8)
        });
    let te = text
        .get(end..)?
        .find(char::is_whitespace)
        .map_or(text.len(), |p| end + p);
    let lead = "([{\"'<`\u{2018}\u{201c}\u{00ab}\u{2039}\u{ff08}\u{300c}\u{300e}\u{3010}";
    let trail = ")]}\"'>,;:!?.`\u{2019}\u{201d}\u{00bb}\u{203a}\u{2026}\u{ff09}\u{300d}\u{300f}\u{3011}\u{3002}\u{ff0c}\u{ff01}\u{ff1f}";
    let mut ts = ts;
    while ts < start && text.get(ts..)?.starts_with(|c| lead.contains(c)) {
        ts += text.get(ts..)?.chars().next().map_or(1, char::len_utf8);
    }
    let mut te = te;
    while te > end && text.get(..te)?.ends_with(|c| trail.contains(c)) {
        te -= text
            .get(..te)?
            .chars()
            .next_back()
            .map_or(1, char::len_utf8);
    }
    let token = text.get(ts..te)?;
    let (s, e) = (start - ts, end - ts);
    let outside = |i: usize| i < s || i >= e;
    let chars: Vec<(usize, char)> = token.char_indices().collect();
    for (k, &(i, c)) in chars.iter().enumerate() {
        if !outside(i) {
            continue;
        }
        if "/@\\".contains(c) {
            return Some(false);
        }
        if c == '.' {
            let prev_alnum = k
                .checked_sub(1)
                .and_then(|p| chars.get(p))
                .is_some_and(|x| x.1.is_alphanumeric());
            let next_alnum = chars.get(k + 1).is_some_and(|x| x.1.is_alphanumeric());
            // `.github`, `github.com` and `docs.github` are paths or domains.
            if prev_alnum || next_alnum {
                return Some(false);
            }
        }
    }
    Some(true)
}
