//! The engine against the golden suite (`tests/golden/v3`, slopvac-8l4.2.7).
//!
//! The suite records what the Python 2.x reference reports for each input at each profile,
//! reduced to one projection (`scripts/contract/golden.py`). This test lints the same inputs
//! with the same profile and the `core:legacy-<profile>` style that the parity mode runs, and
//! compares the projection fact by fact. `golden-divergences.json` records, for each run that
//! differs, the exact difference: the golden findings Rust misses, the findings it adds, the
//! findings whose end, message, AI signal or replacement differ, and the values Rust reports
//! for the document facts that differ. A run passes only when its difference equals the
//! record, so an unrecorded difference and a recorded one that no longer occurs both fail.
//! `record` (ignored by default) rewrites the record from the engine; the reasons are written
//! by hand, one for each input.
//!
//! Inputs that the engine cannot lint yet are in `SKIPPED`, each with the bead that owns it.
//! The run summary and process exit status of a group belong to the CLI, which builds them.

#![allow(
    missing_docs,
    clippy::assert_is_empty,
    clippy::cloned_ref_to_slice_refs
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod common;

use common::{legacy_engine, read_json, root};

use serde_json::{Value, json};
use slopvac_engine::rst::{RstAdapter, RstOptions};
use slopvac_engine::{Engine, Input};
use slopvac_model::Document;

const PROFILES: [&str; 3] = ["strict", "normal", "relaxed"];

/// Inputs that no engine entry point reads yet: input id, the owning bead and why.
const SKIPPED: [(&str, &str, &str); 3] = [
    (
        "format-html",
        "slopvac-8l4.3.42",
        "the engine has no standalone HTML file kind (`Kind` stops at Markdown, MDX, RST and diagrams)",
    ),
    (
        "format-python-comments",
        "slopvac-8l4.3.26",
        "code-comment mode: comments are not extracted from source files yet",
    ),
    (
        "format-javascript-comments",
        "slopvac-8l4.3.26",
        "code-comment mode: comments are not extracted from source files yet",
    ),
];

/// The repository is the project: an interpreter inside it is never used.
fn rst_options() -> RstOptions {
    let mut options = RstOptions::default();
    options.project_roots = vec![root()];
    options
}

struct Entry {
    id: String,
    tier: String,
    path: PathBuf,
    language: String,
}

fn inputs() -> Vec<Entry> {
    let inputs: toml::Table = std::fs::read_to_string(root().join("tests/golden/v3/inputs.toml"))
        .unwrap()
        .parse()
        .unwrap();
    inputs["input"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            let text = |k: &str| i[k].as_str().unwrap().to_owned();
            Entry {
                id: text("id"),
                tier: text("tier"),
                path: root().join(text("path")),
                language: text("language"),
            }
        })
        .collect()
}

fn skipped(id: &str) -> bool {
    SKIPPED.iter().any(|(s, _, _)| *s == id)
}

/// The engine's report for one input in the shape of the golden document.
fn lint(engine: &Engine, entry: &Entry) -> Option<Value> {
    // The runner stages every Markdown input as `.md`; the corpus files end in `.corpus`.
    let staged = format!("{}.md", entry.id);
    let name = if entry.tier == "corpus" {
        staged.as_str()
    } else {
        entry.path.file_name().unwrap().to_str().unwrap()
    };
    let text = std::fs::read_to_string(&entry.path).unwrap();
    let input = match entry.language.as_str() {
        "markdown" => Input::markdown(name, &text),
        "mdx" => Input::mdx(name, &text),
        "rst" => {
            // RST needs docutils. CI sets the variable, so a missing interpreter fails there.
            if let Err(error) = RstAdapter::new(rst_options()).interpreter() {
                assert!(
                    std::env::var_os("SLOPVAC_RST_LIVE").is_none(),
                    "SLOPVAC_RST_LIVE is set but there is no usable docutils: {error}"
                );
                eprintln!("skipped {}: {error}", entry.id);
                return None;
            }
            Input::rst(name, &text)
        }
        other => panic!("{}: no entry point for {other}", entry.id),
    };
    Some(project(
        &engine
            .lint(&input)
            .unwrap_or_else(|e| panic!("{}: {e}", entry.id)),
        &text,
    ))
}

