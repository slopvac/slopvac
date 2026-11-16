//! No linked tree-sitter grammar ships an external scanner.
//!
//! A scanner is hand-written C that runs on every parse and keeps its own state in raw
//! memory. The F#, Kotlin and Swift scanners hung or corrupted memory on hostile input,
//! so this crate links only grammars that are generated tables with no scanner, and
//! reads every other language with the pure-Rust lexer.
//!
//! The check fails closed. A grammar crate whose source directory is missing or cannot
//! be read is an error, not a pass, and every linked grammar must show a `parser.c`.

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
        .args(["metadata", "--format-version", "1", "--locked", "--offline"])
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

/// Whether a directory name holds sources that do not ship in the grammar's library.
fn skipped_dir(name: &str) -> bool {
    matches!(
        name,
        "test" | "tests" | "examples" | "node_modules" | "target"
    )
}

/// Collect every scanner source under `dir`. Any directory or entry that cannot be read
/// is an error: a scanner in an unreadable place would otherwise pass unseen.
fn scanner_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    let entries = std::fs::read_dir(dir)
        .map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("bad entry in {}: {error}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot stat {}: {error}", path.display()))?;
        if file_type.is_dir() {
            if !skipped_dir(&name) {
                found.extend(scanner_files(&path)?);
            }
        } else if name.starts_with("scanner.") || name.starts_with("scanner_") {
            found.push(path);
        }
    }
    Ok(found)
}

/// Inspect one grammar crate: its sources must be present, and none may be a scanner.
fn inspect(name: &str, dir: &Path) -> Result<(), String> {
    if !dir.join("src").join("parser.c").is_file() {
        return Err(format!(
            "{name}: {} has no src/parser.c, so its sources were not found",
            dir.display()
        ));
    }
    let found = scanner_files(dir)?;
    if found.is_empty() {
        Ok(())
    } else {
        Err(format!("{name}: external scanner {found:?}"))
    }
}

fn grammar_crates() -> Vec<(String, PathBuf)> {
    linked_packages(&metadata())
        .into_iter()
        .filter(|(name, _)| name.starts_with("tree-sitter-") && name != "tree-sitter-language")
        .collect()
}

#[test]
fn no_linked_grammar_has_an_external_scanner() {
    let grammars = grammar_crates();
    assert!(
        !grammars.is_empty(),
        "no grammar crate was found to inspect"
    );
    let mut inspected = BTreeSet::new();
    let mut problems = Vec::new();
    for (name, dir) in &grammars {
        match inspect(name, dir) {
            Ok(()) => {
                inspected.insert(name.clone());
            }
            Err(problem) => problems.push(problem),
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    let linked: BTreeSet<String> = grammars.into_iter().map(|(name, _)| name).collect();
    assert_eq!(inspected, linked, "every linked grammar must be inspected");
}

#[test]
fn only_the_allowed_grammar_crates_are_linked() {
    let linked: BTreeSet<String> = grammar_crates().into_iter().map(|(name, _)| name).collect();
    let allowed: BTreeSet<String> = ALLOWED.iter().map(|name| (*name).to_owned()).collect();
    let extra: Vec<_> = linked.difference(&allowed).collect();
    assert!(
        extra.is_empty(),
        "grammar crates outside the allow list: {extra:?}"
    );
}

/// A scratch crate directory under the system temp dir, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("slopvac-comments-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).expect("dir");
        Self(dir)
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(path, text).expect("file");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Restore permissions a test removed, so the cleanup can read the tree.
        #[cfg(unix)]
        restore(&self.0);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
fn restore(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                restore(&entry.path());
            }
        }
    }
}

#[test]
fn a_crate_with_a_scanner_fails_the_check() {
    let crate_dir = Scratch::new("with-scanner");
    crate_dir.write("src/parser.c", "int parser;");
    crate_dir.write("src/scanner.c", "int scanner;");
    let error = inspect("tree-sitter-fake", &crate_dir.0).expect_err("has a scanner");
    assert!(error.contains("external scanner"), "{error}");
}

#[test]
fn a_scanner_in_a_nested_directory_fails_the_check() {
    let crate_dir = Scratch::new("nested-scanner");
    crate_dir.write("src/parser.c", "int parser;");
    crate_dir.write("php/src/scanner.cc", "int scanner;");
    assert!(inspect("tree-sitter-fake", &crate_dir.0).is_err());
}

#[test]
fn a_scanner_free_crate_passes_the_check() {
    let crate_dir = Scratch::new("clean");
    crate_dir.write("src/parser.c", "int parser;");
    crate_dir.write("tests/scanner.c", "ignored: test sources do not ship");
    assert_eq!(inspect("tree-sitter-fake", &crate_dir.0), Ok(()));
}

#[test]
fn a_missing_source_directory_fails_the_check() {
    let missing = std::env::temp_dir().join("slopvac-comments-does-not-exist");
    let error = inspect("tree-sitter-fake", &missing).expect_err("missing dir");
    assert!(error.contains("not found"), "{error}");
    assert!(scanner_files(&missing).is_err());
}

#[test]
fn a_crate_without_a_parser_fails_the_check() {
    let crate_dir = Scratch::new("no-parser");
    crate_dir.write("src/lib.rs", "");
    let error = inspect("tree-sitter-fake", &crate_dir.0).expect_err("no parser.c");
    assert!(error.contains("src/parser.c"), "{error}");
}

#[cfg(unix)]
#[test]
fn an_unreadable_directory_fails_the_check() {
    use std::os::unix::fs::PermissionsExt;
    let crate_dir = Scratch::new("unreadable");
    crate_dir.write("src/parser.c", "int parser;");
    crate_dir.write("hidden/scanner.c", "int scanner;");
    std::fs::set_permissions(
        crate_dir.0.join("hidden"),
        std::fs::Permissions::from_mode(0o000),
    )
    .expect("chmod");
    // Running as root can read through mode 000, so only assert when the read fails.
    if std::fs::read_dir(crate_dir.0.join("hidden")).is_err() {
        let error = inspect("tree-sitter-fake", &crate_dir.0).expect_err("unreadable");
        assert!(error.contains("cannot read"), "{error}");
    }
}
