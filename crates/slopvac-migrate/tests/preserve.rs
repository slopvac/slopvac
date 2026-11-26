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

use std::collections::{BTreeMap, BTreeSet};
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

/// The scenarios that `--preserve-behavior` refuses, from `tests/fixtures/preserve/refused.json`.
fn refused() -> BTreeSet<String> {
    let text =
        fs::read_to_string(crate_dir().join("tests/fixtures/preserve/refused.json")).unwrap();
    let data: Value = serde_json::from_str(&text).unwrap();
    data["refused"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

fn e3_layouts() -> Map<String, Value> {
    let text =
        fs::read_to_string(crate_dir().join("tests/fixtures/preserve/e3-layout.json")).unwrap();
    let data: Value = serde_json::from_str(&text).unwrap();
    data["layouts"].as_object().unwrap().clone()
}

#[test]
fn expansion_matches_the_reference_for_each_scenario() {
    // STUB(slopvac-8l4.16.24): the reference still writes the old E3, so scenarios with
    // a profile override are checked by `e3_puts_each_block_where_the_rule_says`.
    let e3 = e3_layouts();
    let refused = refused();
    let cases = expansions();
    assert!(cases.len() >= 27);
    let wrong: Vec<String> = cases
        .iter()
        .filter(|(name, _, _)| !e3.contains_key(name) && !refused.contains(name))
        .filter(|(_, input, expected)| expansion_of(&preserve(input).unwrap().text) != *expected)
        .map(|(name, _, _)| name.clone())
        .collect();
    assert!(wrong.is_empty(), "differs from the reference: {wrong:?}");
}

#[test]
fn e3_puts_each_block_where_the_rule_says() {
    let e3 = e3_layouts();
    let with_profile: BTreeSet<String> = expansions()
        .into_iter()
        .filter(|(_, input, _)| {
            as_json(input)["overrides"]
                .as_array()
                .is_some_and(|b| b.iter().any(|b| b.get("profile").is_some()))
        })
        .map(|(name, _, _)| name)
        .collect();
    let refused = refused();
    let laid_out: BTreeSet<String> = e3.keys().cloned().collect();
    assert!(laid_out.is_disjoint(&refused), "{laid_out:?}");
    assert_eq!(
        laid_out.union(&refused).cloned().collect::<BTreeSet<_>>(),
        with_profile
    );
    for (name, input, _) in expansions() {
        if refused.contains(&name) {
            let code = preserve(&input).err().as_ref().map(Error::code);
            assert_eq!(code, Some("migrate-output-invalid"), "{name}");
            continue;
        }
        let Some(want) = e3.get(&name) else {
            continue;
        };
        let doc = as_json(&preserve(&input).unwrap().text);
        let got: Vec<Value> = doc["overrides"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| json!({ "files": b["files"], "profile": b.get("profile") }))
            .collect();
        assert_eq!(Value::Array(got), *want, "{name}");
    }
}

#[test]
fn inline_override_blocks_expand_like_array_tables() {
    let text = "profile = \"strict\"\nrules = { \"orwell.not-un\" = \"warning\" }\n\
                overrides = [\n  { files = [\"docs/**\"], profile = \"normal\" },\n  \
                { files = [\"docs/api/**\"], profile = \"relaxed\" },\n]\n";
    let (_, tables, _) = expansions()
        .into_iter()
        .find(|(name, _, _)| name == "top-rule-ignored-only-in-a-nested-scope")
        .unwrap();
    assert_eq!(
        expansion_of(&preserve(text).unwrap().text),
        expansion_of(&preserve(&tables).unwrap().text)
    );
}

#[test]
fn an_earlier_block_inside_a_profile_override_moves_after_it_with_its_comments() {
    let rw = preserve(
        "profile = \"strict\"\n\n# api docs\n[[overrides]]\nfiles = [\"docs/api/**\"]\n\
         locale = \"en-GB\"\n\n\
         [[overrides]]\nfiles = [\"docs/**\"]\nprofile = \"relaxed\"\nlocale = \"en-US\"\n",
    )
    .unwrap();
    let api = rw.text.find("# api docs").unwrap();
    let docs = rw.text.find("files = [\"docs/**\"]").unwrap();
    assert!(docs < api, "{}", rw.text);
    let doc = as_json(&rw.text);
    let blocks = doc["overrides"].as_array().unwrap();
    // 2.x applied the later block's locale to docs/api too, so the moved block drops its own.
    assert_eq!(blocks[1]["files"], json!(["docs/api/**"]));
    assert_eq!(blocks[1].get("locale"), None, "{}", rw.text);
    assert!(
        rw.notes
            .iter()
            .any(|n| n.starts_with("moved overrides[0] after overrides[1]")),
        "{:?}",
        rw.notes
    );
}

#[test]
fn a_block_moved_twice_still_gives_way_to_the_block_that_beat_it() {
    // 2.x applied all three blocks to docs/api/v1, in order, so the locale of the
    // second block won there.
    let rw = preserve(
        "profile = \"strict\"\n\n[[overrides]]\nfiles = [\"docs/api/v1/**\"]\n\
         locale = \"en-GB\"\n\n[[overrides]]\nfiles = [\"docs/api/**\"]\n\
         profile = \"normal\"\nlocale = \"en-US\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n\
         profile = \"relaxed\"\n",
    )
    .unwrap();
    let doc = as_json(&rw.text);
    let blocks = doc["overrides"].as_array().unwrap();
    let block = |files: &str| {
        blocks
            .iter()
            .find(|b| b["files"] == json!([files]))
            .unwrap_or_else(|| panic!("no block for {files}: {}", rw.text))
    };
    assert_eq!(block("docs/api/**").get("locale"), Some(&json!("en-US")));
    assert_eq!(block("docs/api/v1/**").get("locale"), None, "{}", rw.text);
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
    let refused = refused();
    for (name, input, _) in expansions() {
        if refused.contains(&name) {
            continue;
        }
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
#[case::later_category_over_an_earlier_rule("later-category-over-an-earlier-rule")]
#[case::later_category_with_no_severity_over_an_off_category(
    "later-category-with-no-severity-over-an-off-category"
)]
#[case::later_category_with_no_severity_over_an_earlier_rule(
    "later-category-with-no-severity-over-an-earlier-rule"
)]
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
    assert_eq!(doc["layouts"].as_object().unwrap().len(), 10);
}

