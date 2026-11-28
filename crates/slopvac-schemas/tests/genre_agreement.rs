//! The genre reader (`slopvac-context`) and the JSON Schema for a genre file give the same
//! verdict on every case of the conformance corpus, and on the cases below that the corpus does
//! not hold. The reader keeps its own checks and messages; the schema is what editors and other
//! tools read. Both come from the same constants and the same enums, and this test is what
//! proves it, for the generated file and for the committed one.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;
use serde_json::{json, Value};
use slopvac_context::GenreSpec;
use slopvac_schemas::generate;

const BASE: &str = "https://slopvac.github.io/slopvac/schemas/v3/";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A validator for the `genre.schema.json` among `files`, with every file in `files` reachable
/// by its `$id`.
fn validator(files: &[(String, Value)]) -> jsonschema::Validator {
    let mut refs = jsonschema::Registry::new();
    for (name, schema) in files {
        refs = refs.add(format!("{BASE}{name}"), schema).unwrap();
    }
    let genre = files.iter().find(|(name, _)| name == "genre.schema.json");
    assert!(genre.is_some(), "genre.schema.json is not among the files");
    jsonschema::options()
        .with_registry(&refs.prepare().unwrap())
        .build(&genre.unwrap().1)
        .unwrap()
}

fn committed() -> Vec<(String, Value)> {
    let dir = root().join("schemas/v3");
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        if name.ends_with(".schema.json") {
            out.push((
                name,
                serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap(),
            ));
        }
    }
    out
}

fn generated() -> Vec<(String, Value)> {
    generate()
        .unwrap()
        .into_iter()
        .filter(|g| g.file.ends_with(".schema.json"))
        .map(|g| (g.file.to_owned(), serde_json::from_str(&g.text).unwrap()))
        .collect()
}

fn corpus(outcome: &str) -> Vec<PathBuf> {
    let dir = root().join("schemas/v3/conformance/genre").join(outcome);
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| !p.ends_with("cases.toml"))
        .collect();
    assert!(!found.is_empty(), "no genre/{outcome} corpus file");
    found.sort();
    found
}

