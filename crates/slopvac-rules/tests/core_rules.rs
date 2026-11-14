//! The shipped core rules, converted from the 2.x files (`fixtures/core-v3`, written by
//! `fixtures/generate.py`), load, compile and match like the Python reference.

#![allow(
    missing_docs,
    clippy::single_range_in_vec_init,
    clippy::cast_possible_truncation
)]

mod common;

use std::path::PathBuf;

use common::{generated, manifest, published};
use serde_json::Value;
use slopvac_model::{PackId, RuleId};
use slopvac_rules::{CompileOptions, MatchKind, RuleSet, Scope, category_schema};

fn core_dir() -> PathBuf {
    manifest().join("tests/fixtures/core-v3")
}

fn core() -> RuleSet {
    RuleSet::load_dir(
        &PackId::new("core").unwrap(),
        &core_dir(),
        &CompileOptions::default(),
    )
    .unwrap_or_else(|e| panic!("core rules must load:\n{e}"))
}

fn no_spans() -> Vec<std::ops::Range<usize>> {
    Vec::new()
}

fn id(text: &str) -> RuleId {
    RuleId::new(text).unwrap()
}

#[test]
fn every_core_category_file_validates_against_both_schemas() {
    let published = published("category.schema.json", None);
    let generated = generated(&category_schema());
    let mut n = 0;
    for path in common::instances(&core_dir()) {
        let value = common::load(&path);
        assert!(published.is_valid(&value), "published: {}", path.display());
        assert!(generated.is_valid(&value), "generated: {}", path.display());
        n += 1;
    }
    assert_eq!(n, 26);
}

#[test]
fn the_core_pack_loads_every_category_and_rule() {
    let set = core();
    assert_eq!(set.categories().len(), 26);
    assert_eq!(set.len(), 235);
    assert_eq!(set.rules().count(), 235);
}

#[test]
fn rules_are_counted_by_kind() {
    let set = core();
    let count = |k| set.rules_of_kind(k).count();
    // 109 + 18 + 17 lexical rules; the rest carry no pattern.
    assert_eq!(count(MatchKind::Pattern), 109);
    assert_eq!(count(MatchKind::Substitution), 18);
    assert_eq!(count(MatchKind::Tokens), 17);
    assert_eq!(count(MatchKind::Judgement), 68);
    assert_eq!(count(MatchKind::Metric), 13);
    assert_eq!(count(MatchKind::Structure), 6);
    assert_eq!(count(MatchKind::Vocabulary), 4);
    assert_eq!(count(MatchKind::Capitalization), 0);
}

#[test]
fn only_lexical_rules_carry_a_matcher() {
    let set = core();
    for rule in set.rules() {
        let lexical = matches!(
            rule.kind(),
            MatchKind::Tokens | MatchKind::Pattern | MatchKind::Substitution
        );
        assert_eq!(rule.matcher().is_some(), lexical, "{}", rule.id());
    }
}

#[test]
fn a_rule_is_found_by_its_qualified_id() {
    let set = core();
    let rule = set.get(&id("core:ai-residue.chat-leakage")).unwrap();
    assert_eq!(rule.category().as_str(), "ai-residue");
    assert_eq!(rule.rule().id.as_str(), "chat-leakage");
    assert!(rule.file().ends_with("ai-residue.json"));
    assert!(rule.line().is_some_and(|l| l > 1));
    assert!(set.get(&id("core:ai-residue.nope")).is_none());
    assert!(set.get(&id("other:ai-residue.chat-leakage")).is_none());
}

#[test]
fn rules_are_filtered_by_scope() {
    let set = core();
    let headings = set.rules_in_scope(Scope::Heading).collect::<Vec<_>>();
    assert!(
        headings
            .iter()
            .any(|r| r.id().local() == "ai-tells-formatting.title-case-heading")
    );
    assert!(headings.iter().all(|r| r.rule().scope == Scope::Heading));
}

