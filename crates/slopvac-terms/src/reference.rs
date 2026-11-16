//! Generated reference page for the built-in terms.
//!
//! [`reference_pages`] returns `(path, markdown)` pairs relative to the docs
//! site root, like the style reference in `slopvac-styles`.
#![expect(
    clippy::let_underscore_must_use,
    reason = "fmt::Write for String never returns an error"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::{TermEntry, TermsFile};

fn mode(t: &TermEntry) -> &'static str {
    if t.ignore_case {
        "any case"
    } else if t.case_sensitive {
        "exact spelling only"
    } else {
        "flags wrong case"
    }
}

fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

/// `terms/index.md`: the built-in list grouped by kind.
#[must_use]
pub fn reference_pages() -> Vec<(String, String)> {
    let file = TermsFile::builtin();
    let mut by_kind: BTreeMap<&str, Vec<&TermEntry>> = BTreeMap::new();
    for t in &file.terms {
        by_kind
            .entry(t.kind.as_deref().unwrap_or("other"))
            .or_default()
            .push(t);
    }
    let mut out = format!(
        "# Built-in terms\n\n\
         slopvac ships {} terms: product, language, protocol and acronym names with a canonical \
         spelling. The casing check flags text that matches a term ignoring case but differs from \
         the canonical spelling and from every listed variant. Plurals and possessives need no \
         entry. Matches inside paths, domains, handles and code are left alone.\n\n\
         Add your own with `slopvac terms add`, or list them in `.slopvac/terms.toml`. A project \
         term with the same canonical spelling replaces the built-in one.\n\n\
         ## Case modes\n\n\
         - **flags wrong case**: any other casing is a finding.\n\
         - **exact spelling only**: the name is also an ordinary word, so only the exact spelling \
         is the term and other casings are not findings.\n\
         - **any case**: every casing is accepted.\n\n\
         ## Provenance\n\n\
         The list is project-authored data under the repository licence (Apache-2.0). Each \
         canonical spelling comes from the project's own documentation. No third-party word list \
         was copied.\n",
        file.terms.len()
    );
    for (kind, terms) in &by_kind {
        let _ = write!(
            out,
            "\n## {kind}\n\n| Term | Variants | Case mode |\n| --- | --- | --- |\n"
        );
        let mut terms = terms.clone();
        terms.sort_by_key(|t| t.term.to_lowercase());
        for t in terms {
            let variants = if t.variants.is_empty() {
                "-".to_owned()
            } else {
                t.variants
                    .iter()
                    .map(|v| format!("`{}`", cell(v)))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let _ = writeln!(out, "| `{}` | {variants} | {} |", cell(&t.term), mode(t));
        }
    }
    vec![("terms/index.md".to_owned(), out)]
}
