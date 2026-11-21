//! Conformance for `.slopvac/plugins.lock`, run against the shared corpus in
//! `schemas/v3/conformance/plugins-lock/`.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use slopvac_plugins::lock::Lockfile;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3/conformance/plugins-lock")
}

fn toml_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| Path::new(n).extension().is_some_and(|e| e == "toml") && n != "cases.toml")
        .collect();
    names.sort();
    names
}

fn cases() -> BTreeMap<String, String> {
    let text = fs::read_to_string(fixtures().join("invalid/cases.toml")).unwrap();
    toml::from_str(&text).unwrap()
}

#[test]
fn every_valid_case_loads() {
    let dir = fixtures().join("valid");
    let names = toml_files(&dir);
    assert!(!names.is_empty(), "valid/ holds no cases");
    for name in names {
        let text = fs::read_to_string(dir.join(&name)).unwrap();
        if let Err(e) = Lockfile::parse(&text) {
            panic!("valid/{name} was rejected: {e}");
        }
    }
}

#[test]
fn every_invalid_case_fails_at_the_listed_pointer() {
    let dir = fixtures().join("invalid");
    let cases = cases();
    assert!(!cases.is_empty(), "invalid/cases.toml lists no cases");
    assert_eq!(
        toml_files(&dir),
        cases.keys().cloned().collect::<Vec<_>>(),
        "cases.toml and the invalid/ directory must list the same files"
    );
    for (name, pointer) in &cases {
        let text = fs::read_to_string(dir.join(name)).unwrap();
        match Lockfile::parse(&text) {
            Ok(_) => panic!("invalid/{name} was accepted"),
            Err(e) => assert_eq!(&e.pointer, pointer, "invalid/{name}: {e}"),
        }
    }
}
