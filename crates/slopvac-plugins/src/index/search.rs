//! `packs search`: match and rank index entries, and render hits for a terminal.
//!
//! Ranking. The query is lowercased and split into tokens at every character
//! that is not alphanumeric. An entry matches when every token is a substring
//! of the lowercased id, name or description. Each token scores the best one of:
//!
//! | The token is | Points |
//! | --- | --- |
//! | equal to the id | 100 |
//! | a prefix of the id | 60 |
//! | inside the id | 40 |
//! | inside the name | 30 |
//! | inside the description | 10 |
//!
//! The scores add up. A query whose tokens, joined by `-`, equal the id adds
//! 50. Hits sort by score, highest first, then by namespace and id. A query
//! with no tokens matches nothing.

use std::fmt::Write as _;

use super::Index;
use super::model::IndexEntry;
use super::text::sanitize_for_terminal;

/// An entry that matched, with its score.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchHit<'a> {
    /// The matched entry.
    pub entry: &'a IndexEntry,
    /// The ranking score; higher is better.
    pub score: u32,
}

fn tokens(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

fn score_token(token: &str, id: &str, name: &str, description: &str) -> Option<u32> {
    if id == token {
        Some(100)
    } else if id.starts_with(token) {
        Some(60)
    } else if id.contains(token) {
        Some(40)
    } else if name.contains(token) {
        Some(30)
    } else if description.contains(token) {
        Some(10)
    } else {
        None
    }
}

fn score(entry: &IndexEntry, tokens: &[String]) -> Option<u32> {
    let name = entry.name.to_lowercase();
    let description = entry.description.to_lowercase();
    let mut total = 0;
    for token in tokens {
        total += score_token(token, &entry.id, &name, &description)?;
    }
    if tokens.join("-") == entry.id {
        total += 50;
    }
    Some(total)
}

/// Returns up to `limit` matching entries, best first.
#[must_use]
pub fn search<'a>(index: &'a Index, query: &str, limit: usize) -> Vec<SearchHit<'a>> {
    let tokens = tokens(query);
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<_> = index
        .entries()
        .iter()
        .filter_map(|entry| score(entry, &tokens).map(|score| SearchHit { entry, score }))
        .collect();
    // The entries are already sorted by namespace and id, and the sort is stable.
    hits.sort_by_key(|h| std::cmp::Reverse(h.score));
    hits.truncate(limit);
    hits
}

impl SearchHit<'_> {
    /// Renders the hit for a terminal. Every value from the index is escaped,
    /// and the key line says that the key is not verified.
    #[must_use]
    pub fn render(&self) -> String {
        let e = self.entry;
        let s = sanitize_for_terminal;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{}/{} {} ({}, needs slopvac API {})",
            s(&e.namespace),
            s(&e.id),
            e.version,
            e.kind.as_str(),
            e.slopvac_api
        );
        let _ = writeln!(out, "  {}", s(&e.name));
        let _ = writeln!(out, "  {}", s(&e.description));
        let _ = writeln!(out, "  source: {}", s(&e.source));
        let _ = writeln!(
            out,
            "  publisher key (from the index, not verified): {} (fingerprint {})",
            s(&e.publisher_key.key_id),
            s(&e.publisher_key.fingerprint)
        );
        out
    }
}