#[test]
fn a_loaded_rule_reports_its_match_spans() {
    let set = core();
    let rule = set.get(&id("core:ai-residue.chat-leakage")).unwrap();
    // "As an AI" starts at byte 0; the leading text is two bytes of "é".
    assert_eq!(rule.find_all("As an AI, hello.").unwrap(), [0..8]);
    assert_eq!(rule.find_all("é As an AI").unwrap(), [3..11]);
    assert_eq!(rule.find_all("Nothing here.").unwrap(), no_spans());
}

#[test]
fn a_rule_without_a_pattern_has_no_spans() {
    let set = core();
    let judgement = set.rules_of_kind(MatchKind::Judgement).next().unwrap();
    assert_eq!(judgement.find_all("anything").unwrap(), no_spans());
}

/// Python reports code point offsets; the Rust matcher reports bytes.
fn byte_offsets(text: &str) -> Vec<usize> {
    let mut at: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    at.push(text.len());
    at
}

fn parity() -> Value {
    let text =
        std::fs::read_to_string(manifest().join("tests/fixtures/pattern-parity.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// Python's caseless matching folds U+0130 (İ) and U+0131 (ı) onto `i` and `I`; PCRE2 does
/// not. `fixtures/divergences.md` records it. Every other difference is a bug.
fn is_known_divergence(text: &str) -> bool {
    text.contains(['\u{130}', '\u{131}'])
}

/// Rule, text index, Python's spans and the Rust spans, for a pair that differs.
type Difference = (String, usize, Vec<(usize, usize)>, Vec<(usize, usize)>);

#[test]
fn every_lexical_pattern_matches_python_on_the_corpus() {
    let set = core();
    let doc = parity();
    let texts: Vec<&str> = doc["texts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    let offsets: Vec<Vec<usize>> = texts.iter().map(|t| byte_offsets(t)).collect();
    let expected = doc["spans"].as_object().unwrap();
    let mut rules = 0;
    let mut compared = 0;
    let mut differing: Vec<Difference> = Vec::new();
    for rule in set.rules().filter(|r| r.matcher().is_some()) {
        let python_id = rule.id().local().to_owned();
        let hits = expected
            .get(&python_id)
            .unwrap_or_else(|| panic!("no Python spans for {python_id}"))
            .as_object()
            .unwrap();
        rules += 1;
        for (index, text) in texts.iter().enumerate() {
            let at = &offsets[index];
            let want: Vec<(usize, usize)> = hits
                .get(&index.to_string())
                .map(|v| {
                    v.as_array()
                        .unwrap()
                        .iter()
                        .map(|p| {
                            (
                                at[p[0].as_u64().unwrap() as usize],
                                at[p[1].as_u64().unwrap() as usize],
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let got: Vec<(usize, usize)> = rule
                .find_all(text)
                .unwrap()
                .into_iter()
                .map(|r| (r.start, r.end))
                .collect();
            compared += 1;
            if got != want {
                differing.push((python_id.clone(), index, want, got));
            }
        }
    }
    let python_matches: usize = expected
        .values()
        .flat_map(|hits| hits.as_object().unwrap().values())
        .map(|spans| spans.as_array().unwrap().len())
        .sum();
    assert!(
        python_matches > 500,
        "the corpus exercises the patterns: {python_matches}"
    );
    assert!(
        expected
            .values()
            .all(|hits| !hits.as_object().unwrap().is_empty()),
        "every rule must match at least one corpus text"
    );
    assert_eq!(rules, 144);
    assert_eq!(compared, 144 * texts.len());
    let unexpected: Vec<_> = differing
        .iter()
        .filter(|(_, i, _, _)| !is_known_divergence(texts[*i]))
        .collect();
    assert!(
        unexpected.is_empty(),
        "{} differing (rule, text) pairs, first: {:#?}",
        unexpected.len(),
        unexpected.iter().take(5).collect::<Vec<_>>()
    );
    // The recorded divergence is real: if it disappears, delete it here and from the docs.
    assert!(differing.len() > 100);
}
