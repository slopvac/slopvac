//! Parity with the 2.x Python engine on the interim fixture.
//!
//! STUB(slopvac-8l4.3.29): `fixtures/engine-parity.json` holds what the 2.x native engine
//! reported for a frozen corpus (`fixtures/generate.py`). The golden suite replaces it.
//! `fixtures/divergences.json` lists the documents that differ on purpose. Each entry gives
//! the reason and the facts that are not compared for it (`position`, `end`, `words`, `sentences`, `paragraphs`,
//! `score`, `gate`). Everything else is still compared. A listed fact must still differ, so
//! the list cannot go stale. `fewer` names rules for which Rust reports exactly that many
//! findings fewer than 2.x; the rest of the findings are still compared. `unmapped` lists,
//! per rule, the reviewed matched text of each 2.x finding that lost its text.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
use slopvac_engine::{CategoryDials, Engine, Input, Level, Settings};
use slopvac_model::{
    CategoryId, Document, GenreId, GenreResult, GenreSource, NonEmptyString, PackId, RuleId,
    Severity,
};
use slopvac_rules::{CompileOptions, RuleSet};
use slopvac_terms::Blocklist;

fn manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[expect(
    clippy::panic,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
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
#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
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

#[expect(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
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
    // The blocklist 2.x ships as its example; `generate.py` runs Vale with the same file.
    let blocklist =
        Blocklist::load(&manifest().join("../../packages/slopvac-lint/examples/blocklist.toml"))
            .unwrap();
    Engine::new(vec![set], settings)
        .unwrap()
        .with_blocklist(&blocklist)
        .unwrap()
}

type Row = (String, usize, usize, usize, String, String);

#[derive(serde::Deserialize)]
struct Divergence {
    #[expect(dead_code, reason = "the fixture records why the divergence exists")]
    reason: String,
    ignore: Vec<String>,
    /// Findings per rule that 2.x reports and Rust does not.
    #[serde(default)]
    fewer: BTreeMap<String, usize>,
    /// Findings for which 2.x lost the matched text and reports an empty `matched`: per rule,
    /// the reviewed text each such finding matches. Each listed text fills one empty 2.x row
    /// of that rule, in order, before the comparison, so Rust must report exactly that text.
    /// Every listed text must fill a row, and no empty row of a listed rule may be left over.
    #[serde(default)]
    unmapped: BTreeMap<String, Vec<String>>,
}

/// Fill the 2.x rows that lost their matched text with the reviewed texts. Returns a problem
/// when the empty rows of a listed rule and its listed texts do not pair up one to one.
fn fill_unmapped(want: &mut [Row], unmapped: &BTreeMap<String, Vec<String>>) -> Option<String> {
    for (rule, texts) in unmapped {
        let mut empty = want.iter_mut().filter(|r| &r.0 == rule && r.5.is_empty());
        for text in texts {
            match empty.next() {
                Some(row) => row.5.clone_from(text),
                None => return Some(format!("{rule}: no empty 2.x finding left for {text:?}")),
            }
        }
        if empty.next().is_some() {
            return Some(format!("{rule}: more empty 2.x findings than listed texts"));
        }
    }
    None
}

#[test]
fn unmapped_texts_pair_one_to_one_with_empty_rows() {
    let row = |m: &str| {
        (
            "r".to_owned(),
            1,
            1,
            1,
            "suggestion".to_owned(),
            m.to_owned(),
        )
    };
    let listed = |t: &[&str]| {
        BTreeMap::from([("r".to_owned(), t.iter().map(|s| (*s).to_owned()).collect())])
    };
    let mut want = vec![row(""), row("kept")];
    assert_eq!(fill_unmapped(&mut want, &listed(&["Install it."])), None);
    assert_eq!(want, vec![row("Install it."), row("kept")]);
    assert!(fill_unmapped(&mut [row("kept")], &listed(&["Install it."])).is_some());
    assert!(fill_unmapped(&mut [row(""), row("")], &listed(&["Install it."])).is_some());
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

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
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

/// The rows of `a` that `b` does not account for, counting repeats.
fn multiset_minus<'a>(a: &'a [Row], b: &[Row]) -> Vec<&'a Row> {
    let mut left = b.to_vec();
    a.iter()
        .filter(|row| {
            left.iter().position(|r| r == *row).is_none_or(|i| {
                left.swap_remove(i);
                false
            })
        })
        .collect()
}

/// Findings as the reference reports them. 2.x ends a finding that spans lines at the end of
/// its first line, so Rust's exact end is converted to that for the comparison.
fn rows_actual(doc: &Document, text: &str) -> Vec<Row> {
    doc.findings
        .iter()
        .map(|f| {
            // A metric finding has a line and no range: the fixture records its end as 0.
            let end = f.extent.map_or(f.start, |e| e.end);
            let end_column = if f.extent.is_none() {
                0
            } else if end.line() == f.start.line() {
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
#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn differences(
    doc: &Document,
    text: &str,
    expected: &Value,
    fewer: &BTreeMap<String, usize>,
    unmapped: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<&'static str, String> {
    let mut out = BTreeMap::new();
    let (mut want, got) = (rows_expected(expected), rows_actual(doc, text));
    if let Some(problem) = fill_unmapped(&mut want, unmapped) {
        out.insert("unmapped", problem);
    }
    // `end` is only reported when the start agrees, so it names the end column alone.
    let start_agrees = keyed(want.clone(), 1) == keyed(got.clone(), 1);
    for (name, level) in [("end", 0), ("position", 1), ("findings", 2)] {
        if name == "end" && !start_agrees {
            continue;
        }
        let (mut w, g) = (keyed(want.clone(), level), keyed(got.clone(), level));
        if name == "findings" {
            if !fewer.is_empty() && w == g {
                out.insert("fewer", "the findings now agree without it".to_owned());
            }
            for (rule, count) in fewer {
                for _ in 0..*count {
                    if let Some(i) = w.iter().position(|r| &r.0 == rule) {
                        w.remove(i);
                    }
                }
            }
        }
        if w != g {
            let (missing, extra) = (multiset_minus(&w, &g), multiset_minus(&g, &w));
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
        let no_fewer = BTreeMap::new();
        let fewer = divergences.get(name).map_or(&no_fewer, |d| &d.fewer);
        let no_unmapped = BTreeMap::new();
        let unmapped = divergences.get(name).map_or(&no_unmapped, |d| &d.unmapped);
        let diff = differences(&doc, text, &entry["expected"], fewer, unmapped);
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
            // A metric finding has a line and no range.
            let Some(extent) = f.extent else { continue };
            let span = extent.span;
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
