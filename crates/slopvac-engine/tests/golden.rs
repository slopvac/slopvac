//! The engine against the golden suite (`tests/golden/v3`, slopvac-8l4.2.7).
//!
//! The suite records what the Python 2.x reference reports for each input at each profile,
//! reduced to one projection (`scripts/contract/golden.py`). This test lints the same inputs
//! with the same profile and the `core:legacy-<profile>` style that the parity mode runs, and
//! compares the projection fact by fact. `golden-divergences.json` lists the facts that differ
//! on purpose. A listed fact must still differ, and every other fact must agree.
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

/// Every fact `differences` can report, in the order of the agreement table.
const FACTS: [&str; 14] = [
    "findings",
    "position",
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
    "unchecked",
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
    let name = entry.path.file_name().unwrap().to_str().unwrap();
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

/// One sorted row for each finding of `doc`.
fn rows(doc: &Value, key: impl Fn(&Value) -> String) -> Vec<String> {
    let mut rows: Vec<String> = doc["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(key)
        .collect();
    rows.sort();
    rows
}

/// Rules of a kind the engine does not run yet (structure) or runs only with a blocklist
/// (vocabulary): the golden findings of these rules have no counterpart to compare.
fn unrun_rules() -> Vec<String> {
    let core = root().join("crates/slopvac-rules/tests/fixtures/core-v3");
    // The spelling rule is generated from the locale (`locale.rs` tests it); the core data has
    // no such rule, so the engine built here never reports it.
    let mut ids = vec!["core:ste-words.spelling".to_owned()];
    for entry in std::fs::read_dir(core).unwrap() {
        let doc = read_json(&entry.unwrap().path());
        for rule in doc["rules"].as_array().into_iter().flatten() {
            if matches!(
                rule["match"]["kind"].as_str(),
                Some("structure" | "vocabulary")
            ) {
                ids.push(format!(
                    "core:{}.{}",
                    doc["id"].as_str().unwrap(),
                    rule["id"].as_str().unwrap()
                ));
            }
        }
    }
    ids
}

/// Make the two documents comparable. A finding whose golden span is null had its code spans
/// masked by 2.x, so its matched text, message and end cannot be proved: both sides drop them.
/// Golden findings of unrun rules are removed; the count is returned.
fn prepare(want: &mut Value, got: &mut Value, unrun: &[String]) -> usize {
    let findings = want["document"]["findings"].as_array_mut().unwrap();
    let before = findings.len();
    findings.retain(|f| {
        let id = f["rule_id"].as_str().unwrap();
        !unrun.iter().any(|r| r == id)
    });
    let removed = before - findings.len();
    let masked: Vec<(String, String, String)> = findings
        .iter()
        .filter(|f| field(f, "span").is_null())
        .map(|f| (s(f, "rule_id"), s(f, "line"), s(f, "column")))
        .collect();
    let blank = |f: &mut Value| {
        let o = f.as_object_mut().unwrap();
        for k in ["matched_text", "message", "span", "end_line", "end_column"] {
            o.insert(k.into(), Value::Null);
        }
    };
    for f in findings.iter_mut() {
        if field(f, "span").is_null() {
            blank(f);
        }
    }
    for f in got["findings"].as_array_mut().unwrap() {
        if masked.contains(&(s(f, "rule_id"), s(f, "line"), s(f, "column"))) {
            blank(f);
        }
    }
    removed
}

/// The facts that follow from all findings of a document, so they are not compared when a
/// golden finding has no counterpart.
const DERIVED: [&str; 7] = [
    "tallies",
    "density",
    "score",
    "categories",
    "gate",
    "exit",
    "unchecked",
];

/// How the engine's document differs from the golden one, by fact. The facts, from coarse to
/// exact: `findings` (rule, category, severity, matched text), `position` (start line and
/// column), `end` (end line, end column and byte span), `message`, `ai_signal`,
/// `replacement`, `counts` (words, sentences, paragraphs), `tallies` (totals by severity),
/// `density`, `score`, `categories`, `gate` and `exit`.
fn differences(want: &Value, got: &Value) -> BTreeMap<&'static str, String> {
    let mut out = BTreeMap::new();
    let w = &want["document"];
    let mut check = |name: &'static str, a: Vec<String>, b: Vec<String>| {
        if a != b {
            let missing: Vec<_> = a.iter().filter(|x| !b.contains(x)).take(500).collect();
            let extra: Vec<_> = b.iter().filter(|x| !a.contains(x)).take(500).collect();
            out.insert(name, format!("missing {missing:?}, extra {extra:?}"));
        }
    };
    let loose = |f: &Value| {
        format!(
            "{} {} {} {}",
            s(f, "rule_id"),
            s(f, "category"),
            s(f, "severity"),
            s(f, "matched_text")
        )
    };
    check("findings", rows(w, loose), rows(got, loose));
    let keyed = |doc: &Value, facts: &[&str]| {
        rows(doc, |f| {
            let mut row = format!("{} {} {}", s(f, "rule_id"), s(f, "line"), s(f, "column"));
            for k in facts {
                row = format!("{row} {}", s(f, k));
            }
            row
        })
    };
    check("position", keyed(w, &[]), keyed(got, &[]));
    let end_of = |doc: &Value| keyed(doc, &["end_line", "end_column", "span"]);
    check("end", end_of(w), end_of(got));
    for (name, fact) in [
        ("message", "message"),
        ("ai_signal", "ai_signal"),
        ("replacement", "replacement"),
    ] {
        check(name, keyed(w, &[fact]), keyed(got, &[fact]));
    }
    for (name, keys) in [
        ("counts", &["words", "sentences", "paragraphs"][..]),
        (
            "tallies",
            &["total_findings", "errors", "warnings", "suggestions"],
        ),
        ("density", &["per_100_words", "ai_register", "prose"]),
        ("score", &["score"]),
        ("categories", &["categories"]),
        ("gate", &["gate"]),
        ("unchecked", &["unchecked"]),
    ] {
        for k in keys {
            if field(w, k) != field(got, k) {
                out.insert(
                    name,
                    format!("{k}: want {}, got {}", field(w, k), field(got, k)),
                );
            }
        }
    }
    // The exit status of one document: 2 when a check did not run, else 0 or 1 by its gate.
    let exit = if got["unchecked"].as_array().unwrap().is_empty() {
        i64::from(!got["gate"]["passed"].as_bool().unwrap())
    } else {
        2
    };
    if want["exit_code"].as_i64().unwrap() != exit {
        out.insert("exit", format!("want {}, got {exit}", want["exit_code"]));
    }
    out
}