const VOCABULARY_RULES: [&str; 4] = [
    "ste-verbs.verb-form-not-listed",
    "ste-words.verb-or-adjective-form-not-permitted",
    "ste-words.word-outside-controlled-vocabulary",
    "ste-words.word-used-in-wrong-part-of-speech",
];

/// The rule levels of one layer, keyed without the `core:` prefix. Each entry is checked
/// before it is normalized: it needs a string level (a bare string or a `severity` string),
/// and the bare and `core:` spellings of one rule must not disagree.
fn rule_levels(layer: &toml::Table) -> Result<BTreeMap<String, String>, String> {
    let mut levels = BTreeMap::new();
    let Some(rules) = layer.get("rules").and_then(toml::Value::as_table) else {
        return Ok(levels);
    };
    for (key, value) in rules {
        let level = value
            .as_str()
            .or_else(|| value.get("severity").and_then(toml::Value::as_str))
            .ok_or_else(|| format!("rule entry {key} has no string level: {value}"))?;
        let id = key.strip_prefix("core:").unwrap_or(key);
        match levels.insert(id.to_owned(), level.to_owned()) {
            Some(earlier) if earlier != level => {
                return Err(format!(
                    "rule entry {id} conflicts: {earlier} and {level} under its bare and core: spellings"
                ));
            }
            _ => {}
        }
    }
    Ok(levels)
}

fn override_blocks(doc: &toml::Table) -> Vec<&toml::Table> {
    doc.get("overrides")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_table)
        .collect()
}

