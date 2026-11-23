//! `--preserve-behavior`: expansion steps E1 to E6 of docs/contract/profiles.md,
//! section 4, and the override glob grammar.
//!
//! `tests/fixtures/generate.py` writes the vectors from the contract reference,
//! `tests/contract/preserve_expansion.py`.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use serde_json::{Map, Value, json};
use slopvac_migrate::overlap::build_regions;
use slopvac_migrate::{Dropped, Error, Options, RenameTable, Rewrite, migrate_config};

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
            "https://slopvac.github.io/slopvac/schemas/v3/common.schema.json",
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

/// 2.x categories that no rule belongs to: 3.0 has none of them.
#[rstest]
#[case::meta_plain("meta", false)]
#[case::meta_preserve("meta", true)]
#[case::orwell_voice_plain("orwell-voice", false)]
#[case::orwell_voice_preserve("orwell-voice", true)]
#[case::prose_density_plain("prose-density", false)]
#[case::prose_density_preserve("prose-density", true)]
#[case::ste_vocabulary_plain("ste-vocabulary", false)]
#[case::ste_vocabulary_preserve("ste-vocabulary", true)]
fn a_rule_less_category_is_dropped_with_one_notice_per_use(
    #[case] name: &str,
    #[case] preserve_behavior: bool,
) {
    let text = format!(
        "profile = \"strict\"\n[categories]\n{name} = \"error\"\nprose-format = \"warning\"\n\
         [[overrides]]\nfiles = [\"docs/**\"]\n[overrides.categories]\n{name} = \"off\"\n"
    );
    let rewrite = migrate_config(
        &text,
        "slopvac.toml",
        &RenameTable::bundled(),
        Options { preserve_behavior },
    )
    .unwrap();
    assert!(
        !rewrite.text.contains(&format!("core:{name}")),
        "{}",
        rewrite.text
    );
    let notices: Vec<String> = rewrite
        .removed_categories
        .iter()
        .map(Dropped::category_removed_notice)
        .collect();
    assert_eq!(
        notices,
        [
            format!(
                "migrate-category-removed: slopvac.toml:3: `categories.{name}`: category no longer exists in 3.0; see removed-rules.toml at https://github.com/slopvac/slopvac/blob/main/docs/contract/compat/removed-rules.toml"
            ),
            format!(
                "migrate-category-removed: slopvac.toml:8: `overrides[0].categories.{name}`: category no longer exists in 3.0; see removed-rules.toml at https://github.com/slopvac/slopvac/blob/main/docs/contract/compat/removed-rules.toml"
            ),
        ]
    );
    let errors: Vec<String> = config_validator()
        .iter_errors(&as_json(&rewrite.text))
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}

/// The 2.x config and the key that the refusal names, from `refused-blocklist.json`.
fn refused_layout(name: &str) -> (String, String) {
    let text =
        fs::read_to_string(crate_dir().join("tests/fixtures/preserve/refused-blocklist.json"))
            .unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    let layout = &doc["layouts"][name];
    (
        layout["config"].as_str().unwrap().to_owned(),
        layout["at"].as_str().unwrap().to_owned(),
    )
}

#[rstest]
#[case::path_only_in_an_override("path-only-in-an-override")]
#[case::path_only_in_an_override_with_a_nested_switch(
    "path-only-in-an-override-with-a-nested-switch"
)]
#[case::switch_to_another_profile("switch-to-another-profile")]
#[case::moved_path_with_a_nested_switch("moved-path-with-a-nested-switch")]
#[case::switch_to_the_same_profile_normal("switch-to-the-same-profile-normal")]
#[case::switch_to_the_same_profile_strict("switch-to-the-same-profile-strict")]
#[case::top_profile_differs_from_the_switch("top-profile-differs-from-the-switch")]
fn a_blocklist_layout_the_companion_style_cannot_keep_is_refused(#[case] name: &str) {
    let (config, at) = refused_layout(name);
    let err = preserve(&config).unwrap_err();
    assert!(matches!(err, Error::BlocklistLayout { .. }), "{err}");
    let text = err.to_string();
    assert!(text.contains(&format!("`{at}`")), "{text}");
    assert!(text.contains("nothing is written"), "{text}");
}

#[test]
fn every_refusal_vector_has_a_case() {
    let text =
        fs::read_to_string(crate_dir().join("tests/fixtures/preserve/refused-blocklist.json"))
            .unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(doc["layouts"].as_object().unwrap().len(), 7);
}

const VOCABULARY_RULES: [&str; 4] = [
    "ste-verbs.verb-form-not-listed",
    "ste-words.verb-or-adjective-form-not-permitted",
    "ste-words.word-outside-controlled-vocabulary",
    "ste-words.word-used-in-wrong-part-of-speech",
];

