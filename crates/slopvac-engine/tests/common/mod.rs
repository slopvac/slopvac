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

/// The engine the 2.x parity fixtures were generated for: the core rules with the levels
/// and category dials that `engine-parity.json` records.
pub fn parity_engine() -> Engine {
    use slopvac_engine::{CategoryDials, Level};
    use slopvac_model::{CategoryId, Severity};

    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string(manifest.join("tests/fixtures/engine-parity.json")).unwrap(),
    )
    .unwrap();
    let core = manifest.join("../slopvac-rules/tests/fixtures/core-v3");
    let set = RuleSet::load_dir(
        &PackId::new("core").unwrap(),
        &core,
        &CompileOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let s = &fixture["settings"];
    let mut settings = settings();
    for (id, level) in s["rule_levels"].as_object().unwrap() {
        let level = match level.as_str().unwrap() {
            "error" => Level::On(Severity::Error),
            "warning" => Level::On(Severity::Warning),
            "suggestion" => Level::On(Severity::Suggestion),
            "off" => Level::Off,
            other => panic!("unknown level {other}"),
        };
        settings.rule_levels.insert(RuleId::new(id).unwrap(), level);
    }
    for (name, dials) in s["categories"].as_object().unwrap() {
        settings.categories.insert(
            CategoryId::new(name).unwrap(),
            CategoryDials {
                level: None,
                weight: dials["weight"].as_f64(),
                max_per_100_words: dials["budget"].as_f64(),
            },
        );
    }
    Engine::new(vec![set], settings).unwrap()
}
