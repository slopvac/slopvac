//! No linked tree-sitter grammar ships an external scanner.
//!
//! A scanner is hand-written C that runs on every parse and keeps its own state in raw
//! memory. The F#, Kotlin and Swift scanners hung or corrupted memory on hostile input,
//! so this crate links only grammars that are generated tables with no scanner, and
//! reads every other language with the pure-Rust lexer.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// Grammar crates that may be linked. Each has `src/parser.c` only.
const ALLOWED: &[&str] = &[
    "tree-sitter-c",
    "tree-sitter-clojure-orchard",
    "tree-sitter-go",
    "tree-sitter-java",
    "tree-sitter-proto",
];

fn metadata() -> Value {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("metadata is JSON")
}

/// Names and manifest directories of every normal dependency of this crate, direct or not.
fn linked_packages(metadata: &Value) -> Vec<(String, PathBuf)> {
    let packages = metadata["packages"].as_array().expect("packages");
    let nodes = metadata["resolve"]["nodes"].as_array().expect("nodes");
    let root = packages
        .iter()
        .find(|p| p["name"] == "slopvac-comments")
        .expect("this crate")["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let mut seen = BTreeSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let node = nodes.iter().find(|n| n["id"] == id.as_str()).expect("node");
        for dep in node["deps"].as_array().expect("deps") {
            let normal = dep["dep_kinds"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|k| k["kind"].is_null()));
            if normal {
                stack.push(dep["pkg"].as_str().expect("pkg").to_owned());
            }
        }
    }
    packages
        .iter()
        .filter(|p| seen.contains(p["id"].as_str().expect("id")))
        .map(|p| {
            let manifest = Path::new(p["manifest_path"].as_str().expect("manifest"));
            (
                p["name"].as_str().expect("name").to_owned(),
                manifest.parent().expect("dir").to_owned(),
            )
        })
        .collect()
}

fn scanner_files(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !matches!(
                name.as_str(),
                "test" | "tests" | "examples" | "node_modules" | "target"
            ) {
                scanner_files(&path, found);
            }
        } else if name.starts_with("scanner.") || name.starts_with("scanner_") {
            found.push(path);
        }
    }
}

#[test]
fn no_linked_grammar_has_an_external_scanner() {
    let mut offenders = Vec::new();
    for (name, dir) in linked_packages(&metadata()) {
        if !name.starts_with("tree-sitter-") || name == "tree-sitter-language" {
            continue;
        }
        let mut found = Vec::new();
        scanner_files(&dir, &mut found);
        if !found.is_empty() {
            offenders.push(format!("{name}: {found:?}"));
        }
    }
    assert!(offenders.is_empty(), "{}", offenders.join("\n"));
}

#[test]
fn only_the_allowed_grammar_crates_are_linked() {
    let linked: BTreeSet<String> = linked_packages(&metadata())
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| name.starts_with("tree-sitter-") && name != "tree-sitter-language")
        .collect();
    let allowed: BTreeSet<String> = ALLOWED.iter().map(|name| (*name).to_owned()).collect();
    let extra: Vec<_> = linked.difference(&allowed).collect();
    assert!(
        extra.is_empty(),
        "grammar crates outside the allow list: {extra:?}"
    );
}

/// The check itself must see a scanner when there is one: `tree-sitter-python` has one.
#[test]
fn the_scanner_finder_finds_a_scanner() {
    let dir = std::env::temp_dir().join("slopvac-comments-scanner-probe");
    std::fs::create_dir_all(dir.join("src")).expect("dir");
    std::fs::write(dir.join("src/scanner.c"), "int x;").expect("file");
    let mut found = Vec::new();
    scanner_files(&dir, &mut found);
    std::fs::remove_dir_all(&dir).expect("cleanup");
    assert_eq!(found.len(), 1);
}