/// What the sweep walked and found: the number of layers and the E4 `off` guards, each as the
/// `files` of its override block and the rule it turns off.
#[derive(Debug)]
struct Walk {
    layers: usize,
    guards: BTreeSet<(Vec<String>, String)>,
}

/// The vocabulary rule entries of a config must match the authored input in both directions.
///
/// Every written entry is allowed in two forms only:
///
/// - at its authored level in the matching layer: the top level with the top level, an
///   override with the authored override that has the same `files`;
/// - as an E4 `off` guard: an `off` entry in an override block that does not author that
///   rule, for a rule that the input names somewhere. E4 writes guards only into override
///   blocks, either a block of its own or an authored block it folded the guard into, so the
///   top level never holds one and an authored layer that names the rule keeps its level.
///
/// Anything else would outrank the companion style and the settings 2.x let win. In the other
/// direction, every vocabulary rule the input authors in a layer must still be there at its
/// authored level, so a dropped or changed entry fails too. Every authored override block must
/// match a written one by its `files`, with or without a vocabulary rule. Extra override
/// blocks that hold only guards are allowed. Where the authored layer sets a level for the
/// `ste-words` category, the matched written layer must set the same one. Returns what it walked, or the first offence.
fn check_vocabulary_entries(text: &str, authored: &str) -> Result<Walk, String> {
    let doc = toml::from_str::<toml::Table>(text)
        .map_err(|e| format!("the written config does not parse: {e}\n{text}"))?;
    let authored = toml::from_str::<toml::Table>(authored).unwrap_or_default();
    let mut authored_blocks = override_blocks(&authored);
    let authored_top = vocabulary_levels(&authored)?;
    let mut named: BTreeSet<String> = authored_top.keys().cloned().collect();
    for block in &authored_blocks {
        named.extend(vocabulary_levels(block)?.into_keys());
    }
    let mut guards = BTreeSet::new();
    // Each layer, with the authored levels of its counterpart and whether it can hold a guard.
    let mut layers: Vec<(&toml::Table, BTreeMap<String, String>, bool, Option<String>)> = vec![(
        &doc,
        authored_top,
        false,
        vocabulary_category_level(&authored),
    )];
    for block in override_blocks(&doc) {
        let counterpart = authored_blocks
            .iter()
            .position(|a| a.get("files") == block.get("files"))
            .map(|at| authored_blocks.remove(at));
        let own = match counterpart {
            Some(c) => vocabulary_levels(c)?,
            None => BTreeMap::new(),
        };
        let category = counterpart.and_then(vocabulary_category_level);
        layers.push((block, own, true, category));
    }
    for (layer, own, may_guard, category) in &layers {
        let written = vocabulary_levels(layer)?;
        for (id, level) in &written {
            let authored_here = own.get(id) == Some(level);
            let guard = *may_guard && level == "off" && named.contains(id) && !own.contains_key(id);
            if !authored_here && !guard {
                return Err(format!("generated vocabulary entry {id} = {level}\n{text}"));
            }
            if guard {
                guards.insert((files_of(layer), id.clone()));
            }
        }
        if let Some(level) = category
            && vocabulary_category_level(layer).as_ref() != Some(level)
        {
            let at = if *may_guard {
                format!("override {:?}", files_of(layer))
            } else {
                "the top level".to_owned()
            };
            return Err(format!(
                "authored category ste-words = {level} is missing from its layer ({at})\n{text}"
            ));
        }
        for (id, level) in own {
            if written.get(id) != Some(level) {
                return Err(format!(
                    "authored vocabulary rule {id} = {level} is missing from its layer\n{text}"
                ));
            }
        }
    }
    // Every authored override must still be there, even one that holds no vocabulary rule: a
    // category-only override changes the levels of the files it matches.
    for block in &authored_blocks {
        if let Some((id, level)) = vocabulary_levels(block)?.into_iter().next() {
            return Err(format!(
                "authored vocabulary rule {id} = {level} is missing: its override block is gone\n{text}"
            ));
        }
    }
    if let Some(block) = authored_blocks.first() {
        let files = block
            .get("files")
            .map_or_else(|| "(no files)".to_owned(), ToString::to_string);
        return Err(format!(
            "authored override {files} is missing from the written config\n{text}"
        ));
    }
    // The layer checks above name the exact entry that went missing. This one catches the rest:
    // an input that authors a blocklist or a category, where the output keeps none of it.
    if has_vocabulary_content(&authored) && !has_vocabulary_content(&doc) {
        return Err(format!(
            "the written config has no vocabulary content, but the authored input has some\n{text}"
        ));
    }
    Ok(Walk {
        layers: layers.len(),
        guards,
    })
}

