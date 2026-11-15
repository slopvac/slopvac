//! Parity with the 2.x Python engine on the interim fixture.
//!
//! STUB(slopvac-8l4.3.29): `fixtures/engine-parity.json` holds what the 2.x native engine
//! reported for a frozen corpus (`fixtures/generate.py`). The golden suite replaces it.
//! `fixtures/divergences.json` lists the documents that differ on purpose. Each entry gives
//! the reason and the facts that are not compared for it (`position`, `end`, `words`, `sentences`, `paragraphs`,
//! `score`, `gate`). Everything else is still compared. A listed fact must still differ, so
//! the list cannot go stale.

#![allow(
    missing_docs,
    clippy::assert_is_empty,
    clippy::cloned_ref_to_slice_refs
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
use slopvac_engine::{CategoryDials, Engine, Input, Level, Settings};
use slopvac_model::{
    CategoryId, Document, GenreId, GenreResult, GenreSource, NonEmptyString, PackId, RuleId,
    Severity,
};
use slopvac_rules::{CompileOptions, RuleSet};

fn manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn severity(name: &str) -> Level {
    match name {
        "error" => Level::On(Severity::Error),
        "warning" => Level::On(Severity::Warning),
        "suggestion" => Level::On(Severity::Suggestion),
        "off" => Level::Off,
        other => panic!("unknown level {other}"),
    }
}

/// The rule the converted 2.x data still writes as a pattern, and the reference ran as one.
/// Rust runs it as the native `capitalization` kind, so its findings check that kind against
/// the 2.x pattern.
const TITLE_CASE: &str = "title-case-heading";

/// The core rule files, with `title-case-heading` rewritten to the capitalization kind.
fn core_sources() -> Vec<(PathBuf, String)> {
    let core = manifest().join("../slopvac-rules/tests/fixtures/core-v3");
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
            let mut doc = read(&path);
            for rule in doc["rules"].as_array_mut().into_iter().flatten() {
                if rule["id"] == TITLE_CASE {
                    rule["match"] =
                        serde_json::json!({"kind": "capitalization", "style": "sentence"});
                    swapped += 1;
                }
            }
            (path, doc.to_string())
        })
        .collect();
    assert_eq!(swapped, 1, "{TITLE_CASE} is one rule of the core data");
    sources
}