/// Reduce a document to the golden projection (`project_document`).
fn project(doc: &Document, text: &str) -> Value {
    let mut value = serde_json::to_value(doc).unwrap();
    for f in value["findings"].as_array_mut().unwrap() {
        let object = f.as_object_mut().unwrap();
        // A finding without a range (a metric) is a point: the golden records the byte offset
        // of its line and column as both ends.
        if !object.contains_key("span") {
            let (line, column) = (
                usize::try_from(object["line"].as_u64().unwrap()).unwrap(),
                usize::try_from(object["column"].as_u64().unwrap()).unwrap(),
            );
            let line_start: usize = text
                .split_inclusive('\n')
                .take(line - 1)
                .map(str::len)
                .sum();
            let offset = line_start
                + text[line_start..]
                    .chars()
                    .take(column - 1)
                    .map(char::len_utf8)
                    .sum::<usize>();
            object.insert("span".into(), json!({"start": offset, "end": offset}));
            object.insert("end_line".into(), object["line"].clone());
            object.insert("end_column".into(), object["column"].clone());
        }
        let replacement = object
            .get("fix")
            .and_then(|fix| fix["edits"].as_array())
            .map(|edits| {
                assert_eq!(edits.len(), 1, "a multi-edit fix has no 2.x replacement");
                edits[0]["text"].clone()
            });
        object.insert("replacement".into(), replacement.unwrap_or(Value::Null));
    }
    value
}

fn field<'a>(f: &'a Value, key: &str) -> &'a Value {
    f.get(key).unwrap_or(&Value::Null)
}

fn s(f: &Value, key: &str) -> String {
    field(f, key).to_string()
}

/// The identity of a finding: what it reports and where it starts. Its other facts are
/// compared once two findings share an identity.
fn identity(f: &Value) -> Value {
    json!({
        "rule_id": field(f, "rule_id"),
        "category": field(f, "category"),
        "severity": field(f, "severity"),
        "line": field(f, "line"),
        "column": field(f, "column"),
        "matched_text": field(f, "matched_text"),
    })
}

/// The facts of a finding other than its identity, each a class of the agreement table.
const ATTRIBUTES: [(&str, &[&str]); 4] = [
    ("end", &["end_line", "end_column", "span"]),
    ("message", &["message"]),
    ("ai_signal", &["ai_signal"]),
    ("replacement", &["replacement"]),
];

/// The scalar facts of a document and the class of the agreement table each belongs to.
const DOCUMENT: [(&str, &str); 12] = [
    ("words", "counts"),
    ("sentences", "counts"),
    ("paragraphs", "counts"),
    ("total_findings", "tallies"),
    ("errors", "tallies"),
    ("warnings", "tallies"),
    ("suggestions", "tallies"),
    ("per_100_words", "density"),
    ("ai_register", "density"),
    ("prose", "density"),
    ("score", "score"),
    ("gate", "gate"),
];

/// The classes of the agreement table, in order.
const CLASSES: [&str; 12] = [
    "findings",
    "end",
    "message",
    "ai_signal",
    "replacement",
    "counts",
    "tallies",
    "density",
    "score",
    "categories",
    "gate",
    "exit",
];

/// A finding whose golden span is null had its code spans masked by 2.x, so its end position
/// cannot be proved: both sides drop the end and the span. Every other fact is compared.
fn mask(want: &mut Value, got: &mut Value) {
    let findings = want["document"]["findings"].as_array_mut().unwrap();
    let masked: Vec<(String, String, String)> = findings
        .iter()
        .filter(|f| field(f, "span").is_null())
        .map(|f| (s(f, "rule_id"), s(f, "line"), s(f, "column")))
        .collect();
    let blank = |f: &mut Value| {
        let o = f.as_object_mut().unwrap();
        for k in ["span", "end_line", "end_column"] {
            o.insert(k.into(), Value::Null);
        }
    };
    findings
        .iter_mut()
        .filter(|f| field(f, "span").is_null())
        .for_each(blank);
    for f in got["findings"].as_array_mut().unwrap() {
        if masked.contains(&(s(f, "rule_id"), s(f, "line"), s(f, "column"))) {
            blank(f);
        }
    }
}

/// The exit status of one document: 2 when a check did not run, else 0 or 1 by its gate.
fn exit_of(doc: &Value) -> i64 {
    if doc["unchecked"].as_array().unwrap().is_empty() {
        i64::from(!doc["gate"]["passed"].as_bool().unwrap())
    } else {
        2
    }
}

