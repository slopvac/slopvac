//! The term-casing rule beyond the basics: fixes, isolation, rule-contract controls and
//! terms files. Expected spellings are worked out from the term lists, not the matcher.

#[expect(
    dead_code,
    reason = "each test binary uses the part of the shared helpers it needs"
)]
mod common;

use common::{matched, settings};
use serde_json::{Value, json};
use slopvac_engine::{Engine, Input, TERMS_PACK, fix_document, terms_rule_set};
use slopvac_model::PackId;

const PROJECT_TERMS: &str = "schema_version = 3\n\n[[terms]]\nterm = \"AcmeCloud\"\n\n[[terms]]\nterm = \"Zorbix\"\nvariants = [\"Zx\"]\n";

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn pack() -> PackId {
    PackId::new(TERMS_PACK).unwrap()
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn base() -> Engine {
    Engine::new(vec![terms_rule_set(&pack()).unwrap()], settings()).unwrap()
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn project_engine() -> Engine {
    let path = common::private_dir("terms").join("terms.toml");
    std::fs::write(&path, PROJECT_TERMS).unwrap();
    base().with_term_files(&[path]).unwrap()
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn lint_text(engine: &Engine, text: &str) -> Vec<String> {
    let doc = engine.lint(&Input::markdown("a.md", text)).unwrap();
    matched(&doc).into_iter().map(str::to_owned).collect()
}

#[test]
fn a_fix_writes_the_canonical_spelling_whatever_the_case_found() {
    let engine = project_engine();
    let input = Input::markdown("a.md", "Buy an Acmecloud from GITHUB.\n");
    let doc = engine.lint(&input).unwrap();
    let report = fix_document(&engine, &input, &doc, None).unwrap();
    assert_eq!(report.text, "Buy an AcmeCloud from GitHub.\n");
    assert_eq!(lint_text(&engine, &report.text), Vec::<String>::new());
}

#[test]
fn isolating_a_rule_keeps_the_project_terms() {
    let engine = project_engine();
    let id = terms_rule_set(&pack())
        .unwrap()
        .rules()
        .next()
        .unwrap()
        .id()
        .clone();
    let isolated = engine.isolate(&id).unwrap();
    // `Acmecloud` exists only in the project file; `Zx` is an accepted variant of Zorbix.
    assert_eq!(
        lint_text(&isolated, "An Acmecloud runs Zx and zx.\n"),
        ["Acmecloud", "zx"]
    );
}

#[test]
fn project_terms_files_come_before_the_built_in_list() {
    let path = common::private_dir("terms").join("terms.toml");
    std::fs::write(
        &path,
        "schema_version = 3\n\n[[terms]]\nterm = \"Github\"\n",
    )
    .unwrap();
    let engine = base().with_term_files(&[path]).unwrap();
    // The project spells it `Github`, so the built-in `GitHub` is the miscased one.
    assert_eq!(lint_text(&engine, "See Github and GitHub.\n"), ["GitHub"]);
}

#[test]
fn a_missing_terms_file_is_an_error() {
    let missing = common::private_dir("terms").join("nope.toml");
    assert!(base().with_term_files(&[missing]).is_err());
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn engine_with_rule(extra: &Value) -> Engine {
    let mut pack: Value = serde_json::from_str(include_str!("../data/terms-pack.json")).unwrap();
    for (k, v) in extra.as_object().unwrap() {
        pack["rules"][0][k] = v.clone();
    }
    let set = common::ruleset(vec![(
        std::path::PathBuf::from("terms.json"),
        pack.to_string(),
    )]);
    Engine::new(vec![set], settings()).unwrap()
}

#[test]
fn a_heading_scoped_rule_does_not_read_the_body() {
    let engine = engine_with_rule(&json!({"scope": "heading"}));
    let doc = engine
        .lint(&Input::markdown(
            "a.md",
            "# Using Github\n\nBody about Github.\n",
        ))
        .unwrap();
    let lines: Vec<usize> = doc.findings.iter().map(|f| f.start.line()).collect();
    assert_eq!(lines, [1]);
}

#[test]
fn the_rule_allowlist_exempts_a_spelling() {
    let engine = engine_with_rule(&json!({"allowlist": ["Github"]}));
    assert_eq!(
        lint_text(&engine, "See Github and Javascript.\n"),
        ["Javascript"]
    );
}

#[test]
fn a_rule_limited_to_a_text_type_skips_other_sentences() {
    let engine = engine_with_rule(&json!({"text_type": "procedural"}));
    assert_eq!(
        lint_text(
            &engine,
            "Open Github quickly.\n\nGithub opens quickly in the summer heat.\n"
        ),
        ["Github"]
    );
}