/// The level of the `ste-words` category in one layer, in its bare or `core:` spelling, as a
/// bare string or a `severity`. `None` when the layer sets no level for it.
fn vocabulary_category_level(layer: &toml::Table) -> Option<String> {
    let categories = layer.get("categories")?.as_table()?;
    ["ste-words", "core:ste-words"].iter().find_map(|key| {
        let entry = categories.get(*key)?;
        entry
            .as_str()
            .or_else(|| entry.get("severity")?.as_str())
            .map(str::to_owned)
    })
}

/// The `files` patterns of an override block.
fn files_of(block: &toml::Table) -> Vec<String> {
    block
        .get("files")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|f| f.as_str().map(str::to_owned))
        .collect()
}

/// Whether a config, or any of its override blocks, carries anything that a word blocklist
/// leaves behind: a `[vocabulary]` table (2.x), a blocklist in `terms` or a companion style
/// (3.0), a vocabulary rule entry, or a category entry for `ste-words`.
fn has_vocabulary_content(doc: &toml::Table) -> bool {
    let in_layer = |layer: &toml::Table| -> bool {
        let companion = layer
            .get("styles")
            .and_then(toml::Value::as_array)
            .is_some_and(|styles| {
                styles
                    .iter()
                    .any(|s| s.as_str().is_some_and(|s| s.ends_with("-vocabulary")))
            });
        let blocklist = layer
            .get("terms")
            .and_then(|t| t.get("blocklists"))
            .and_then(toml::Value::as_array)
            .is_some_and(|b| !b.is_empty());
        let category = layer
            .get("categories")
            .and_then(toml::Value::as_table)
            .is_some_and(|c| {
                c.keys()
                    .any(|k| k.strip_prefix("core:").unwrap_or(k) == "ste-words")
            });
        let rule = rule_levels(layer)
            .is_ok_and(|l| l.keys().any(|id| VOCABULARY_RULES.contains(&id.as_str())));
        layer.contains_key("vocabulary") || companion || blocklist || category || rule
    };
    in_layer(doc) || override_blocks(doc).into_iter().any(in_layer)
}

/// The vocabulary rules among the rule levels of one layer.
fn vocabulary_levels(layer: &toml::Table) -> Result<BTreeMap<String, String>, String> {
    let mut levels = rule_levels(layer)?;
    levels.retain(|id, _| VOCABULARY_RULES.contains(&id.as_str()));
    Ok(levels)
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
#[case::category_override(
    "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n[categories]\nste-words = \"off\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.categories]\nste-words = \"error\"\n"
)]
#[case::authored_category_level(
    "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n[categories]\nste-words = \"warning\"\n"
)]
#[case::e4_guard_for_an_authored_rule(
    "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.categories]\nste-words = \"off\"\n"
)]
#[case::override_changes_path(
    "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.vocabulary]\npath = \"o.toml\"\n"
)]
fn an_accepted_blocklist_layout_writes_no_vocabulary_rule_entry_of_its_own(#[case] config: &str) {
    let rw = preserve(config).unwrap();
    let walk = check_vocabulary_entries(&rw.text, config)
        .unwrap_or_else(|why| panic!("written config: {why}"));
    assert!(walk.layers >= 1, "the check walked no layer\n{}", rw.text);
}