fn engine(fixture: &Value) -> Engine {
    let set = RuleSet::from_sources(
        &PackId::new("core").unwrap(),
        core_sources(),
        &CompileOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let s = &fixture["settings"];
    let mut settings = Settings::new(
        NonEmptyString::new(s["profile"].as_str().unwrap()).unwrap(),
        GenreResult {
            id: GenreId::new("reference").unwrap(),
            source: GenreSource::Default,
            reasons: Vec::new(),
        },
    );
    for (id, level) in s["rule_levels"].as_object().unwrap() {
        settings
            .rule_levels
            .insert(RuleId::new(id).unwrap(), severity(level.as_str().unwrap()));
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
    let t = &s["thresholds"];
    let count = |k: &str| t[k].as_u64().map(|n| usize::try_from(n).unwrap());
    settings.thresholds.max_errors = count("max_errors");
    settings.thresholds.max_warnings = count("max_warnings");
    settings.thresholds.max_unicode_dashes = count("max_unicode_dashes");
    settings.thresholds.max_total_per_100_words = t["max_total_per_100_words"].as_f64();
    settings.thresholds.min_score = t["min_score"].as_f64();
    Engine::new(vec![set], settings).unwrap()
}

type Row = (String, usize, usize, usize, String, String);

#[derive(serde::Deserialize)]
struct Divergence {
    #[allow(dead_code)]
    reason: String,
    ignore: Vec<String>,
}

/// Findings reduced to what one comparison reads: `0` keeps everything, `1` drops the end
/// column, `2` drops the position too.
fn keyed(rows: Vec<Row>, level: u8) -> Vec<Row> {
    let mut rows: Vec<Row> = rows
        .into_iter()
        .map(|(r, l, c, e, s, m)| match level {
            0 => (r, l, c, e, s, m),
            1 => (r, l, c, 0, s, m),
            _ => (r, 0, 0, 0, s, m),
        })
        .collect();
    if level == 2 {
        rows.sort();
    }
    rows
}

fn rows_expected(expected: &Value) -> Vec<Row> {
    expected["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["rule"].as_str().unwrap().to_owned(),
                usize::try_from(f["line"].as_u64().unwrap()).unwrap(),
                usize::try_from(f["column"].as_u64().unwrap()).unwrap(),
                usize::try_from(f["end_column"].as_u64().unwrap()).unwrap(),
                f["severity"].as_str().unwrap().to_owned(),
                f["matched"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// Findings as the reference reports them. 2.x ends a finding that spans lines at the end of
/// its first line, so Rust's exact end is converted to that for the comparison.
fn rows_actual(doc: &Document, text: &str) -> Vec<Row> {
    doc.findings
        .iter()
        .map(|f| {
            let end = f.extent.unwrap().end;
            let end_column = if end.line() == f.start.line() {
                end.column()
            } else {
                text.lines()
                    .nth(f.start.line() - 1)
                    .map_or(0, |l| l.chars().count())
                    + 1
            };
            (
                f.rule_id.to_string(),
                f.start.line(),
                f.start.column(),
                end_column,
                format!("{:?}", f.severity).to_lowercase(),
                f.matched_text.clone(),
            )
        })
        .collect()
}

/// The facts on which Rust and the reference differ for one document, by name.
fn differences(doc: &Document, text: &str, expected: &Value) -> BTreeMap<&'static str, String> {
    let mut out = BTreeMap::new();
    let (want, got) = (rows_expected(expected), rows_actual(doc, text));
    // `end` is only reported when the start agrees, so it names the end column alone.
    let start_agrees = keyed(want.clone(), 1) == keyed(got.clone(), 1);
    for (name, level) in [("end", 0), ("position", 1), ("findings", 2)] {
        if name == "end" && !start_agrees {
            continue;
        }
        let (w, g) = (keyed(want.clone(), level), keyed(got.clone(), level));
        if w != g {
            let missing: Vec<_> = w.iter().filter(|r| !g.contains(r)).collect();
            let extra: Vec<_> = g.iter().filter(|r| !w.contains(r)).collect();
            out.insert(name, format!("missing {missing:?}, extra {extra:?}"));
        }
    }
    let count = |k: &str| usize::try_from(expected[k].as_u64().unwrap()).unwrap();
    for (name, actual) in [
        ("words", doc.words),
        ("sentences", doc.sentences),
        ("paragraphs", doc.paragraphs),
        ("errors", doc.errors),
        ("warnings", doc.warnings),
        ("suggestions", doc.suggestions),
    ] {
        if count(name) != actual {
            out.insert(name, format!("want {}, got {actual}", count(name)));
        }
    }
    if (expected["score"].as_f64().unwrap() - doc.score.get()).abs() > f64::EPSILON {
        out.insert(
            "score",
            format!("want {}, got {}", expected["score"], doc.score.get()),
        );
    }
    let reasons: Vec<&str> = expected["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    if reasons != doc.gate.reasons() {
        out.insert(
            "gate",
            format!("want {reasons:?}, got {:?}", doc.gate.reasons()),
        );
    }
    out
}

#[test]
fn rust_matches_the_2x_engine_on_the_interim_fixture() {
    let fixture = read(&manifest().join("tests/fixtures/engine-parity.json"));
    let divergences: BTreeMap<String, Divergence> =
        serde_json::from_value(read(&manifest().join("tests/fixtures/divergences.json"))).unwrap();
    let engine = engine(&fixture);
    let mut problems = Vec::new();
    let documents = fixture["documents"].as_array().unwrap();
    for entry in documents {
        let name = entry["name"].as_str().unwrap();
        let text = entry["text"].as_str().unwrap();
        let doc = engine.lint(&Input::markdown(name, text)).unwrap();
        let diff = differences(&doc, text, &entry["expected"]);
        let allowed: &[String] = divergences.get(name).map_or(&[], |d| &d.ignore);
        // `position` is the exact comparison. `findings` ignores positions, so ignoring
        // `position` still compares which rules matched which text.
        for (fact, detail) in &diff {
            if !allowed.iter().any(|a| a == fact) {
                problems.push(format!("{name}: {fact}: {detail}"));
            }
        }
        for fact in allowed {
            if !diff.contains_key(fact.as_str()) {
                problems.push(format!(
                    "{name}: `{fact}` now agrees; remove it from divergences.json"
                ));
            }
        }
    }
    println!(
        "{} documents, {} listed",
        documents.len(),
        divergences.len()
    );
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn findings_point_at_their_matched_text() {
    let fixture = read(&manifest().join("tests/fixtures/engine-parity.json"));
    let engine = engine(&fixture);
    let mut checked = 0;
    for entry in fixture["documents"].as_array().unwrap() {
        let name = entry["name"].as_str().unwrap();
        let text = entry["text"].as_str().unwrap();
        let doc = engine.lint(&Input::markdown(name, text)).unwrap();
        for f in &doc.findings {
            let span = f.extent.unwrap().span;
            let source = &text[span.start()..span.end()];
            // Projected text differs from the source for entities, markup and soft wraps.
            // A match that holds none of them reads back exactly.
            if !f.matched_text.contains(['\n', '\0', '&', '*', '[', '`'])
                && !source.contains(['\n', '&', '*', '[', '`'])
            {
                assert_eq!(source, f.matched_text, "{name} {}", f.rule_id);
                checked += 1;
            }
        }
    }
    assert!(checked > 500, "only {checked} findings were checked");
}
