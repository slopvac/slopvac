//! Loading reports every problem with its file and line, and validates examples.

#![allow(missing_docs, clippy::single_range_in_vec_init)]

use std::path::PathBuf;

use serde_json::{Value, json};
use slopvac_model::PackId;
use slopvac_rules::{CompileOptions, LoadErrorKind, LoadErrors, MatchKind, RuleSet};

fn pack() -> PackId {
    PackId::new("core").unwrap()
}

fn tokens_rule(id: &str, token: &str, examples: &Value) -> Value {
    json!({
        "id": id, "name": "n", "message": "m",
        "match": {"kind": "tokens", "tokens": [token]},
        "examples": examples,
        "citation": {"source": "s"}
    })
}

fn category(id: &str, rules: &[Value]) -> String {
    let doc = json!({
        "schema_version": 3, "id": id, "title": "t", "description": "d", "rules": rules
    });
    serde_json::to_string_pretty(&doc).unwrap()
}

fn load(files: &[(&str, String)]) -> Result<RuleSet, LoadErrors> {
    let sources = files
        .iter()
        .map(|(name, text)| (PathBuf::from(name), text.clone()));
    RuleSet::from_sources(&pack(), sources, &CompileOptions::default())
}

fn errors(files: &[(&str, String)]) -> LoadErrors {
    load(files).expect_err("loading should fail")
}

#[test]
fn a_bad_example_that_does_not_match_is_reported_at_the_rule() {
    // The rule's "id" line, found independently of the loader.
    let text = category(
        "cat",
        &[tokens_rule(
            "dull",
            "leverage",
            &json!([{"bad": "Use the tool.", "good": "Use it."}]),
        )],
    );
    let index = text
        .lines()
        .position(|l| l.trim_start().starts_with("\"id\": \"dull\""))
        .unwrap();
    let line = u32::try_from(index + 1).unwrap();
    let e = errors(&[("rules/cat.json", text)]);
    assert_eq!(e.errors().len(), 1);
    let e = &e.errors()[0];
    assert_eq!(e.file, PathBuf::from("rules/cat.json"));
    assert_eq!(e.line, Some(line));
    assert_eq!(e.rule.as_deref(), Some("cat.dull"));
    assert_eq!(
        e.kind,
        LoadErrorKind::BadExampleDoesNotMatch {
            index: 0,
            text: "Use the tool.".into()
        }
    );
    assert_eq!(
        e.to_string(),
        format!(
            "rules/cat.json:{line}: cat.dull: example 0 'bad' text does not match its own pattern: \"Use the tool.\""
        )
    );
}

#[test]
fn a_good_example_that_matches_is_reported() {
    let text = category(
        "cat",
        &[tokens_rule(
            "dull",
            "leverage",
            &json!([{"bad": "We leverage it.", "good": "We still leverage it."}]),
        )],
    );
    let e = errors(&[("cat.json", text)]);
    assert!(matches!(
        &e.errors()[0].kind,
        LoadErrorKind::GoodExampleMatches { index: 0, .. }
    ));
}

#[test]
fn an_example_without_a_good_text_checks_only_the_bad_one() {
    let text = category(
        "cat",
        &[tokens_rule(
            "dull",
            "leverage",
            &json!([{"bad": "We leverage it."}]),
        )],
    );
    assert!(load(&[("cat.json", text)]).is_ok());
}

#[test]
fn a_pattern_that_does_not_compile_is_reported() {
    let rule = json!({
        "id": "broken", "name": "n", "message": "m",
        "match": {"kind": "pattern", "pattern": "(?<=a+)b"},
        "citation": {"source": "s"}
    });
    let e = errors(&[("cat.json", category("cat", &[rule]))]);
    assert!(matches!(&e.errors()[0].kind, LoadErrorKind::Compile(_)));
    assert_eq!(e.errors()[0].rule.as_deref(), Some("cat.broken"));
}

#[test]
fn every_broken_rule_is_reported_not_only_the_first() {
    let a = tokens_rule("a", "x", &json!([{"bad": "no hit"}]));
    let b = tokens_rule("b", "y", &json!([{"bad": "no hit either"}]));
    let c = tokens_rule("c", "z", &json!([{"bad": "a z here"}]));
    let e = errors(&[("cat.json", category("cat", &[a, b, c]))]);
    let rules: Vec<_> = e.errors().iter().map(|e| e.rule.clone().unwrap()).collect();
    assert_eq!(rules, ["cat.a", "cat.b"]);
}

#[test]
fn a_rule_id_repeated_in_a_category_is_reported() {
    let rule = tokens_rule("same", "x", &json!([]));
    let e = errors(&[("cat.json", category("cat", &[rule.clone(), rule]))]);
    assert_eq!(
        e.errors()[0].kind,
        LoadErrorKind::DuplicateRule("same".into())
    );
}

#[test]
fn the_same_rule_id_in_two_categories_is_fine() {
    let rule = tokens_rule("same", "x", &json!([]));
    let set = load(&[
        ("a.json", category("a", std::slice::from_ref(&rule))),
        ("b.json", category("b", &[rule])),
    ])
    .unwrap();
    assert_eq!(set.len(), 2);
}

