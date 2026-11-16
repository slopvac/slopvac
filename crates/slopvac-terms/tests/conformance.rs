//! The terms conformance corpus under `schemas/v3/conformance/`.
//!
//! Every file under `valid/` must load. Every file under `invalid/` other than
//! `cases.toml` must be rejected. The blocklist corpus is not covered here: it
//! requires `schema_version`, which the 2.x blocklist format lacks.
#![cfg(test)]

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
