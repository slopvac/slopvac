//! Builders shared by the engine tests.

use std::path::PathBuf;

use serde_json::{Value, json};
use slopvac_engine::{CategoryDials, Engine, Level, Settings};
use slopvac_model::{
    CategoryId, Document, GenreId, GenreResult, GenreSource, NonEmptyString, PackId, RuleId,
    Severity,
};
use slopvac_rules::{CompileOptions, RuleSet};

pub(crate) fn id(text: &str) -> RuleId {
    RuleId::new(text).unwrap()
}

/// A tokens rule `cat.<id>` with the given extra keys merged in.
pub(crate) fn tokens(id: &str, token: &str, extra: &Value) -> Value {
    rule(id, &json!({"kind": "tokens", "tokens": [token]}), extra)
}

pub(crate) fn rule(id: &str, matcher: &Value, extra: &Value) -> Value {
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

pub(crate) fn category(id: &str, rules: &[Value]) -> (PathBuf, String) {
    let doc = json!({
        "schema_version": 3, "id": id, "title": "t", "description": "d", "rules": rules
    });
    (PathBuf::from(format!("{id}.json")), doc.to_string())
}

pub(crate) fn ruleset(files: Vec<(PathBuf, String)>) -> RuleSet {
    RuleSet::from_sources(
        &PackId::new("core").unwrap(),
        files,
        &CompileOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"))
}

pub(crate) fn settings() -> Settings {
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
pub(crate) fn engine(rules: &[Value]) -> Engine {
    Engine::new(vec![ruleset(vec![category("cat", rules)])], settings()).unwrap()
}

/// `(rule id, line, column)` of each finding, in report order.
pub(crate) fn places(doc: &Document) -> Vec<(String, usize, usize)> {
    doc.findings
        .iter()
        .map(|f| (f.rule_id.to_string(), f.start.line(), f.start.column()))
        .collect()
}

/// The matched text of each finding that is not a directive error.
pub(crate) fn matched(doc: &Document) -> Vec<&str> {
    doc.findings
        .iter()
        .filter(|f| !f.rule_id.as_str().contains("meta."))
        .map(|f| f.matched_text.as_str())
        .collect()
}

/// A directory allocated for this caller alone, under the cargo test tmp directory.
///
/// `cargo nextest` runs every test in its own process, so two processes must never share
/// a directory, and nothing here deletes a directory it did not create. The name is unique
/// (`tempfile` picks it), and the directory stays until `cargo clean`: a test process has
/// no hook that runs after its last test, and its helper cannot be dropped earlier because
/// other tests of the same process still use the scripts in it.
pub(crate) fn private_dir(prefix: &str) -> PathBuf {
    tempfile::Builder::new()
        .prefix(&format!("{prefix}-"))
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
        .keep()
}

pub(crate) fn root() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub(crate) fn read_json(path: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn level(name: &str) -> Level {
    match name {
        "error" => Level::On(Severity::Error),
        "warning" => Level::On(Severity::Warning),
        "suggestion" => Level::On(Severity::Suggestion),
        "off" => Level::Off,
        other => panic!("unknown level {other}"),
    }
}

/// The rule that the converted 2.x data still writes as a pattern. Rust runs it as the native
/// `capitalization` kind, so its findings check that kind against the 2.x pattern.
pub(crate) const TITLE_CASE: &str = "title-case-heading";

fn core_sources() -> Vec<(PathBuf, String)> {
    let core = root().join("crates/slopvac-rules/tests/fixtures/core-v3");
    let mut files: Vec<PathBuf> = std::fs::read_dir(core)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    let mut swapped = 0;
    let sources = files
        .into_iter()
        .map(|path| {
            let mut doc = read_json(&path);
            for rule in doc["rules"].as_array_mut().into_iter().flatten() {
                if rule["id"] == TITLE_CASE {
                    rule["match"] = json!({"kind": "capitalization", "style": "sentence"});
                    swapped += 1;
                }
            }
            (path, doc.to_string())
        })
        .collect();
    assert_eq!(swapped, 1, "{TITLE_CASE} is one rule of the core data");
    sources
}

/// The engine for one profile: the category dials and rule levels of `core:legacy-<profile>`
/// and the thresholds of the profile, both from the profile mapping contract.
pub(crate) fn legacy_engine(profile: &str) -> Engine {
    let set = RuleSet::from_sources(
        &PackId::new("core").unwrap(),
        core_sources(),
        &CompileOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let mut settings = Settings::new(
        NonEmptyString::new(profile).unwrap(),
        GenreResult {
            id: GenreId::new("reference").unwrap(),
            source: GenreSource::Default,
            reasons: Vec::new(),
        },
    );
    let profiles = root().join("docs/contract/compat/profiles");
    let style: toml::Table =
        std::fs::read_to_string(profiles.join(format!("legacy-{profile}.toml")))
            .unwrap()
            .parse()
            .unwrap();
    for (name, dials) in style["categories"].as_table().unwrap() {
        settings.categories.insert(
            CategoryId::new(name).unwrap(),
            CategoryDials {
                level: Some(level(dials["severity"].as_str().unwrap())),
                weight: dials.get("weight").and_then(toml::Value::as_float),
                max_per_100_words: dials
                    .get("max_per_100_words")
                    .and_then(toml::Value::as_float),
            },
        );
    }
    for (id, value) in style["rules"].as_table().unwrap() {
        settings.rule_levels.insert(
            RuleId::new(format!("core:{id}")).unwrap(),
            level(value.as_str().unwrap()),
        );
    }
    let mapping = read_json(&profiles.join("mapping.json"));
    let t = &mapping["profiles"][profile]["thresholds"];
    let count = |k: &str| t[k].as_u64().map(|n| usize::try_from(n).unwrap());
    settings.thresholds.max_errors = count("max_errors");
    settings.thresholds.max_warnings = count("max_warnings");
    settings.thresholds.max_unicode_dashes = count("max_unicode_dashes");
    settings.thresholds.max_total_per_100_words = t["max_total_per_100_words"].as_f64();
    settings.thresholds.min_score = t["min_score"].as_f64();
    Engine::new(vec![set], settings).unwrap()
}