#[rstest]
#[case::an_override_carries_the_top_level_rule(
    "schema_version = 2\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n",
    "schema_version = 2\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n",
    "generated vocabulary entry ste-words.word-outside-controlled-vocabulary"
)]
#[case::a_layer_changes_the_authored_level(
    "schema_version = 2\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n",
    "schema_version = 2\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"suggestion\"\n",
    "generated vocabulary entry ste-words.word-outside-controlled-vocabulary"
)]
#[case::a_guard_turns_off_a_rule_the_input_does_not_author(
    "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.categories]\nste-words = \"off\"\n",
    "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.categories]\nste-words = \"off\"\n[overrides.rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"off\"\n",
    "generated vocabulary entry ste-words.word-outside-controlled-vocabulary"
)]
#[case::an_off_replaces_the_authored_level_of_an_override(
    "schema_version = 2\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n",
    "schema_version = 2\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"off\"\n",
    "generated vocabulary entry ste-words.word-outside-controlled-vocabulary"
)]
#[case::the_top_level_carries_an_off_guard(
    "schema_version = 2\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n",
    "schema_version = 2\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"off\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n",
    "generated vocabulary entry ste-words.word-outside-controlled-vocabulary"
)]
#[case::an_authored_top_level_rule_is_dropped(
    "schema_version = 2\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n",
    "schema_version = 2\n",
    "authored vocabulary rule ste-words.word-outside-controlled-vocabulary"
)]
#[case::an_authored_rule_is_dropped_from_its_override(
    "schema_version = 2\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n",
    "schema_version = 2\n\n[[overrides]]\nfiles = [\"docs/**\"]\n",
    "authored vocabulary rule ste-words.word-outside-controlled-vocabulary"
)]
#[case::an_authored_override_is_dropped_with_its_rule(
    "schema_version = 2\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n",
    "schema_version = 2\n",
    "authored vocabulary rule ste-words.word-outside-controlled-vocabulary"
)]
#[case::the_output_is_empty(
    "schema_version = 2\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n",
    "",
    "authored vocabulary rule ste-words.word-outside-controlled-vocabulary"
)]
#[case::an_empty_output_for_an_authored_blocklist(
    "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n",
    "",
    "no vocabulary content"
)]
#[case::a_category_only_override_is_dropped(
    "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n[categories]\nste-words = \"off\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.categories]\nste-words = \"error\"\n",
    "schema_version = 2\nstyles = [\"core:legacy-normal\", \"core:legacy-normal-vocabulary\"]\n[categories]\nste-words = \"off\"\n",
    "authored override [\"docs/**\"] is missing"
)]
#[case::an_override_keeps_its_files_but_drops_its_category(
    "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n[categories]\nste-words = \"off\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.categories]\nste-words = \"error\"\n",
    "schema_version = 2\nstyles = [\"core:legacy-normal\", \"core:legacy-normal-vocabulary\"]\n[categories]\nste-words = \"off\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n",
    "authored category ste-words = error is missing from its layer"
)]
#[case::an_override_keeps_its_files_but_changes_its_category_level(
    "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n[categories]\nste-words = \"off\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.categories]\nste-words = \"error\"\n",
    "schema_version = 2\nstyles = [\"core:legacy-normal\", \"core:legacy-normal-vocabulary\"]\n[categories]\nste-words = \"off\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.categories]\nste-words = \"warning\"\n",
    "authored category ste-words = error is missing from its layer"
)]
#[case::an_entry_has_no_string_level(
    "schema_version = 2\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n",
    "schema_version = 2\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = { other = 1 }\n",
    "has no string level"
)]
#[case::the_bare_and_core_spellings_conflict(
    "schema_version = 2\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n",
    "schema_version = 2\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n\"core:ste-words.word-outside-controlled-vocabulary\" = \"suggestion\"\n",
    "conflicts"
)]
fn the_sweep_rejects_a_written_config_that_departs_from_the_authored_input(
    #[case] authored: &str,
    #[case] written: &str,
    #[case] reason: &str,
) {
    let outcome = check_vocabulary_entries(written, authored);
    assert!(outcome.is_err(), "the sweep accepted\n{written}");
    let err = outcome.unwrap_err();
    assert!(err.contains(reason), "{err}");
}