/// A finding reduced to the facts of the golden projection.
fn reduced(f: &Value) -> Value {
    let mut out = identity(f);
    for (_, keys) in ATTRIBUTES {
        for k in keys {
            out[*k] = field(f, k).clone();
        }
    }
    out
}

/// The findings of a document, reduced, with their identity, in a fixed order.
fn sorted(doc: &Value) -> Vec<(String, Value)> {
    let mut all: Vec<(String, Value)> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (identity(f).to_string(), reduced(f)))
        .collect();
    all.sort_by_key(|(k, f)| (k.clone(), f.to_string()));
    all
}

/// Every way the engine's document differs from the golden one, as the exact values Rust
/// reports. An empty object means the two agree.
///
/// - `missing`: golden findings that Rust does not report (identity only);
/// - `extra`: Rust findings that the golden does not hold, with every fact;
/// - `changed`: findings with the same identity whose `end`, `message`, `ai_signal` or
///   `replacement` differ, with Rust's values;
/// - `document`: the scalar facts that differ, with Rust's values; `categories` holds the
///   Rust entry of each category that differs;
/// - `exit` and `unchecked`: Rust's exit status and notice codes.
fn differences(want: &Value, got: &Value) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    let w = &want["document"];
    let (mut left, mut right) = (sorted(w), sorted(got));
    let (mut missing, mut extra, mut changed) = (Vec::new(), Vec::new(), Vec::new());
    // Identical findings cancel first; what is left pairs by identity.
    let mut i = 0;
    while i < left.len() {
        if let Some(j) = right.iter().position(|r| r.1 == left[i].1) {
            right.remove(j);
            left.remove(i);
        } else {
            i += 1;
        }
    }
    for (key, f) in left {
        if let Some(j) = right.iter().position(|r| r.0 == key) {
            let (_, g) = right.remove(j);
            let mut attrs = serde_json::Map::new();
            for (_, keys) in ATTRIBUTES {
                for k in keys {
                    if field(&f, k) != field(&g, k) {
                        attrs.insert((*k).to_owned(), field(&g, k).clone());
                    }
                }
            }
            let mut entry = identity(&f);
            entry["rust"] = Value::Object(attrs);
            changed.push(entry);
        } else {
            missing.push(identity(&f));
        }
    }
    // An extra finding is recorded whole, so a change to any of its facts is a difference.
    extra.extend(right.into_iter().map(|(_, g)| g));
    for (name, list) in [("missing", missing), ("extra", extra), ("changed", changed)] {
        if !list.is_empty() {
            out.insert(name.to_owned(), Value::Array(list));
        }
    }
    let mut document = serde_json::Map::new();
    for (k, _) in DOCUMENT {
        if field(w, k) != field(got, k) {
            document.insert(k.to_owned(), field(got, k).clone());
        }
    }
    let by_name = |doc: &Value| -> BTreeMap<String, Value> {
        doc["categories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| (s(c, "category"), c.clone()))
            .collect()
    };
    let (wc, gc) = (by_name(w), by_name(got));
    let categories: serde_json::Map<String, Value> = gc
        .iter()
        .filter(|(name, c)| wc.get(*name) != Some(c))
        .map(|(name, c)| (name.trim_matches('"').to_owned(), c.clone()))
        .collect();
    assert_eq!(wc.len(), gc.len(), "the categories are the catalogue");
    if !categories.is_empty() {
        document.insert("categories".to_owned(), Value::Object(categories));
    }
    if !document.is_empty() {
        out.insert("document".to_owned(), Value::Object(document));
    }
    if want["exit_code"].as_i64().unwrap() != exit_of(got) {
        out.insert("exit".to_owned(), json!(exit_of(got)));
    }
    let messages = |doc: &Value| -> Vec<String> {
        doc["unchecked"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.get("message").map_or_else(|| n.clone(), Clone::clone))
            .map(|m| m.as_str().unwrap().to_owned())
            .collect()
    };
    if messages(w) != messages(got) {
        let codes: Vec<&Value> = got["unchecked"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| &n["code"])
            .collect();
        out.insert("unchecked".to_owned(), json!(codes));
    }
    out
}