#[derive(Default)]
struct Tally {
    documents: usize,
    compared: BTreeMap<&'static str, usize>,
    agree: BTreeMap<&'static str, usize>,
    findings_want: usize,
    findings_exact: usize,
    findings_unrun: usize,
}

/// The facts that `golden-divergences.json` lists for a document: the entries for every input
/// (`*`), for an input at every profile (`*/<id>`) and for one run (`<profile>/<id>`).
fn allowed_facts(divergences: &BTreeMap<String, Value>, profile: &str, id: &str) -> Vec<String> {
    ["*".to_owned(), format!("*/{id}"), format!("{profile}/{id}")]
        .iter()
        .filter_map(|k| divergences.get(k))
        .flat_map(|d| d["ignore"].as_array().unwrap())
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

fn run_tier(tier: &str) {
    let divergences: BTreeMap<String, Value> = serde_json::from_value(read_json(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden-divergences.json"),
    ))
    .unwrap();
    let unrun = unrun_rules();
    let mut tally = Tally::default();
    let mut problems = Vec::new();
    for profile in PROFILES {
        let engine = legacy_engine(profile).with_rst(rst_options());
        for entry in inputs()
            .iter()
            .filter(|e| e.tier == tier && !skipped(&e.id))
        {
            let path = format!("tests/golden/v3/parity/{profile}/{}.json", entry.id);
            let mut want = read_json(&root().join(path));
            let Some(mut got) = lint(&engine, entry) else {
                continue;
            };
            let removed = prepare(&mut want, &mut got, &unrun);
            let mut diff = differences(&want, &got);
            // With a golden finding unmatched, the facts that follow from all findings differ
            // by construction: they are not compared.
            let compared: Vec<&'static str> = FACTS
                .into_iter()
                .filter(|f| removed == 0 || !DERIVED.contains(f))
                .collect();
            diff.retain(|f, _| compared.contains(f));
            let key = format!("{profile}/{}", entry.id);
            let allowed = allowed_facts(&divergences, profile, &entry.id);
            tally.documents += 1;
            tally.findings_unrun += removed;
            for fact in &compared {
                *tally.compared.entry(fact).or_default() += 1;
                if !diff.contains_key(fact) {
                    *tally.agree.entry(fact).or_default() += 1;
                }
            }
            let exact = |d: &Value| {
                rows(d, |f| {
                    format!(
                        "{} {} {} {}",
                        s(f, "rule_id"),
                        s(f, "line"),
                        s(f, "column"),
                        s(f, "severity")
                    )
                })
            };
            let (w, g) = (exact(&want["document"]), exact(&got));
            tally.findings_want += w.len();
            tally.findings_exact += w.iter().filter(|r| g.contains(r)).count();
            for (fact, detail) in &diff {
                if !allowed.iter().any(|a| a == fact) {
                    problems.push(format!("{key}: {fact}: {detail}"));
                }
            }
            for fact in &allowed {
                let fact = fact.as_str();
                if compared.contains(&fact) && !diff.contains_key(fact) {
                    problems.push(format!(
                        "{key}: `{fact}` now agrees; remove it from golden-divergences.json"
                    ));
                }
            }
        }
    }
    let agree: Vec<String> = FACTS
        .iter()
        .map(|f| {
            let n = tally.agree.get(f).copied().unwrap_or(0);
            format!("{f} {n}/{}", tally.compared.get(f).copied().unwrap_or(0))
        })
        .collect();
    println!(
        "AGREEMENT {tier} ({} runs): {}; findings at the golden position and severity {}/{}; \
         golden findings of unrun rules {}",
        tally.documents,
        agree.join(", "),
        tally.findings_exact,
        tally.findings_want,
        tally.findings_unrun
    );
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}
#[test]
fn rule_examples_agree_with_the_golden() {
    run_tier("rule-examples");
}

#[test]
fn stacklok_pages_agree_with_the_golden() {
    run_tier("stacklok");
}

#[test]
fn issue_samples_agree_with_the_golden() {
    run_tier("issues");
}

#[test]
fn format_samples_agree_with_the_golden() {
    run_tier("formats");
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