#[test]
fn a_category_defined_in_two_files_is_reported_at_the_second() {
    let e = errors(&[
        ("one.json", category("cat", &[])),
        ("two.json", category("cat", &[])),
    ]);
    assert_eq!(e.errors()[0].file, PathBuf::from("two.json"));
    assert_eq!(
        e.errors()[0].kind,
        LoadErrorKind::DuplicateCategory("cat".into())
    );
}

#[test]
fn a_json_syntax_error_names_its_line() {
    let e = errors(&[("cat.json", "{\n  \"id\": \"cat\",\n  oops\n}".into())]);
    assert_eq!(e.errors()[0].line, Some(3));
    assert!(matches!(&e.errors()[0].kind, LoadErrorKind::Parse(_)));
}

#[test]
fn a_toml_error_names_its_line() {
    let text = "schema_version = 3\nid = \"cat\"\ntitle = \"t\"\nweight = \n";
    let e = errors(&[("cat.toml", text.into())]);
    assert_eq!(e.errors()[0].line, Some(4));
}

#[test]
fn a_yaml_error_names_its_line() {
    let text = "schema_version: 3\nid: cat\ntitle: t\ndescription: d\nrules:\n  - id: r\n    colour: red\n";
    let e = errors(&[("cat.yaml", text.into())]);
    let e = &e.errors()[0];
    assert!(e.to_string().contains("colour"), "{e}");
    assert_eq!(e.line, Some(7));
}

#[test]
fn an_unknown_key_is_a_parse_error_with_the_key_in_the_message() {
    let text = category("cat", &[tokens_rule("r", "x", &json!([]))])
        .replace("\"rules\"", "\"colour\": 1, \"rules\"");
    let e = errors(&[("cat.json", text)]);
    assert!(e.errors()[0].to_string().contains("unknown field `colour`"));
}

#[test]
fn a_file_with_an_unsupported_extension_is_reported() {
    let e = errors(&[("cat.txt", category("cat", &[]))]);
    assert_eq!(e.errors()[0].kind, LoadErrorKind::UnsupportedFormat);
}

#[test]
fn categories_load_from_json_yaml_and_toml_in_file_name_order() {
    let yaml = "schema_version: 3\nid: bee\ntitle: t\ndescription: d\nrules:\n  - id: r\n    name: n\n    message: m\n    match:\n      kind: judgement\n      question: q?\n    citation:\n      source: s\n";
    let toml = "schema_version = 3\nid = \"ay\"\ntitle = \"t\"\ndescription = \"d\"\nrules = []\n";
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("2-bee.yaml"), yaml).unwrap();
    std::fs::write(dir.path().join("1-ay.toml"), toml).unwrap();
    std::fs::write(dir.path().join("3-cee.json"), category("cee", &[])).unwrap();
    std::fs::write(dir.path().join("notes.md"), "not a rule file").unwrap();
    let set = RuleSet::load_dir(&pack(), dir.path(), &CompileOptions::default()).unwrap();
    let ids: Vec<_> = set
        .categories()
        .iter()
        .map(|c| c.id().to_string())
        .collect();
    assert_eq!(ids, ["ay", "bee", "cee"]);
    assert_eq!(set.rules_of_kind(MatchKind::Judgement).count(), 1);
}

#[test]
fn a_missing_directory_is_a_read_error() {
    let e = RuleSet::load_dir(
        &pack(),
        std::path::Path::new("/nonexistent/slopvac-rules"),
        &CompileOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(&e.errors()[0].kind, LoadErrorKind::Read(_)));
}

#[test]
fn an_allowed_word_exempts_the_heading_rule_it_is_injected_into() {
    let rule = json!({
        "id": "caps", "name": "n", "message": "m",
        "match": {
            "kind": "pattern",
            "pattern": r"\b(?!(?:ALLOWED)\b)[A-Z][a-z]+",
            "placeholders": {"ALLOWED": "allowed-words"},
            "ignore_case": false
        },
        "citation": {"source": "s"}
    });
    let sources = vec![(PathBuf::from("c.json"), category("cat", &[rule]))];
    let options = CompileOptions {
        allowed_words: vec!["Acme".into()],
    };
    let set = RuleSet::from_sources(&pack(), sources, &options).unwrap();
    let rule = set.rules().next().unwrap();
    assert_eq!(rule.find_all("Acme Gizmo").unwrap(), [5..10]);
}

#[test]
fn substitution_keys_keep_their_2x_meaning_through_the_loader() {
    let rule = json!({
        "id": "latin", "name": "n", "message": "m",
        "match": {"kind": "substitution", "substitutions": {
            "i.e.": "that is", "e.g.": "for example", "whitelist": "allowlist"
        }},
        "examples": [{"bad": "Use e.g. a hammer.", "good": "Use for example a hammer."}],
        "citation": {"source": "s"}
    });
    let set = load(&[("c.json", category("cat", &[rule]))]).unwrap();
    let rule = set.rules().next().unwrap();
    let text = "i.e. here, e.g. there, iXeX, whitelisted, anti-whitelist, whitelist.";
    let found: Vec<&str> = rule
        .find_all(text)
        .unwrap()
        .into_iter()
        .map(|r| &text[r])
        .collect();
    assert_eq!(found, ["i.e.", "e.g.", "whitelist"]);
}