/// The E4 input: a top-level rule, and an authored override that turns the whole category off.
/// E4 keeps the rule for the files outside `docs/**` and guards it inside.
const E4_GUARDED: &str = "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.categories]\nste-words = \"off\"\n";

/// The written config must hold an `off` guard for `rule` in the override block with `files`.
fn require_guard(text: &str, authored: &str, files: &[&str], rule: &str) -> Result<(), String> {
    let walk = check_vocabulary_entries(text, authored)?;
    let at = (
        files.iter().map(|f| (*f).to_owned()).collect(),
        rule.to_owned(),
    );
    if walk.guards.contains(&at) {
        Ok(())
    } else {
        Err(format!(
            "the override {files:?} has no off guard for {rule}\n{text}"
        ))
    }
}

#[test]
fn an_e4_guard_sits_at_the_override_it_guards() {
    let rw = preserve(E4_GUARDED).unwrap();
    require_guard(&rw.text, E4_GUARDED, &["docs/**"], VOCABULARY_RULE)
        .unwrap_or_else(|why| panic!("written config: {why}"));
}

#[rstest]
#[case::no_guard_is_written(
    "schema_version = 2\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.categories]\nste-words = \"off\"\n"
)]
#[case::the_guard_is_in_another_override(
    "schema_version = 2\n[rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"error\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.categories]\nste-words = \"off\"\n\n[[overrides]]\nfiles = [\"src/**\"]\n[overrides.rules]\n\"ste-words.word-outside-controlled-vocabulary\" = \"off\"\n"
)]
fn a_written_config_without_the_e4_guard_at_its_override_fails(#[case] written: &str) {
    let outcome = require_guard(written, E4_GUARDED, &["docs/**"], VOCABULARY_RULE);
    assert!(outcome.is_err(), "the guard check accepted\n{written}");
    let err = outcome.unwrap_err();
    assert!(err.contains("has no off guard"), "{err}");
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

const VOCABULARY_RULE: &str = "ste-words.word-outside-controlled-vocabulary";

#[rstest]
#[case::top_level(
    "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n[rules.\"ste-words.word-outside-controlled-vocabulary\"]\nseverity = \"error\"\n",
    "/rules/core:ste-words.word-outside-controlled-vocabulary"
)]
#[case::in_an_override(
    "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.rules.\"ste-words.word-outside-controlled-vocabulary\"]\nseverity = \"error\"\n",
    "/overrides/0/rules/core:ste-words.word-outside-controlled-vocabulary"
)]
fn an_authored_severity_table_rule_is_written_as_a_bare_level(
    #[case] config: &str,
    #[case] at: &str,
) {
    let rw = preserve(config).unwrap();
    assert_eq!(
        as_json(&rw.text).pointer(at),
        Some(&json!("error")),
        "{}",
        rw.text
    );
}

#[test]
fn an_override_keeps_its_own_blocklist_beside_an_authored_off_rule() {
    // The override both turns the rule off and names its own blocklist. A block that kept
    // only the `off` entry would look the same as a guard with that selector, so the
    // blocklist is what shows the authored override survived.
    let rw = preserve(&format!(
        "schema_version = 2\n[vocabulary]\npath = \"d.toml\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.rules]\n\"{VOCABULARY_RULE}\" = \"off\"\n[overrides.vocabulary]\npath = \"o.toml\"\n"
    ))
    .unwrap();
    let doc = as_json(&rw.text);
    assert_eq!(
        doc.pointer("/overrides/0/terms/blocklists"),
        Some(&json!(["o.toml"])),
        "{}",
        rw.text
    );
    assert_eq!(
        doc.pointer(&format!("/overrides/0/rules/core:{VOCABULARY_RULE}")),
        Some(&json!("off")),
        "{}",
        rw.text
    );
}
