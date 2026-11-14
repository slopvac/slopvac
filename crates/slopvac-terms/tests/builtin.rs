//! The bundled terms list.

use std::collections::BTreeSet;

use slopvac_terms::{TermSet, TermsFile};

/// Ordinary words that the list must not carry: case-insensitive matches on
/// them would flag everyday prose.
const AMBIGUOUS: &[&str] = &[
    "go",
    "rust",
    "swift",
    "ruby",
    "react",
    "windows",
    "spark",
    "express",
    "rails",
    "yarn",
    "jest",
    "pandas",
    "vault",
    "rest",
    "ml",
    "os",
    "apple",
    "angular",
    "mocha",
    "slack",
    "notion",
    "bun",
    "snowflake",
    "ram",
    "id",
    "ok",
    "cd",
    "pr",
    "db",
    "vm",
    "ip",
    "nat",
    "spa",
    "soap",
    "helm",
    "pip",
    "bash",
    "safari",
    "edge",
    "opera",
    "gemini",
    "cargo",
    "spring",
    "prettier",
    "playwright",
    "cypress",
    "babel",
    "arm",
    "elm",
    "dart",
    "julia",
];

#[test]
fn builtin_file_parses_and_has_substance() {
    let f = TermsFile::builtin();
    assert!(f.terms.len() >= 300, "only {} terms", f.terms.len());
    assert_eq!(TermSet::builtin().len(), f.terms.len());
}

#[test]
fn builtin_has_no_ordinary_words() {
    let file = TermsFile::builtin();
    let bad: Vec<_> = file
        .terms
        .iter()
        .flat_map(|t| std::iter::once(&t.term).chain(t.variants.iter()))
        .filter(|s| AMBIGUOUS.contains(&s.to_lowercase().as_str()))
        .collect();
    assert_eq!(bad, Vec::<&String>::new());
}

#[test]
fn builtin_spellings_do_not_collide_case_insensitively() {
    let mut seen = BTreeSet::new();
    for t in TermsFile::builtin().terms {
        for s in std::iter::once(t.term.clone()).chain(t.variants.clone()) {
            assert!(seen.insert(s.to_lowercase()), "duplicate spelling {s}");
        }
    }
}

#[test]
fn builtin_flags_common_misspellings() {
    let set = TermSet::builtin();
    let text = "We use Github, javascript and Postgres with kubernetes on Linux.";
    let found: Vec<_> = set
        .check_casing(text, &[])
        .into_iter()
        .map(|f| (f.found, f.expected))
        .collect();
    assert_eq!(
        found,
        [
            ("Github".to_owned(), "GitHub".to_owned()),
            ("javascript".to_owned(), "JavaScript".to_owned()),
            ("kubernetes".to_owned(), "Kubernetes".to_owned()),
        ]
    );
}
