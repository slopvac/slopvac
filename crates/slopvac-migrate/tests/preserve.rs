//! `--preserve-behavior`: expansion steps E1 to E6 of docs/contract/profiles.md,
//! section 4, and the override glob grammar.
// STUB(slopvac-8l4.16.11): vectors copied from PR #296 b138f5888a; re-verify when it lands.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use serde_json::{Map, Value, json};
use slopvac_migrate::overlap::build_regions;
use slopvac_migrate::{Error, Options, RenameTable, Rewrite, migrate_config};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn vectors() -> Value {
    let text =
        fs::read_to_string(crate_dir().join("tests/fixtures/preserve/vectors.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn preserve(text: &str) -> Result<Rewrite, Error> {
    migrate_config(
        text,
        "slopvac.toml",
        &RenameTable::bundled(),
        Options {
            preserve_behavior: true,
        },
    )
}

fn as_json(text: &str) -> Value {
    serde_json::to_value(toml::from_str::<toml::Table>(text).unwrap()).unwrap()
}

/// The layer fields of a config or block, in the reference's shape: a bare category
/// level becomes `{severity = level}` and empty tables are left out.
fn layer(from: &Map<String, Value>, to: &mut Map<String, Value>) {
    if let Some(Value::Object(cats)) = from.get("categories") {
        let cats: Map<String, Value> = cats
            .iter()
            .map(|(k, v)| match v {
                Value::String(_) => (k.clone(), json!({ "severity": v })),
                other => (k.clone(), other.clone()),
            })
            .collect();
        if !cats.is_empty() {
            to.insert("categories".into(), Value::Object(cats));
        }
    }
    if let Some(Value::Object(rules)) = from.get("rules")
        && !rules.is_empty()
    {
        to.insert("rules".into(), Value::Object(rules.clone()));
    }
}

/// The parts of a migrated config that the reference expansion produces.
fn expansion_of(text: &str) -> Value {
    let doc = as_json(text);
    let doc = doc.as_object().unwrap();
    let mut out = Map::new();
    for key in ["profile", "styles"] {
        if let Some(v) = doc.get(key) {
            out.insert(key.into(), v.clone());
        }
    }
    layer(doc, &mut out);
    if let Some(Value::Array(blocks)) = doc.get("overrides") {
        let blocks: Vec<Value> = blocks
            .iter()
            .map(|b| {
                let b = b.as_object().unwrap();
                let mut nb = Map::new();
                for key in ["files", "profile"] {
                    if let Some(v) = b.get(key) {
                        nb.insert(key.into(), v.clone());
                    }
                }
                layer(b, &mut nb);
                Value::Object(nb)
            })
            .collect();
        if !blocks.is_empty() {
            out.insert("overrides".into(), Value::Array(blocks));
        }
    }
    Value::Object(out)
}

fn expansions() -> Vec<(String, String, Value)> {
    vectors()["expansions"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, v)| {
            (
                name.clone(),
                v["input"].as_str().unwrap().to_owned(),
                v["expected"].clone(),
            )
        })
        .collect()
}

#[test]
fn expansion_matches_the_reference_for_each_scenario() {
    let cases = expansions();
    assert!(cases.len() >= 27);
    let wrong: Vec<String> = cases
        .iter()
        .filter(|(_, input, expected)| expansion_of(&preserve(input).unwrap().text) != *expected)
        .map(|(name, _, _)| name.clone())
        .collect();
    assert!(wrong.is_empty(), "differs from the reference: {wrong:?}");
}

#[test]
fn inline_override_blocks_expand_like_array_tables() {
    let text = "profile = \"strict\"\nrules = { \"orwell.not-un\" = \"warning\" }\n\
                overrides = [\n  { files = [\"docs/**\"], profile = \"normal\" },\n  \
                { files = [\"docs/api/**\"], profile = \"relaxed\" },\n]\n";
    let (_, _, expected) = expansions()
        .into_iter()
        .find(|(name, _, _)| name == "top-rule-ignored-only-in-a-nested-scope")
        .unwrap();
    assert_eq!(expansion_of(&preserve(text).unwrap().text), expected);
}

fn config_validator() -> jsonschema::Validator {
    let schemas = crate_dir().join("../../schemas/v3");
    let read = |name: &str| -> Value {
        serde_json::from_str(&fs::read_to_string(schemas.join(name)).unwrap()).unwrap()
    };
    let registry = jsonschema::Registry::new()
        .add(
            "https://srobroek.github.io/slopvac/schemas/v3/common.schema.json",
            read("common.schema.json"),
        )
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(&read("config.schema.json"))
        .unwrap()
}

#[test]
fn expanded_configs_validate_against_the_config_schema() {
    let validator = config_validator();
    let mut wrong = Vec::new();
    for (name, input, _) in expansions() {
        let doc = as_json(&preserve(&input).unwrap().text);
        let errors: Vec<String> = validator.iter_errors(&doc).map(|e| e.to_string()).collect();
        if !errors.is_empty() {
            wrong.push((name, errors));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn globs_outside_the_grammar_stop_with_the_overlap_code_and_name_the_block() {
    let cases = vectors()["errors"].as_object().unwrap().clone();
    assert_eq!(cases.len(), 8);
    for (name, case) in cases {
        let err = preserve(case["input"].as_str().unwrap()).unwrap_err();
        assert_eq!(err.code(), "migrate-override-overlap", "{name}");
        let block = case["block"].as_u64().unwrap();
        assert!(
            err.to_string().contains(&format!("`overrides[{block}]`")),
            "{name}: {err}"
        );
    }
}

#[test]
fn regions_match_the_reference() {
    for (name, case) in vectors()["regions"].as_object().unwrap() {
        let blocks: Vec<Vec<String>> = case["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| vec![g.as_str().unwrap().to_owned()])
            .collect();
        let got: Vec<Value> = build_regions(&blocks)
            .unwrap()
            .into_iter()
            .map(|r| json!({ "scope": r.scope.into_iter().collect::<Vec<_>>(), "files": r.files }))
            .collect();
        assert_eq!(Value::Array(got), case["regions"], "{name}");
    }
}

#[test]
fn disjoint_blocks_give_one_region_each_plus_the_root() {
    let blocks: Vec<Vec<String>> = (0..40).map(|i| vec![format!("dir{i}/**")]).collect();
    let regions = build_regions(&blocks).unwrap();
    assert_eq!(regions.len(), 41);
    let scopes: BTreeSet<_> = regions.iter().map(|r| r.scope.clone()).collect();
    assert_eq!(scopes.len(), 41);
}

#[test]
fn a_dropped_rule_entry_is_reported_in_a_note() {
    // orwell.not-un is excluded at relaxed, so 2.x ignored this entry.
    let rw = preserve("profile = \"relaxed\"\n[rules]\n\"orwell.not-un\" = \"error\"\n").unwrap();
    assert!(!rw.text.contains("orwell.not-un"), "{}", rw.text);
    assert!(
        rw.notes
            .iter()
            .any(|n| n.starts_with("dropped rules.orwell.not-un:")),
        "{:?}",
        rw.notes
    );
}

#[test]
fn authored_comments_and_other_override_keys_survive() {
    let rw = preserve(
        "profile = \"normal\"\n[rules]\n# keep this\n\"ste-verbs.passive-voice\" = \"error\"\n\
         [[overrides]]\nfiles = [\"docs/**\"]\nprofile = \"relaxed\"\n[overrides.locale]\ndefault = \"en-US\"\n",
    )
    .unwrap();
    assert!(
        rw.text
            .contains("# keep this\n\"core:ste-verbs.passive-voice\" = \"error\""),
        "{}",
        rw.text
    );
    let doc = as_json(&rw.text);
    let original: Vec<&Value> = doc["overrides"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b.get("profile").is_some())
        .collect();
    assert_eq!(original.len(), 1, "{}", rw.text);
    assert_eq!(original[0]["profile"], "relaxed");
    assert_eq!(original[0]["locale"], "en-US");
}

#[test]
fn a_rewritten_rule_entry_keeps_its_comments() {
    // 2.x ignored this entry (its category is off), so the strict default replaces it.
    let rw = preserve(
        "profile = \"strict\"\n[categories]\n\"prose-craft\" = \"off\"\n[rules]\n\
         # why\n\"prose-craft.latinisms\" = \"error\" # trailing\n",
    )
    .unwrap();
    assert!(
        rw.text
            .contains("# why\n\"core:prose-craft.latinisms\" = \"off\" # trailing\n"),
        "{}",
        rw.text
    );
}

/// The key and line of the only dropped setting.
fn only_dropped(text: &str) -> (String, usize) {
    match preserve(text) {
        Err(Error::Dropped(d)) if d.len() == 1 => (d[0].key.clone(), d[0].line),
        other => panic!("expected one Dropped, got {other:?}"),
    }
}

#[test]
fn unknown_names_are_dropped_settings_at_their_lines() {
    let block = "[[overrides]]\nfiles = [\"docs/**\"]\n";
    let cases = [
        ("profile = \"lax\"\n".to_owned(), "profile", 1),
        (
            format!("{block}profile = \"lax\"\n"),
            "overrides[0].profile",
            3,
        ),
        (
            "[categories]\n\"no-such\" = \"off\"\n".to_owned(),
            "categories.no-such",
            2,
        ),
        (
            format!("{block}[overrides.categories]\n\"no-such\" = \"off\"\n"),
            "overrides[0].categories.no-such",
            4,
        ),
        (
            "[rules]\n\"prose-craft.no-such-rule\" = \"error\"\n".to_owned(),
            "rules.prose-craft.no-such-rule",
            2,
        ),
        (
            format!("{block}[overrides.rules]\n\"prose-craft.no-such-rule\" = \"error\"\n"),
            "overrides[0].rules.prose-craft.no-such-rule",
            4,
        ),
    ];
    for (text, key, line) in cases {
        assert_eq!(only_dropped(&text), (key.to_owned(), line), "{text}");
    }
}

#[test]
fn the_overlap_error_gives_the_line_of_the_block() {
    let cases = [
        (
            "[[overrides]]\nfiles = [\"docs/**\"]\n\n[[overrides]]\nfiles = [\"a.md\"]\n",
            1,
            4,
        ),
        (
            "overrides = [\n  { files = [\"docs/**\"] },\n  { files = [\"a.md\"] },\n]\n",
            1,
            3,
        ),
    ];
    for (text, block, line) in cases {
        match preserve(text) {
            Err(Error::OverrideOverlap {
                block: b, line: l, ..
            }) => assert_eq!((b, l), (block, line), "{text}"),
            other => panic!("expected OverrideOverlap, got {other:?}"),
        }
    }
}