fn instance(path: &Path) -> Value {
    toml::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// What the reader says about a document: it parses and passes every check of `validate`.
fn reader_accepts(document: &Value) -> bool {
    serde_json::from_value::<GenreSpec>(document.clone()).is_ok_and(|spec| spec.validate().is_ok())
}

/// Every corpus file on which the reader and the schema differ, or on which the reader differs
/// from the directory (`valid` or `invalid`) that holds the file.
fn disagreements(schema: &jsonschema::Validator) -> Vec<String> {
    let mut found = Vec::new();
    let mut seen = 0;
    for outcome in ["valid", "invalid"] {
        let want = outcome == "valid";
        for path in corpus(outcome) {
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let id = format!("genre/{outcome}/{name}");
            let document = instance(&path);
            let reader = reader_accepts(&document);
            assert_eq!(reader, want, "{id}: the reader disagrees with the corpus");
            seen += 1;
            let by_schema = schema.is_valid(&document);
            if by_schema != want {
                found.push(format!("{id}: reader {reader}, schema {by_schema}"));
            }
        }
    }
    assert!(seen > 0, "no genre corpus case ran");
    found
}

#[test]
fn the_generated_schema_gives_the_readers_verdict_on_every_corpus_file() {
    let schema = validator(&generated());
    assert_eq!(disagreements(&schema), Vec::<String>::new());
}

#[test]
fn the_committed_schema_gives_the_readers_verdict_on_every_corpus_file() {
    let schema = validator(&committed());
    assert_eq!(disagreements(&schema), Vec::<String>::new());
}

/// A genre with the two required members, and `extra` set over it.
fn genre(extra: &Value) -> Value {
    let mut document = json!({"schema_version": 3, "id": "demo"});
    for (key, value) in extra.as_object().unwrap() {
        document[key] = value.clone();
    }
    document
}

fn signal(fields: &Value) -> Value {
    json!({"detect": {"signals": [fields]}})
}

/// Documents that the corpus does not hold, each with the verdict of the reader's own rules.
/// The schema must say the same for the generated and for the committed file.
#[rstest]
#[case::bare_level(json!({"categories": {"style": "warning"}}), true)]
#[case::table_with_every_dial(
    json!({"categories": {"core:style": {
        "severity": "error", "minimum_severity": "off", "max_per_100_words": 2.5, "weight": 0
    }}}),
    true
)]
#[case::budget_none_clears_a_budget(json!({"categories": {"style": {"max_per_100_words": "none"}}}), true)]
#[case::extension_key_in_a_table(json!({"categories": {"style": {"x-note": 1}}}), true)]
#[case::rule_level(json!({"rules": {"core:no-hedge": "suggestion", "Mixed.Case_1": "off"}}), true)]
#[case::qualified_extends(json!({"extends": ["core:article", "how-to"]}), true)]
#[case::front_matter_equals_each_scalar(
    json!({"detect": {"signals": [
        {"kind": "front-matter", "key": "a", "equals": "x"},
        {"kind": "front-matter", "key": "b", "equals": 1},
        {"kind": "front-matter", "key": "c", "equals": 1.5},
        {"kind": "front-matter", "key": "d", "equals": true}
    ]}}),
    true
)]
#[case::negative_budget(json!({"categories": {"style": {"max_per_100_words": -0.5}}}), false)]
#[case::unknown_budget_word(json!({"categories": {"style": {"max_per_100_words": "unlimited"}}}), false)]
#[case::negative_weight(json!({"categories": {"style": {"weight": -1}}}), false)]
#[case::unknown_dial(json!({"categories": {"style": {"colour": "red"}}}), false)]
#[case::unknown_level(json!({"categories": {"style": "fatal"}}), false)]
#[case::category_level_not_a_string(json!({"categories": {"style": 3}}), false)]
#[case::rule_level_unknown(json!({"rules": {"core:no-hedge": "fatal"}}), false)]
#[case::rule_level_is_not_a_table(json!({"rules": {"no-hedge": {"severity": "error"}}}), false)]
#[case::rule_key_with_a_space(json!({"rules": {"no hedge": "error"}}), false)]
#[case::rule_key_starts_with_a_hyphen(json!({"rules": {"-x": "error"}}), false)]
#[case::category_key_with_two_colons(json!({"categories": {"a:b:c": "error"}}), false)]
#[case::extends_repeats(json!({"extends": ["a", "a"]}), false)]
#[case::extends_capital(json!({"extends": ["Core"]}), false)]
#[case::extends_trailing_newline(json!({"extends": ["core\n"]}), false)]
#[case::styles_not_a_name(json!({"styles": ["core:"]}), false)]
#[case::unknown_member(json!({"colour": "red"}), false)]
#[case::extension_member(json!({"x-owner": "docs"}), true)]
#[case::empty_title(json!({"title": ""}), false)]
#[case::empty_surfaces(json!({"surfaces": ""}), false)]
#[case::empty_glob(signal(&json!({"kind": "path", "glob": ""})), false)]
#[case::negative_weight_signal(signal(&json!({"kind": "text", "pattern": "x", "weight": -1})), false)]
#[case::min_score_zero(json!({"detect": {"min_score": 0, "signals": [{"kind": "text", "pattern": "x"}]}}), false)]
#[case::equals_is_a_list(signal(&json!({"kind": "front-matter", "key": "a", "equals": ["x"]})), false)]
#[case::heading_takes_no_key(signal(&json!({"kind": "heading", "pattern": "x", "key": "a"})), false)]
#[case::front_matter_takes_no_pattern(signal(&json!({"kind": "front-matter", "key": "a", "pattern": "x"})), false)]
#[case::context_not_a_tag(json!({"contexts": ["Security"]}), false)]
#[case::profile_each_name(json!({"profile": "relaxed"}), true)]
fn the_schema_gives_the_readers_verdict_on_a_document(#[case] extra: Value, #[case] valid: bool) {
    let document = genre(&extra);
    assert_eq!(
        reader_accepts(&document),
        valid,
        "the reader, on {document}"
    );
    for (label, files) in [("generated", generated()), ("committed", committed())] {
        assert_eq!(
            validator(&files).is_valid(&document),
            valid,
            "the {label} schema, on {document}"
        );
    }
}
