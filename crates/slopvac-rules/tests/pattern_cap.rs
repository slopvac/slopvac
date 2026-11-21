//! An authored pattern is capped at 8,192 UTF-8 bytes as written. Patterns the loader builds
//! are not.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "test crate: nothing public to document")]

use std::path::PathBuf;

use rstest::rstest;
use serde_json::{Value, json};
use slopvac_model::PackId;
use slopvac_rules::{CompileOptions, LoadErrorKind, LoadErrors, RuleSet};

fn rule(match_: &Value, bad: &str) -> Value {
    json!({
        "id": "r", "name": "n", "message": "m",
        "match": match_,
        "examples": [{"bad": bad, "good": "zzz"}],
        "citation": {"source": "s"}
    })
}

fn load(rule: &Value, options: &CompileOptions) -> (String, Result<RuleSet, LoadErrors>) {
    let text = serde_json::to_string_pretty(&json!({
        "schema_version": 3, "id": "cat", "title": "t", "description": "d", "rules": [rule]
    }))
    .unwrap();
    let sources = vec![(PathBuf::from("rules/cat.json"), text.clone())];
    let result = RuleSet::from_sources(&PackId::new("core").unwrap(), sources, options);
    (text, result)
}

fn pattern_rule(pattern: &str) -> Value {
    rule(&json!({"kind": "pattern", "pattern": pattern}), pattern)
}

fn substitution_rule(pattern: &str) -> Value {
    rule(
        &json!({"kind": "substitution", "substitutions": {pattern: "r"}}),
        pattern,
    )
}

#[rstest]
#[case::pattern_at_cap(pattern_rule(&"q".repeat(8192)))]
#[case::substitution_key_at_cap(substitution_rule(&"q".repeat(8192)))]
#[case::multibyte_pattern_at_cap_in_bytes(pattern_rule(&"é".repeat(4096)))]
fn an_authored_pattern_of_8192_bytes_loads(#[case] rule: Value) {
    let (_, result) = load(&rule, &CompileOptions::default());
    assert!(result.is_ok(), "{:?}", result.err());
}

#[rstest]
#[case::pattern(pattern_rule(&"q".repeat(8193)), 8193)]
#[case::substitution_key(substitution_rule(&"q".repeat(8193)), 8193)]
#[case::multibyte_counts_bytes_not_chars(pattern_rule(&"é".repeat(4097)), 8194)]
fn an_authored_pattern_over_8192_bytes_is_refused_at_its_rule(
    #[case] rule: Value,
    #[case] len: usize,
) {
    // The example text repeats the pattern, so it shares the pattern's first characters.
    let (text, result) = load(&rule, &CompileOptions::default());
    let errors = result.expect_err("the pattern is over the cap");
    // The line of the rule's `id`, found independently of the loader.
    let rules = text.lines().position(|l| l.contains("\"rules\"")).unwrap();
    let id = text
        .lines()
        .enumerate()
        .skip(rules)
        .find(|(_, l)| l.trim_start().starts_with("\"id\": \"r\""))
        .map(|(i, _)| i + 1)
        .unwrap();
    let [e] = errors.errors() else {
        panic!("expected one error: {errors}")
    };
    assert_eq!(e.file, PathBuf::from("rules/cat.json"));
    assert_eq!(e.line, Some(u32::try_from(id).unwrap()));
    assert_eq!(e.rule.as_deref(), Some("cat.r"));
    assert_eq!(e.kind, LoadErrorKind::PatternTooLong { len, limit: 8192 });
    let shown = errors.to_string();
    assert!(shown.contains("[rules-pattern-too-long]"), "{shown}");
    assert!(shown.contains("rules/cat.json"), "{shown}");
    assert!(!shown.contains("qqqq") && !shown.contains("éé"), "{shown}");
}

/// The text of a one-rule pattern category in YAML. The rule starts on line 6.
fn yaml_category(pattern: &str) -> String {
    format!(
        "schema_version: 3\nid: cat\ntitle: t\ndescription: d\nrules:\n  - id: r\n    name: n\n    message: m\n    match:\n      kind: pattern\n      pattern: \"{pattern}\"\n    examples:\n      - bad: \"{pattern}\"\n        good: zzz\n    citation:\n      source: s\n"
    )
}

/// The same category in TOML. The rule starts on line 7.
fn toml_category(pattern: &str) -> String {
    format!(
        "schema_version = 3\nid = \"cat\"\ntitle = \"t\"\ndescription = \"d\"\n\n[[rules]]\nid = \"r\"\nname = \"n\"\nmessage = \"m\"\ncitation = {{ source = \"s\" }}\n\n[rules.match]\nkind = \"pattern\"\npattern = \"{pattern}\"\n\n[[rules.examples]]\nbad = \"{pattern}\"\ngood = \"zzz\"\n"
    )
}

#[rstest]
#[case::yaml("rules/cat.yml", yaml_category(&"q".repeat(8193)), 6)]
#[case::toml("rules/cat.toml", toml_category(&"q".repeat(8193)), 7)]
fn the_refusal_line_is_the_rule_start_in_every_format(
    #[case] file: &str,
    #[case] text: String,
    #[case] line: u32,
) {
    let sources = vec![(PathBuf::from(file), text)];
    let errors = RuleSet::from_sources(
        &PackId::new("core").unwrap(),
        sources,
        &CompileOptions::default(),
    )
    .expect_err("the pattern is over the cap");
    let [e] = errors.errors() else {
        panic!("expected one error: {errors}")
    };
    assert_eq!(e.line, Some(line));
    assert_eq!(
        e.kind,
        LoadErrorKind::PatternTooLong {
            len: 8193,
            limit: 8192
        }
    );
}

#[test]
fn a_pattern_that_placeholder_expansion_makes_longer_than_the_cap_loads() {
    // 3,000 distinct four-letter words: the allow list keeps ASCII letters only.
    let words: Vec<String> = (0..3000_u32)
        .map(|i| {
            [i / 17576, i / 676 % 26, i / 26 % 26, i % 26]
                .map(|d| char::from(b'a' + u8::try_from(d).unwrap()))
                .iter()
                .collect()
        })
        .collect();
    let options = CompileOptions {
        allowed_words: words,
    };
    let rule = rule(
        &json!({
            "kind": "pattern",
            "pattern": r"\b(?!(?:ALLOWED)\b)[A-Z][a-z]+",
            "placeholders": {"ALLOWED": "allowed-words"},
            "ignore_case": false
        }),
        "Gizmo",
    );
    let (_, result) = load(&rule, &options);
    let set = result.expect("the authored pattern is short");
    let built = set
        .rules()
        .next()
        .unwrap()
        .matcher()
        .unwrap()
        .source()
        .len();
    assert!(built > 8192, "expanded pattern is {built} bytes");
}

#[test]
fn a_tokens_rule_whose_built_pattern_is_over_the_cap_loads() {
    let tokens: Vec<String> = (0..800).map(|i| format!("phrase number {i}")).collect();
    let rule = rule(
        &json!({"kind": "tokens", "tokens": tokens}),
        "phrase number 7",
    );
    let (_, result) = load(&rule, &CompileOptions::default());
    let set = result.expect("tokens are literals, not authored patterns");
    let built = set
        .rules()
        .next()
        .unwrap()
        .matcher()
        .unwrap()
        .source()
        .len();
    assert!(built > 8192, "built pattern is {built} bytes");
}
