//! Context gates on rules, one table of documents per rule.
//!
//! Each `fixtures/contexts/*.md` file holds the cases for one rule. Rule PRs that gate a
//! rule with `contexts` add a file, or cases, here instead of writing a Rust test. The
//! format is Markdown with directives in single-line HTML comments:
//!
//! - `<!-- rule: <plugin>:<category>.<rule> -->` (once, first): the rule under test.
//! - `<!-- synthetic: {json} -->` (optional): the rule is a test-only rule. The object holds
//!   a `match` and any other rule keys (`contexts`, `scope`, ...). It becomes the rule `<rule>`
//!   in an engine of its own. Without it, the rule comes from the shipped core rules.
//! - `<!-- fires: <name> -->` or `<!-- silent: <name> -->`: starts a case. The lines up to
//!   the next case directive are the document. The rule must report at least one finding
//!   on a `fires` document and none on a `silent` document.
//!
//! The harness checks the rule's own findings only; other rules may fire on the document.

#![allow(missing_docs)]

mod common;

use std::path::{Path, PathBuf};

use common::{engine, rule, settings};
use serde_json::Value;
use slopvac_engine::{Engine, Input};
use slopvac_model::PackId;
use slopvac_rules::{CompileOptions, RuleSet};

struct Case {
    name: String,
    fires: bool,
    document: String,
}

struct Fixture {
    file: String,
    rule: String,
    synthetic: Option<Value>,
    cases: Vec<Case>,
}

fn directive(line: &str) -> Option<(&str, &str)> {
    line.strip_prefix("<!-- ")?
        .strip_suffix(" -->")?
        .split_once(": ")
}

fn parse(file: &str, text: &str) -> Fixture {
    let mut fixture = Fixture {
        file: file.to_owned(),
        rule: String::new(),
        synthetic: None,
        cases: Vec::new(),
    };
    for line in text.lines() {
        match directive(line) {
            Some(("rule", id)) => id.clone_into(&mut fixture.rule),
            Some(("synthetic", json)) => {
                fixture.synthetic =
                    Some(serde_json::from_str(json).unwrap_or_else(|e| panic!("{file}: {e}")));
            }
            Some((verb @ ("fires" | "silent"), name)) => fixture.cases.push(Case {
                name: name.to_owned(),
                fires: verb == "fires",
                document: String::new(),
            }),
            _ => {
                if let Some(case) = fixture.cases.last_mut() {
                    case.document.push_str(line);
                    case.document.push('\n');
                }
            }
        }
    }
    assert!(!fixture.rule.is_empty(), "{file}: no `rule` directive");
    assert!(!fixture.cases.is_empty(), "{file}: no cases");
    fixture
}

fn fixtures() -> Vec<Fixture> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/contexts");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            parse(&name, &std::fs::read_to_string(p).unwrap())
        })
        .collect()
}

fn core_engine() -> Engine {
    let core =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../slopvac-rules/tests/fixtures/core-v3");
    let set = RuleSet::load_dir(
        &PackId::new("core").unwrap(),
        &core,
        &CompileOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    Engine::new(vec![set], settings()).unwrap()
}

/// The engine for a fixture: its own for a synthetic rule, the core rules otherwise.
fn engine_for(fixture: &Fixture) -> Engine {
    let Some(synthetic) = &fixture.synthetic else {
        return core_engine();
    };
    // The shared builder puts the rule in category `cat`, so the fixture names it `<plugin>:cat.<id>`.
    let local = fixture
        .rule
        .strip_prefix("core:cat.")
        .unwrap_or_else(|| panic!("{}: a synthetic rule is `core:cat.<id>`", fixture.file));
    let mut extra = synthetic.clone();
    let matcher = extra
        .as_object_mut()
        .and_then(|o| o.remove("match"))
        .unwrap_or_else(|| panic!("{}: synthetic rule has no `match`", fixture.file));
    engine(&[rule(local, &matcher, &extra)])
}

#[test]
fn every_context_fixture_case_holds() {
    let fixtures = fixtures();
    assert!(!fixtures.is_empty());
    let mut failures = Vec::new();
    for fixture in &fixtures {
        let engine = engine_for(fixture);
        for case in &fixture.cases {
            let doc = engine
                .lint(&Input::markdown("a.md", &case.document))
                .unwrap();
            let count = doc
                .findings
                .iter()
                .filter(|f| f.rule_id.as_str() == fixture.rule)
                .count();
            if (count > 0) != case.fires {
                failures.push(format!(
                    "{} / {}: expected {}, rule reported {count} finding(s)",
                    fixture.file,
                    case.name,
                    if case.fires { "a finding" } else { "silence" },
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