/// The classes of the agreement table in which `diff` records a difference.
fn classes_of(diff: &serde_json::Map<String, Value>) -> Vec<&'static str> {
    let mut classes = Vec::new();
    let mut add = |c: &'static str| {
        if !classes.contains(&c) {
            classes.push(c);
        }
    };
    if diff.contains_key("missing") || diff.contains_key("extra") {
        add("findings");
    }
    for entry in diff
        .get("changed")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for (class, keys) in ATTRIBUTES {
            if keys.iter().any(|k| entry["rust"].get(k).is_some()) {
                add(class);
            }
        }
    }
    for (k, class) in DOCUMENT {
        if diff.get("document").is_some_and(|d| d.get(k).is_some()) {
            add(class);
        }
    }
    if diff
        .get("document")
        .is_some_and(|d| d.get("categories").is_some())
    {
        add("categories");
    }
    if diff.contains_key("exit") {
        add("exit");
    }
    classes
}

/// The golden modes a delta can list (`tests/golden/v3/README.md`).
const MODES: [&str; 3] = ["parity", "preserve", "policy"];

/// The deltas of `source` that apply in `mode`. A delta without `modes` applies to
/// `policy`. Every entry is validated first, so a misspelled mode is an error even when
/// no listed mode would select the entry.
fn deltas_for(source: &str, mode: &str) -> Result<Vec<toml::Table>, String> {
    let doc: toml::Table = source.parse().map_err(|e| format!("{e}"))?;
    let all: Vec<toml::Table> = doc
        .get("delta")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .map(|d| d.as_table().unwrap().clone())
        .collect();
    for d in &all {
        let Some(list) = d.get("modes") else { continue };
        for m in list.as_array().ok_or("`modes` is not a list")? {
            let m = m.as_str().ok_or("a mode is not a string")?;
            if !MODES.contains(&m) {
                return Err(format!("unknown mode `{m}`"));
            }
        }
    }
    Ok(all
        .into_iter()
        .filter(|d| match d.get("modes") {
            None => mode == "policy",
            Some(list) => list
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m.as_str() == Some(mode)),
        })
        .collect())
}

/// The expectation of the engine run: the parity golden with the reviewed deltas that list
/// `parity` applied, as `scripts/contract/golden.py` does for a 3.0 candidate.
fn apply_deltas(want: &mut Value, deltas: &[toml::Table], profile: &str, input: &str) {
    for d in deltas {
        if d["input"].as_str() != Some(input) || d["profile"].as_str() != Some(profile) {
            continue;
        }
        let delta = serde_json::to_value(d).unwrap();
        let doc = &mut want["document"];
        let findings = doc["findings"].as_array_mut().unwrap();
        if delta["kind"] == "added" {
            findings.push(delta["finding"].clone());
        } else {
            let hit: Vec<usize> = findings
                .iter()
                .enumerate()
                .filter(|(_, f)| {
                    f["rule_id"] == delta["rule_id"]
                        && f["line"] == delta["line"]
                        && f["column"] == delta["column"]
                })
                .map(|(i, _)| i)
                .collect();
            assert_eq!(hit.len(), 1, "delta does not match one finding: {delta}");
            if delta["kind"] == "removed" {
                findings.remove(hit[0]);
            } else {
                for (k, v) in delta["set"].as_object().unwrap() {
                    findings[hit[0]][k] = v.clone();
                }
            }
        }
        for (k, v) in delta
            .get("document")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            doc[k] = v.clone();
        }
        want["exit_code"] = json!(exit_of(&want["document"]));
    }
}

fn recorded_deltas(mode: &str) -> Vec<toml::Table> {
    let text = std::fs::read_to_string(root().join("tests/golden/v3/policy/deltas.toml")).unwrap();
    deltas_for(&text, mode).unwrap_or_else(|e| panic!("deltas.toml: {e}"))
}

fn divergence_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden-divergences.json")
}

/// Lint every input of `tier` at every profile and return the differences, by `profile/id`.
/// Runs that the engine skips (RST without docutils) are absent.
fn compute(tier: &str) -> BTreeMap<String, serde_json::Map<String, Value>> {
    let mut runs = BTreeMap::new();
    let deltas = recorded_deltas("parity");
    for profile in PROFILES {
        let engine = legacy_engine(profile).with_rst(rst_options());
        for entry in inputs()
            .iter()
            .filter(|e| e.tier == tier && !skipped(&e.id))
        {
            let path = format!("tests/golden/v3/parity/{profile}/{}.json", entry.id);
            let mut want = read_json(&root().join(path));
            apply_deltas(&mut want, &deltas, profile, &entry.id);
            let Some(mut got) = lint(&engine, entry) else {
                continue;
            };
            mask(&mut want, &mut got);
            runs.insert(format!("{profile}/{}", entry.id), differences(&want, &got));
        }
    }
    runs
}

