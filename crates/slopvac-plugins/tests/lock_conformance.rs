//! Conformance for `.slopvac/packs.lock`.
//!
//! The fixtures under `tests/fixtures/packs-lock/` are copies of
//! `schemas/v3/conformance/packs-lock/` from the pack-schema contract
//! (PR #288, branch `contract/8l4.2.4-pack-schemas`, commit 5f39a88efc).
//! Once that contract is on main, point `FIXTURES` at the schema directory
//! and delete the copies.
#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use slopvac_plugins::lock::Lockfile;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/packs-lock")
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
    assert_eq!(names, ["empty.toml", "full.toml"]);
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
