//! Candidate terms inferred from text, for the user to confirm.
//!
//! STUB(slopvac-8l4.8.9): a local heuristic. The real inference is the
//! doc- and repo-level proper-noun inference of rule-propernoun
//! (slopvac-8l4.6.2). A token is a candidate when it is mixed case inside
//! (`GitHub`, `tRPC`), a short all-caps acronym (`SLO`), or a capitalised word
//! that does not start a sentence. Code spans and fences are skipped.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

/// A candidate with the evidence for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// The spelling found.
    pub term: String,
    /// Occurrences across all inputs.
    pub count: usize,
    /// Number of inputs that contain it.
    pub files: usize,
    /// `name:line:col` of the first occurrence.
    pub first: String,
}

fn tokens(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut start: Option<usize> = None;
    let mut chars = text.char_indices();
    let mut done = false;
    std::iter::from_fn(move || {
        loop {
            if done {
                return None;
            }
            match chars.next() {
                Some((i, c)) if c.is_alphanumeric() => {
                    start.get_or_insert(i);
                }
                Some((i, _)) => {
                    if let Some(s) = start.take() {
                        return Some(s..i);
                    }
                }
                None => {
                    done = true;
                    return start.take().map(|s| s..text.len());
                }
            }
        }
    })
}

fn starts_sentence(text: &str, at: usize) -> bool {
    let Some(before) = text.get(..at) else {
        return false;
    };
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let prefix = before.get(line_start..).unwrap_or_default().trim_start();
    let prefix = prefix
        .trim_start_matches(['#', '>', '-', '*', '+'])
        .trim_start();
    let prefix = prefix
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim_start_matches(['.', ')'])
        .trim();
    let prefix = prefix.trim_end_matches(['"', '\'', '(', '*', '_', '[', '\u{201c}', '\u{2018}']);
    prefix.is_empty() || prefix.ends_with(['.', '?', '!', ':'])
}

/// Longest all-caps token that counts as an acronym.
const MAX_ACRONYM: usize = 6;

fn is_candidate(word: &str, text: &str, at: usize) -> bool {
    let chars: Vec<char> = word.chars().collect();
    let letters = chars.iter().filter(|c| c.is_alphabetic()).count();
    if letters < 2 || !chars.iter().any(|c| c.is_alphabetic()) {
        return false;
    }
    let upper = chars.iter().filter(|c| c.is_uppercase()).count();
    let first_upper = chars.first().is_some_and(|c| c.is_uppercase());
    if upper == letters {
        // Short all-caps tokens are acronyms; longer ones are shouting.
        return letters <= MAX_ACRONYM;
    }
    if upper >= 2 || (!first_upper && upper >= 1) {
        return chars.len() >= 3;
    }
    first_upper && letters >= 3 && !starts_sentence(text, at)
}

/// `line:col` (1-based) of byte offset `at`, or `?:?` when it is not a char boundary.
fn location(text: &str, at: usize) -> String {
    text.get(..at).map_or_else(
        || "?:?".to_owned(),
        |before| {
            let line = before.matches('\n').count() + 1;
            let col = before.rsplit('\n').next().unwrap_or(before);
            format!("{line}:{}", col.chars().count() + 1)
        },
    )
}

/// Infer candidate terms from `(name, text, skip ranges)` inputs.
///
/// `known` holds the lower-cased spellings that are already terms. Candidates
/// seen fewer than `min` times are dropped. The result is ordered by count
/// (highest first), then spelling.
#[must_use]
pub fn suggest(
    inputs: &[(&str, &str, Vec<Range<usize>>)],
    known: &BTreeSet<String>,
    min: usize,
) -> Vec<Suggestion> {
    let mut found: BTreeMap<String, (usize, BTreeSet<usize>, String)> = BTreeMap::new();
    for (n, (name, text, skip)) in inputs.iter().enumerate() {
        for r in tokens(text) {
            let Some(word) = text.get(r.clone()) else {
                continue;
            };
            if known.contains(&word.to_lowercase())
                || skip.iter().any(|s| s.start < r.end && r.start < s.end)
                || !is_candidate(word, text, r.start)
            {
                continue;
            }
            let at = location(text, r.start);
            let entry = found
                .entry(word.to_owned())
                .or_insert_with(|| (0, BTreeSet::new(), format!("{name}:{at}")));
            entry.0 += 1;
            entry.1.insert(n);
        }
    }
    let mut out: Vec<Suggestion> = found
        .into_iter()
        .filter(|(_, (count, _, _))| *count >= min)
        .map(|(term, (count, files, first))| Suggestion {
            term,
            count,
            files: files.len(),
            first,
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.term.cmp(&b.term)));
    out
}