fn check_tier(tier: &str) {
    let stored = read_json(&divergence_path());
    let runs = compute(tier);
    let mut problems = Vec::new();
    let mut agree: BTreeMap<&str, usize> = BTreeMap::new();
    let (mut exact_want, mut exact_got) = (0, 0);
    for (key, diff) in &runs {
        let expected = stored["runs"]
            .get(key)
            .cloned()
            .unwrap_or_else(|| json!({}));
        if expected != Value::Object(diff.clone()) {
            problems.push(format!(
                "{key}\n  recorded: {expected}\n  actual:   {}",
                Value::Object(diff.clone())
            ));
        }
        let classes = classes_of(diff);
        for class in CLASSES {
            if !classes.contains(&class) {
                *agree.entry(class).or_default() += 1;
            }
        }
        let count = |k: &str| diff.get(k).and_then(Value::as_array).map_or(0, Vec::len);
        let want_total = {
            let id = key.split_once('/').unwrap();
            let golden =
                read_json(&root().join(format!("tests/golden/v3/parity/{}/{}.json", id.0, id.1)));
            golden["document"]["findings"].as_array().unwrap().len()
        };
        exact_want += want_total;
        exact_got += want_total - count("missing");
    }
    let table: Vec<String> = CLASSES
        .iter()
        .map(|c| format!("{c} {}/{}", agree.get(c).copied().unwrap_or(0), runs.len()))
        .collect();
    println!(
        "AGREEMENT {tier} ({} runs): {}; golden findings that Rust reports {exact_got}/{exact_want}",
        runs.len(),
        table.join(", ")
    );
    assert!(
        problems.is_empty(),
        "{} runs differ from golden-divergences.json (record with `GOLDEN_RECORD=1 cargo test -p slopvac-engine --test golden record -- --ignored` and review the diff):\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn rule_examples_agree_with_the_golden() {
    check_tier("rule-examples");
}

#[test]
fn stacklok_pages_agree_with_the_golden() {
    check_tier("stacklok");
}

#[test]
fn issue_samples_agree_with_the_golden() {
    check_tier("issues");
}

#[test]
fn format_samples_agree_with_the_golden() {
    check_tier("formats");
}

#[test]
fn corpus_documents_agree_with_the_golden() {
    check_tier("corpus");
}

/// Every recorded difference belongs to a run of the suite and has a reason; every reason
/// belongs to a recorded difference.
#[test]
fn the_divergence_file_matches_the_suite() {
    let stored = read_json(&divergence_path());
    let ids: Vec<String> = inputs().into_iter().map(|e| e.id).collect();
    let mut recorded_inputs = Vec::new();
    for (key, diff) in stored["runs"].as_object().unwrap() {
        let (profile, id) = key.split_once('/').unwrap();
        assert!(PROFILES.contains(&profile), "{key}: unknown profile");
        assert!(ids.iter().any(|i| i == id), "{key}: not a golden input");
        assert!(!skipped(id), "{key}: the input is skipped");
        assert!(!diff.as_object().unwrap().is_empty(), "{key}: empty entry");
        recorded_inputs.push(id.to_owned());
    }
    let reasons = stored["reasons"].as_object().unwrap();
    for id in &recorded_inputs {
        let reason = reasons.get(id).and_then(Value::as_str);
        assert!(
            reason.is_some_and(|r| !r.is_empty() && !r.starts_with("TODO")),
            "{id}: no reason"
        );
    }
    for id in reasons.keys() {
        assert!(
            recorded_inputs.contains(id),
            "{id}: a reason without a recorded difference"
        );
    }
}

/// Rewrite `runs` in `golden-divergences.json` from the current engine, keeping the reasons.
/// Run it by hand, then read the diff: each change is a new or changed difference.
#[test]
#[ignore = "writes tests/golden-divergences.json; run with GOLDEN_RECORD=1"]
fn record() {
    assert!(
        std::env::var_os("GOLDEN_RECORD").is_some(),
        "set GOLDEN_RECORD=1 to rewrite the file"
    );
    assert!(
        RstAdapter::new(rst_options()).interpreter().is_ok(),
        "recording needs docutils for the RST sample"
    );
    let old = read_json(&divergence_path());
    let mut runs = serde_json::Map::new();
    let mut reasons = serde_json::Map::new();
    for tier in ["rule-examples", "stacklok", "issues", "formats", "corpus"] {
        for (key, diff) in compute(tier) {
            if diff.is_empty() {
                continue;
            }
            let id = key.split_once('/').unwrap().1.to_owned();
            let reason = old["reasons"]
                .get(&id)
                .cloned()
                .unwrap_or_else(|| json!("TODO: explain"));
            reasons.insert(id, reason);
            runs.insert(key, Value::Object(diff));
        }
    }
    let file = json!({"reasons": reasons, "runs": runs});
    std::fs::write(
        divergence_path(),
        serde_json::to_string_pretty(&file).unwrap() + "\n",
    )
    .unwrap();
}

/// Every finding that the markup does not change reads back from its byte span as its matched
/// text. This checks the span of Rust itself, which the golden cannot ground for masked findings.
#[test]
fn findings_point_at_their_matched_text() {
    let engine = legacy_engine("normal");
    let mut checked = 0;
    for entry in inputs().iter().filter(|e| e.language == "markdown") {
        let text = std::fs::read_to_string(&entry.path).unwrap();
        let doc = engine.lint(&Input::markdown(&entry.id, &text)).unwrap();
        for f in &doc.findings {
            // A metric finding has a line and no range.
            let Some(extent) = f.extent else { continue };
            let source = &text[extent.span.start()..extent.span.end()];
            // Projected text differs from the source for entities, markup and soft wraps.
            if !f.matched_text.contains(['\n', '\0', '&', '*', '[', '`'])
                && !source.contains(['\n', '&', '*', '[', '`'])
            {
                assert_eq!(source, f.matched_text, "{} {}", entry.id, f.rule_id);
                checked += 1;
            }
        }
    }
    assert!(checked > 500, "only {checked} findings were checked");
}

#[test]
fn every_skipped_input_is_in_the_suite() {
    let ids: Vec<String> = inputs().into_iter().map(|e| e.id).collect();
    for (id, _, _) in SKIPPED {
        assert!(ids.iter().any(|i| i == id), "{id} is not a golden input");
    }
}

/// An engine that reports nothing must not pass: the recorded differences are exact, so an
/// empty report differs from every golden document that holds findings.
#[test]
fn an_empty_report_is_a_difference() {
    let want = read_json(&root().join("tests/golden/v3/parity/normal/issue-254.json"));
    let mut got = project(
        &legacy_engine("normal")
            .lint(&Input::markdown("empty.md", ""))
            .unwrap(),
        "",
    );
    got["findings"] = json!([]);
    let diff = differences(&want, &got);
    assert_eq!(diff["missing"].as_array().unwrap().len(), 3);
    assert!(diff.contains_key("document"));
}

fn base_pair() -> (Value, Value) {
    let want = read_json(&root().join("tests/golden/v3/parity/normal/issue-254.json"));
    let got = want["document"].clone();
    (want, got)
}

fn set_first(doc: &mut Value, key: &str, value: Value) {
    doc["findings"][0][key] = value;
}

#[test]
fn identical_documents_have_no_difference() {
    let (want, got) = base_pair();
    assert!(differences(&want, &got).is_empty());
}

/// A change to any fact of a finding the golden also holds is recorded under `changed`.
#[test]
fn a_changed_fact_of_a_matched_finding_is_a_difference() {
    for (key, value) in [
        ("message", json!("other")),
        ("replacement", json!("other")),
        ("ai_signal", json!("strong")),
        ("end_column", json!(999)),
    ] {
        let (want, mut got) = base_pair();
        set_first(&mut got, key, value);
        assert!(differences(&want, &got).contains_key("changed"), "{key}");
    }
}

/// An extra finding is recorded whole: a changed fact makes a different record.
#[test]
fn a_changed_fact_of_an_extra_finding_changes_the_record() {
    let (mut want, got) = base_pair();
    want["document"]["findings"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    let recorded = differences(&want, &got);
    assert_eq!(recorded["extra"].as_array().unwrap().len(), 1);
    for (key, value) in [
        ("message", json!("other")),
        ("replacement", json!("other")),
        ("matched_text", json!("other")),
        ("end_column", json!(999)),
        ("span", json!({"start": 1, "end": 2})),
    ] {
        let mut changed = got.clone();
        set_first(&mut changed, key, value);
        assert_ne!(differences(&want, &changed), recorded, "{key}");
    }
}

/// 2.x masks the end of a finding with code spans; the rest of that finding is still compared.
#[test]
fn a_masked_finding_is_compared_except_for_its_end() {
    let (mut want, got) = base_pair();
    for key in ["span", "end_line", "end_column"] {
        set_first(&mut want["document"], key, Value::Null);
    }
    let masked = |want: &Value, got: &Value| {
        let (mut want, mut got) = (want.clone(), got.clone());
        mask(&mut want, &mut got);
        differences(&want, &got)
    };
    assert!(masked(&want, &got).is_empty());
    let mut other_end = got.clone();
    set_first(&mut other_end, "end_column", json!(999));
    assert!(masked(&want, &other_end).is_empty());
    for (key, value) in [("message", json!("other")), ("replacement", json!("other"))] {
        let mut changed = got.clone();
        set_first(&mut changed, key, value);
        assert!(!masked(&want, &changed).is_empty(), "{key}");
    }
}

/// 2.x loses the matched text of a sentence with inline code followed by a parenthesis and
/// reports it empty. The native finding must carry the whole sentence, once.
#[test]
fn a_sentence_with_inline_code_and_a_parenthesis_matches_whole() {
    let text = "Install the snapshot reviewer with `cargo binstall cargo-insta` (or `cargo install cargo-insta --locked`) before you change a snapshot.\n";
    let doc = legacy_engine("normal")
        .lint(&Input::markdown("insta.md", text))
        .unwrap();
    let hits: Vec<&str> = doc
        .findings
        .iter()
        .filter(|f| f.rule_id.as_str() == "core:ste-procedural.condition-after-command")
        .map(|f| f.matched_text.as_str())
        .collect();
    assert_eq!(
        hits,
        [
            "Install the snapshot reviewer with `cargo binstall cargo-insta` (or `cargo install cargo-insta --locked`) before you change a snapshot."
        ]
    );
}

const SAMPLE: &str = r#"
[[delta]]
input = "a"
[[delta]]
input = "b"
modes = ["parity", "policy"]
[[delta]]
input = "c"
modes = ["parity", "preserve", "policy"]
"#;

fn sample_inputs(mode: &str) -> Vec<String> {
    deltas_for(SAMPLE, mode)
        .unwrap()
        .iter()
        .map(|d| d["input"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_delta_without_modes_applies_to_policy_only() {
    assert_eq!(sample_inputs("preserve"), ["c"]);
    assert!(sample_inputs("policy").contains(&"a".to_owned()));
    assert!(!sample_inputs("parity").contains(&"a".to_owned()));
}

#[test]
fn a_delta_applies_to_the_modes_it_lists() {
    assert_eq!(sample_inputs("parity"), ["b", "c"]);
    assert_eq!(sample_inputs("policy"), ["a", "b", "c"]);
}

#[test]
fn a_delta_that_lists_only_a_misspelled_mode_is_an_error() {
    let source = "[[delta]]\ninput = \"a\"\nmodes = [\"polcy\"]\n";
    for mode in MODES {
        assert_eq!(
            deltas_for(source, mode).unwrap_err(),
            "unknown mode `polcy`"
        );
    }
}

#[test]
fn every_recorded_delta_names_known_modes() {
    for mode in MODES {
        recorded_deltas(mode);
    }
}

#[test]
fn a_parity_delta_moves_the_expected_findings() {
    let (mut want, _) = base_pair();
    let first = want["document"]["findings"][0].clone();
    let source = format!(
        "[[delta]]\ninput = \"issue-254\"\nprofile = \"normal\"\nkind = \"removed\"\nrule_id = {}\nline = {}\ncolumn = {}\nmodes = [\"parity\"]\n",
        first["rule_id"], first["line"], first["column"]
    );
    let deltas = deltas_for(&source, "parity").unwrap();
    let before = want["document"]["findings"].as_array().unwrap().len();
    apply_deltas(&mut want, &deltas, "normal", "issue-254");
    assert_eq!(
        want["document"]["findings"].as_array().unwrap().len(),
        before - 1
    );
}
