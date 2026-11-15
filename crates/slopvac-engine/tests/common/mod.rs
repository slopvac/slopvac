//! Builders shared by the engine tests.

#![allow(dead_code, unreachable_pub, missing_docs)]

use std::path::PathBuf;

use serde_json::{Value, json};
use slopvac_engine::{Engine, Settings};
use slopvac_model::{Document, GenreId, GenreResult, GenreSource, NonEmptyString, PackId, RuleId};
use slopvac_rules::{CompileOptions, RuleSet};

pub fn id(text: &str) -> RuleId {
    RuleId::new(text).unwrap()
}

/// A tokens rule `cat.<id>` with the given extra keys merged in.
pub fn tokens(id: &str, token: &str, extra: &Value) -> Value {
    rule(id, &json!({"kind": "tokens", "tokens": [token]}), extra)
}

pub fn rule(id: &str, matcher: &Value, extra: &Value) -> Value {
    let mut rule = json!({
        "id": id, "name": "n", "message": "Found {match}.", "severity": "warning",
        "match": matcher,
        "examples": [],
        "citation": {"source": "s"}
    });
    rule.as_object_mut().unwrap().remove("examples");
    for (k, v) in extra.as_object().into_iter().flatten() {
        rule[k] = v.clone();
    }
    rule
}

pub fn category(id: &str, rules: &[Value]) -> (PathBuf, String) {
    let doc = json!({
        "schema_version": 3, "id": id, "title": "t", "description": "d", "rules": rules
    });
    (PathBuf::from(format!("{id}.json")), doc.to_string())
}

pub fn ruleset(files: Vec<(PathBuf, String)>) -> RuleSet {
    RuleSet::from_sources(
        &PackId::new("core").unwrap(),
        files,
        &CompileOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"))
}

pub fn settings() -> Settings {
    Settings::new(
        NonEmptyString::new("normal").unwrap(),
        GenreResult {
            id: GenreId::new("reference").unwrap(),
            source: GenreSource::Default,
            reasons: Vec::new(),
        },
    )
}

/// An engine over one category `cat` holding `rules`.
pub fn engine(rules: &[Value]) -> Engine {
    Engine::new(vec![ruleset(vec![category("cat", rules)])], settings()).unwrap()
}

/// `(rule id, line, column)` of each finding, in report order.
pub fn places(doc: &Document) -> Vec<(String, usize, usize)> {
    doc.findings
        .iter()
        .map(|f| (f.rule_id.to_string(), f.start.line(), f.start.column()))
        .collect()
}

/// The matched text of each finding that is not a directive error.
pub fn matched(doc: &Document) -> Vec<&str> {
    doc.findings
        .iter()
        .filter(|f| !f.rule_id.as_str().contains("meta."))
        .map(|f| f.matched_text.as_str())
        .collect()
}

/// A directory only this process uses, under the cargo test tmp directory.
///
/// `cargo nextest` runs every test in its own process, so two processes must never share
/// (or clear) one directory. Directories left by finished runs are removed after an hour.
pub fn private_dir(prefix: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    if let Ok(entries) = std::fs::read_dir(&root) {
        let stale = std::time::Duration::from_secs(3600);
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > stale);
            if old
                && entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{prefix}-"))
            {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    let dir = root.join(format!("{prefix}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
