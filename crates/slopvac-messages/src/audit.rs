//! Registry checks that every catalog must pass together.

use std::collections::HashMap;
use std::fmt;

use crate::entry::Entry;
use crate::template::{Segment, segments};

/// One broken registry rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The catalog that defines the entry.
    pub catalog: String,
    /// The entry code.
    pub code: String,
    /// What is wrong.
    pub problem: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} / {}: {}", self.catalog, self.code, self.problem)
    }
}

/// Check `catalogs` (name and entries) as one registry.
///
/// The rules: every code is unique across all catalogs, every entry has a summary, every
/// template placeholder names a declared parameter, and no text names a bead or a pull
/// request, because users cannot act on either.
#[must_use]
pub fn audit(catalogs: &[(&str, &[Entry])]) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut owner: HashMap<&str, &str> = HashMap::new();
    for &(catalog, entries) in catalogs {
        for entry in entries {
            let mut report = |problem: String| {
                violations.push(Violation {
                    catalog: catalog.to_owned(),
                    code: entry.code.to_owned(),
                    problem,
                });
            };
            if let Some(first) = owner.insert(entry.code, catalog) {
                report(format!("the code is already defined in `{first}`"));
            }
            if entry.summary.trim().is_empty() {
                report("the summary is empty".to_owned());
            }
            for segment in segments(entry.template) {
                if let Segment::Placeholder(name) = segment
                    && !entry.params.iter().any(|p| p.name == name)
                {
                    report(format!(
                        "the template uses `{{{name}}}`, which is not a parameter"
                    ));
                }
            }
            for (field, text) in [("template", entry.template), ("summary", entry.summary)] {
                if let Some(id) = internal_id(text) {
                    report(format!("the {field} names the internal id `{id}`"));
                }
            }
        }
    }
    violations
}

/// A bead id (`slopvac-8l4.21.6`) or pull request number (`#470`) in `text`.
fn internal_id(text: &str) -> Option<&str> {
    for (start, _) in text.match_indices("slopvac-") {
        let rest = &text[start + "slopvac-".len()..];
        let word_len = rest
            .find(|c: char| !(c.is_ascii_digit() || c.is_ascii_lowercase()))
            .unwrap_or(rest.len());
        if word_len > 0 && rest[word_len..].starts_with('.') {
            return Some(&text[start..=start + "slopvac-".len() + word_len]);
        }
    }
    text.match_indices('#').find_map(|(start, _)| {
        let digits = text[start + 1..]
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len() - start - 1);
        (digits > 0).then(|| &text[start..=start + digits])
    })
}
