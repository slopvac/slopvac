//! The terms conformance corpus under `schemas/v3/conformance/`.
//!
//! Every file under `valid/` must load. Every file under `invalid/` other than
//! `cases.toml` must be rejected. Blocklist files also name the reason: the
//! loader must give the diagnostic that matches each file's fault.

use std::path::{Path, PathBuf};

use slopvac_terms::TermsFile;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schemas/v3/conformance");

fn files(kind: &str, outcome: &str) -> Vec<PathBuf> {
    let dir = Path::new(ROOT).join(kind).join(outcome);
    let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().is_some_and(|n| n != "cases.toml"))
        .collect();
    found.sort();
    assert!(!found.is_empty(), "no corpus files in {}", dir.display());
    found
}

#[test]
fn valid_terms_files_load() {
    for path in files("terms", "valid") {
        TermsFile::load(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
}

#[test]
fn invalid_terms_files_are_rejected() {
    for path in files("terms", "invalid") {
        assert!(
            TermsFile::load(&path).is_err(),
            "{} should be rejected",
            path.display()
        );
    }
}

use slopvac_terms::{Blocklist, TermsError};

/// Each invalid terms file and the part of the diagnostic that names its fault, pinned to the
/// shape `TermsFile` has today (`schemas/v3/terms.schema.json`; update this table with the
/// type). `invalid/cases.toml` gives a JSON pointer per file; the loader words the fault, so the
/// table pins the wording instead.
///
/// Files the loader turns away while parsing, before its own checks, show the parser's message.
/// That includes `no-version.toml`: unlike a blocklist, a terms file with no `schema_version`
/// gets a generic parse error, not a "missing `schema_version`" diagnostic.
const TERMS_REASONS: [(&str, &str); 7] = [
    (
        "both-casing-modes.toml",
        "sets both ignore_case and case_sensitive",
    ),
    (
        "case-sensitive-not-bool.toml",
        "TOML parse error at line 5, column 18",
    ),
    ("empty-term.toml", "term 1 is empty"),
    ("kind-not-kebab.toml", "has kind \"Brand Name\""),
    ("no-version.toml", "TOML parse error at line 1, column 1"),
    ("term-missing.toml", "TOML parse error at line 3, column 1"),
    ("unknown-key.toml", "unknown key `terms[1].case`"),
];

#[test]
fn invalid_terms_files_are_rejected_for_their_reason() {
    let found = files("terms", "invalid");
    let names: Vec<_> = found
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap().to_owned())
        .collect();
    let mut expected: Vec<_> = TERMS_REASONS.iter().map(|(n, _)| (*n).to_owned()).collect();
    expected.sort();
    assert_eq!(names, expected, "corpus files and reason table differ");
    for (name, reason) in TERMS_REASONS {
        let path = Path::new(ROOT).join("terms/invalid").join(name);
        let error = TermsFile::load(&path).expect_err(name);
        assert!(error.to_string().contains(reason), "{name}: {error}");
    }
}

#[test]
fn terms_cases_file_lists_exactly_the_invalid_files() {
    let text = std::fs::read_to_string(Path::new(ROOT).join("terms/invalid/cases.toml")).unwrap();
    let cases: toml::Table = text.parse().unwrap();
    let mut listed: Vec<_> = cases.keys().cloned().collect();
    listed.sort();
    let mut expected: Vec<_> = TERMS_REASONS.iter().map(|(n, _)| (*n).to_owned()).collect();
    expected.sort();
    assert_eq!(listed, expected);
}

#[test]
fn valid_terms_files_load_with_the_terms_they_declare() {
    let load =
        |name: &str| TermsFile::load(&Path::new(ROOT).join("terms/valid").join(name)).unwrap();
    assert_eq!(load("empty.toml").terms, []);
    assert!(load("full.toml").terms.len() > 1);
    let casing = load("casing.toml");
    assert!(casing.terms.iter().any(|t| t.ignore_case));
    assert!(casing.terms.iter().any(|t| t.case_sensitive));
}

/// Each invalid blocklist file and the part of the diagnostic that names its
/// fault. `invalid/cases.toml` gives a JSON pointer per file; the loader reports
/// the entry number and field in words, so the table pins the reason instead.
const BLOCKLIST_REASONS: [(&str, &str); 9] = [
    ("no-version.toml", "missing `schema_version`"),
    ("wrong-table.toml", "no `entries` array; it defines `words`"),
    ("unknown-key.toml", "unknown key `entries[1].severity`"),
    ("no-word.toml", "entry 1 has no `word`"),
    ("whitespace-word.toml", "entry 1 has no `word`"),
    ("no-pos.toml", "unrecognized `pos`"),
    ("bad-pos.toml", "unrecognized `pos`"),
    ("no-reason.toml", "has no `reason`"),
    ("blank-reason.toml", "has no `reason`"),
];

#[test]
fn valid_blocklist_files_load() {
    for path in files("blocklist", "valid") {
        let list = Blocklist::load(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(!list.is_empty(), "{} loaded no entries", path.display());
    }
}

#[test]
fn invalid_blocklist_files_are_rejected_for_their_reason() {
    let found = files("blocklist", "invalid");
    let names: Vec<_> = found
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap().to_owned())
        .collect();
    let mut expected: Vec<_> = BLOCKLIST_REASONS
        .iter()
        .map(|(n, _)| (*n).to_owned())
        .collect();
    expected.sort();
    assert_eq!(names, expected, "corpus files and reason table differ");
    for (name, reason) in BLOCKLIST_REASONS {
        let path = Path::new(ROOT).join("blocklist/invalid").join(name);
        let error: TermsError = Blocklist::load(&path).expect_err(name);
        assert!(error.to_string().contains(reason), "{name}: {error}");
    }
}

#[test]
fn blocklist_cases_file_lists_exactly_the_invalid_files() {
    let text =
        std::fs::read_to_string(Path::new(ROOT).join("blocklist/invalid/cases.toml")).unwrap();
    let cases: toml::Table = text.parse().unwrap();
    let mut listed: Vec<_> = cases.keys().cloned().collect();
    listed.sort();
    let mut expected: Vec<_> = BLOCKLIST_REASONS
        .iter()
        .map(|(n, _)| (*n).to_owned())
        .collect();
    expected.sort();
    assert_eq!(listed, expected);
}