fn rule_ids(layer: &toml::Table) -> BTreeSet<String> {
    layer
        .get("rules")
        .and_then(toml::Value::as_table)
        .map(|r| {
            r.keys()
                .map(|k| k.strip_prefix("core:").unwrap_or(k).to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn override_blocks(doc: &toml::Table) -> Vec<&toml::Table> {
    doc.get("overrides")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_table)
        .collect()
}

/// A vocabulary rule entry that the user did not author must not sit in a layer above the
/// styles: it would outrank the companion style and the settings 2.x let win. Each written
/// layer is compared only with the authored layer it came from: the top level with the top
/// level, and an override with the authored override that has the same `files`. A layer
/// with no authored counterpart (a guard, moved or switch block) gets no allowance.
/// Returns the number of layers walked, or the first offending entry.
fn check_no_generated_vocabulary_entry(text: &str, authored: &str) -> Result<usize, String> {
    let doc = toml::from_str::<toml::Table>(text).unwrap();
    let authored = toml::from_str::<toml::Table>(authored).unwrap_or_default();
    let mut authored_blocks = override_blocks(&authored);
    let mut layers: Vec<(&toml::Table, BTreeSet<String>)> = vec![(&doc, rule_ids(&authored))];
    for block in override_blocks(&doc) {
        let counterpart = authored_blocks
            .iter()
            .position(|a| a.get("files") == block.get("files"))
            .map(|at| authored_blocks.remove(at));
        layers.push((block, counterpart.map(rule_ids).unwrap_or_default()));
    }
    for (layer, allowed) in &layers {
        for id in rule_ids(layer) {
            if VOCABULARY_RULES.contains(&id.as_str()) && !allowed.contains(&id) {
                return Err(format!("generated vocabulary entry {id}\n{text}"));
            }
        }
    }
    Ok(layers.len())
}

#[rstest]
#[case::normal("schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n")]
#[case::strict("schema_version = 2\nprofile = \"strict\"\n[vocabulary]\npath = \"d.toml\"\n")]
#[case::relaxed("schema_version = 2\nprofile = \"relaxed\"\n[vocabulary]\npath = \"d.toml\"\n")]
#[case::authored_rule(
    "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n"
)]
#[case::authored_category(
    "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n[categories]\nste-words = \"off\"\n"
)]
#[case::top_rule_with_an_override_that_does_not_author_it(
    "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.categories]\nste-words = \"warning\"\n"
)]
#[case::category_override(
    "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n[categories]\nste-words = \"off\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.categories]\nste-words = \"error\"\n"
)]
#[case::override_changes_path(
    "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.vocabulary]\npath = \"o.toml\"\n"
)]
fn an_accepted_blocklist_layout_writes_no_vocabulary_rule_entry_of_its_own(#[case] config: &str) {
    let rw = preserve(config).unwrap();
    let walked = check_no_generated_vocabulary_entry(&rw.text, config)
        .unwrap_or_else(|why| panic!("written config: {why}"));
    assert!(walked >= 1, "the check walked no layer\n{}", rw.text);
}

#[test]
#[should_panic(
    expected = "generated vocabulary entry ste-words.word-outside-controlled-vocabulary"
)]
fn the_sweep_fails_when_an_override_carries_the_top_level_rule() {
    let authored = "schema_version = 2\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n";
    let broken = "schema_version = 2\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n";
    check_no_generated_vocabulary_entry(broken, authored)
        .unwrap_or_else(|why| panic!("written config: {why}"));
}

#[rstest]
// The companion follows the profile that the legacy style reproduces.
#[case::default_profile("", "normal")]
#[case::strict("profile = \"strict\"\n", "strict")]
#[case::relaxed("profile = \"relaxed\"\n", "relaxed")]
fn a_moved_top_level_blocklist_adds_the_vocabulary_companion_style(
    #[case] profile: &str,
    #[case] name: &str,
) {
    let rw = preserve(&format!(
        "schema_version = 2\n{profile}[vocabulary]\npath = \"deploy.toml\"\n"
    ))
    .unwrap();
    let doc = toml::from_str::<toml::Table>(&rw.text).unwrap();
    let styles: Vec<&str> = doc["styles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert_eq!(
        styles,
        [
            format!("core:legacy-{name}"),
            format!("core:legacy-{name}-vocabulary")
        ]
    );
    // The levels sit in the style, not in the user's own layers.
    assert!(!doc.contains_key("rules"), "{}", rw.text);
}

#[test]
fn a_config_without_a_blocklist_gets_no_vocabulary_companion_style() {
    let rw = preserve("schema_version = 2\nprofile = \"strict\"\n").unwrap();
    let doc = toml::from_str::<toml::Table>(&rw.text).unwrap();
    assert_eq!(doc["styles"].as_array().unwrap().len(), 1, "{}", rw.text);
}
