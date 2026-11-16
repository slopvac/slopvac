//! Parity of the generated spelling rule with 2.x on the interim locale fixture.
//!
//! STUB(slopvac-8l4.3.29): `fixtures/locale-parity.json` holds what 2.x builds and reports
//! (`fixtures/generate_locale.py`). The golden suite replaces it.
//! `fixtures/locale-divergences.json` lists the documents whose positions differ on purpose,
//! with the reason; every other fact is still compared, and a listed difference must still
//! exist.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
use slopvac_engine::locale::{self, Locale, LocaleError};
use slopvac_engine::{Engine, Input, Level, Settings};
use slopvac_model::{GenreId, GenreResult, GenreSource, NonEmptyString, PackId};
use slopvac_rules::{CompileOptions, RuleSet};
use slopvac_terms::{TermEntry, TermSet};

fn manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn read(name: &str) -> Value {
    let path = manifest().join("tests/fixtures").join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn pack() -> PackId {
    PackId::new("core").unwrap()
}

/// The core `ste-words` category: the base the generated rule joins.
#[expect(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn base() -> RuleSet {
    let path = manifest().join("../slopvac-rules/tests/fixtures/core-v3/ste-words.json");
    let text = std::fs::read_to_string(&path).unwrap();
    RuleSet::from_sources(&pack(), [(path, text)], &CompileOptions::default())
        .unwrap_or_else(|e| panic!("{e}"))
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn pairs(value: &Value) -> Vec<(String, String)> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p[0].as_str().unwrap().to_owned(),
                p[1].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn locale_of(tag: &str) -> Locale {
    Locale::parse(tag).unwrap()
}

/// A `ste-words` category with a non-default weight and budget and one rule `id` that
/// replaces `flavor` by `flavour`.
#[expect(
    clippy::panic,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn custom_base(id: &str) -> RuleSet {
    let doc = serde_json::json!({
        "schema_version": 3,
        "id": "ste-words",
        "title": "Custom title",
        "description": "Custom description.",
        "weight": 2.5,
        "max_per_100_words": {"normal": 0.7},
        "recommended_for": ["reference"],
        "rules": [{
            "id": id,
            "name": "Other",
            "message": "Other rule {match}.",
            "severity": "error",
            "match": {"kind": "substitution", "substitutions": {"flavor": "flavour"}},
            "citation": {"source": "test"}
        }]
    });
    RuleSet::from_sources(
        &pack(),
        [(PathBuf::from("custom.json"), doc.to_string())],
        &CompileOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"))
}

/// An engine with the base rules off, so only the generated rule reports.
fn engine(tag: &str, allow: &[String]) -> Engine {
    engine_over(base(), tag, allow, true)
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn engine_over(base: RuleSet, tag: &str, allow: &[String], base_off: bool) -> Engine {
    let generated = locale::spelling_rules(&pack(), &base, tag, allow, &CompileOptions::default())
        .unwrap()
        .unwrap();
    let mut settings = Settings::new(
        NonEmptyString::new("normal").unwrap(),
        GenreResult {
            id: GenreId::new("reference").unwrap(),
            source: GenreSource::Default,
            reasons: Vec::new(),
        },
    );
    if base_off {
        for rule in base.rules() {
            settings.rule_levels.insert(rule.id().clone(), Level::Off);
        }
    }
    Engine::new(vec![base, generated], settings).unwrap()
}

#[test]
fn the_variant_table_resolves_as_2x_does() {
    let fixture = read("locale-parity.json");
    for tag in ["en-US", "en-GB"] {
        let want = &fixture["locales"][tag];
        let got = locale::spelling(locale_of(tag), &[]).unwrap();
        assert_eq!(got.substitutions, pairs(&want["substitutions"]), "{tag}");
        assert_eq!(got.allowlist, strings(&want["allowlist"]), "{tag}");
    }
    assert!(fixture["locales"]["und"].is_null());
    assert!(locale::spelling(Locale::Und, &[]).is_none());
}

#[test]
fn allowed_words_resolve_as_2x_does() {
    let fixture = read("locale-parity.json");
    let allow = strings(&fixture["with_allow"]["allow"]);
    for tag in ["en-US", "en-GB"] {
        let want = &fixture["with_allow"][tag];
        let got = locale::spelling(locale_of(tag), &allow).unwrap();
        assert_eq!(got.substitutions, pairs(&want["substitutions"]), "{tag}");
        assert_eq!(got.allowlist, strings(&want["allowlist"]), "{tag}");
    }
}

#[test]
fn the_generated_rule_carries_the_2x_name_message_and_severity() {
    let fixture = read("locale-parity.json");
    let base = base();
    for tag in ["en-US", "en-GB"] {
        let want = &fixture["locales"][tag];
        let set = locale::spelling_rules(&pack(), &base, tag, &[], &CompileOptions::default())
            .unwrap()
            .unwrap();
        let loaded = set.rules().next().unwrap();
        assert_eq!(loaded.id().as_str(), "core:ste-words.spelling");
        assert_eq!(loaded.rule().name.as_str(), want["name"].as_str().unwrap());
        assert_eq!(
            loaded.rule().message.as_str(),
            want["message"].as_str().unwrap()
        );
        assert_eq!(
            format!("{:?}", loaded.rule().severity).to_lowercase(),
            want["severity"].as_str().unwrap()
        );
    }
}

#[test]
fn und_builds_no_rule_and_an_unknown_tag_says_so_as_2x_does() {
    let fixture = read("locale-parity.json");
    let base = base();
    let options = CompileOptions::default();
    assert!(
        locale::spelling_rules(&pack(), &base, "und", &[], &options)
            .unwrap()
            .is_none()
    );
    let err = locale::spelling_rules(&pack(), &base, "fr-FR", &[], &options).unwrap_err();
    assert!(matches!(err, LocaleError::Unknown(_)));
    assert_eq!(
        err.to_string(),
        fixture["unknown_locale_note"].as_str().unwrap()
    );
}

type Row = (String, usize, usize, usize, String, String, String);

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn rows_expected(findings: &Value) -> Vec<Row> {
    findings
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["rule"].as_str().unwrap().to_owned(),
                usize::try_from(f["line"].as_u64().unwrap()).unwrap(),
                usize::try_from(f["column"].as_u64().unwrap()).unwrap(),
                usize::try_from(f["end_column"].as_u64().unwrap()).unwrap(),
                f["severity"].as_str().unwrap().to_owned(),
                f["matched"].as_str().unwrap().to_owned(),
                f["message"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn rows_actual(engine: &Engine, name: &str, text: &str) -> Vec<Row> {
    let doc = engine.lint(&Input::markdown(name, text)).unwrap();
    doc.findings
        .iter()
        .map(|f| {
            let end = f.extent.map_or(f.start, |e| e.end);
            (
                f.rule_id.to_string(),
                f.start.line(),
                f.start.column(),
                end.column(),
                format!("{:?}", f.severity).to_lowercase(),
                f.matched_text.clone(),
                f.message.to_string(),
            )
        })
        .collect()
}

/// Rows with the position dropped, in a fixed order.
fn without_position(mut rows: Vec<Row>) -> Vec<Row> {
    for row in &mut rows {
        row.1 = 0;
        row.2 = 0;
        row.3 = 0;
    }
    rows.sort();
    rows
}

#[test]
fn locale_findings_match_the_2x_engine_on_the_interim_fixture() {
    let fixture = read("locale-parity.json");
    let divergences: BTreeMap<String, Value> =
        serde_json::from_value(read("locale-divergences.json")).unwrap();
    let mut problems = Vec::new();
    let mut compared = 0;
    for doc in fixture["documents"].as_array().unwrap() {
        let name = doc["name"].as_str().unwrap();
        let text = doc["text"].as_str().unwrap();
        let tag = doc["locale"].as_str().unwrap();
        let want = rows_expected(&doc["findings"]);
        let got = if tag == "und" {
            // `und` builds no rule, so no engine runs and nothing is reported.
            Vec::new()
        } else {
            rows_actual(&engine(tag, &strings(&doc["allow"])), name, text)
        };
        compared += want.len();
        let agrees = want == got;
        if divergences.contains_key(name) {
            if agrees {
                problems.push(format!(
                    "{name}: agrees now; remove it from locale-divergences.json"
                ));
            } else if without_position(want) != without_position(got) {
                problems.push(format!("{name}: differs beyond position"));
            }
        } else if !agrees {
            problems.push(format!("{name}: want {want:?}, got {got:?}"));
        }
    }
    assert!(compared > 30, "only {compared} findings were compared");
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

fn term(spelling: &str) -> TermEntry {
    TermEntry {
        term: spelling.to_owned(),
        variants: Vec::new(),
        kind: Some("product".to_owned()),
        ignore_case: false,
        case_sensitive: false,
    }
}

#[test]
fn a_term_spelling_is_exempt_from_the_spelling_check() {
    // A product called "Colour" in an en-US project: the term is not a misspelling.
    let allow = locale::term_words(&TermSet::new([term("Colour")]));
    assert_eq!(allow, ["Colour"]);
    let text = "Open Colour and the colour wheel.\n";
    let matched = |rows: Vec<Row>| rows.into_iter().map(|r| r.5).collect::<Vec<_>>();
    let plain = matched(rows_actual(&engine("en-US", &[]), "a.md", text));
    // The allowlist is exact and case-insensitive, so the term hides `colour` too.
    let exempt = matched(rows_actual(&engine("en-US", &allow), "a.md", text));
    assert_eq!(plain, ["Colour", "colour"]);
    assert_eq!(exempt, Vec::<String>::new());
}

#[test]
fn the_builtin_terms_hide_no_variant() {
    // The shipped terms are product and language names; none is a spelling variant, so
    // exempting them changes no substitution.
    let allow = locale::term_words(&TermSet::builtin());
    assert_ne!(allow, Vec::<String>::new());
    for tag in [Locale::EnUs, Locale::EnGb] {
        let plain = locale::spelling(tag, &[]).unwrap();
        let with_terms = locale::spelling(tag, &allow).unwrap();
        assert_eq!(plain.substitutions, with_terms.substitutions);
    }
}

#[test]
fn a_term_variant_is_exempt_from_the_spelling_check() {
    // The variant `colour` of the term `Chroma` is an accepted spelling in an en-US project.
    let mut chroma = term("Chroma");
    chroma.variants = vec!["colour".to_owned()];
    let allow = locale::term_words(&TermSet::new([chroma]));
    let matched = |rows: Vec<Row>| rows.into_iter().map(|r| r.5).collect::<Vec<_>>();
    let text = "Use the colour here.\n";
    assert_eq!(
        matched(rows_actual(&engine("en-US", &[]), "a.md", text)),
        ["colour"]
    );
    assert_eq!(
        matched(rows_actual(&engine("en-US", &allow), "a.md", text)),
        Vec::<String>::new()
    );
}

#[test]
fn the_locale_set_keeps_the_scoring_metadata_of_the_category() {
    let set = locale::spelling_rules(
        &pack(),
        &custom_base("other"),
        "en-GB",
        &[],
        &CompileOptions::default(),
    )
    .unwrap()
    .unwrap();
    let category = &set.categories()[0];
    assert_eq!(category.id().as_str(), "ste-words");
    assert_eq!(category.title().as_str(), "Custom title");
    assert_eq!(category.description().as_str(), "Custom description.");
    assert!((category.weight().get() - 2.5).abs() < f64::EPSILON);
    let budget = category.max_per_100_words().unwrap();
    assert_eq!(budget.len(), 1);
    assert!((budget.values().next().unwrap().get() - 0.7).abs() < f64::EPSILON);
    assert_eq!(category.recommended_for().len(), 1);
}

#[test]
fn the_generated_rule_replaces_a_spelling_rule_of_the_base() {
    // The base defines `ste-words.spelling` too, with an error severity and a different
    // pair. Only the generated rule may report.
    let e = engine_over(custom_base("spelling"), "en-US", &[], false);
    let doc = e
        .lint(&Input::markdown("a.md", "The colour and the flavor.\n"))
        .unwrap();
    let got: Vec<(String, String)> = doc
        .findings
        .iter()
        .map(|f| (f.matched_text.clone(), format!("{:?}", f.severity)))
        .collect();
    assert_eq!(got, [("colour".to_owned(), "Warning".to_owned())]);
}
