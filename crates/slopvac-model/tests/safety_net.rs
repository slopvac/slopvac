//! Safety net for the report types. Three checks, none tied to one field:
//!
//! 1. every invalid conformance fixture is rejected by Rust `Deserialize`;
//! 2. whenever a randomly mutated valid report decodes, the Rust output still validates
//!    against the PUBLISHED schema;
//! 3. a report built through public constructors serializes schema-valid.
//!
//! Set `PROPTEST_CASES` to change the case count (default 256).

#![allow(missing_docs)]

mod common;

use std::num::NonZeroUsize;

use common::{corpus, instances, load, patch_layer, published_lint_report};
use proptest::prelude::*;
use proptest::sample::Index;
use serde_json::{Value, json};
use slopvac_model::{
    AiRegister, AiRegisterSummary, AiSignal, ApiVersion, CategoryId, CategoryScore,
    CategorySummary, ConfigLayer, ConfigSource, ContextTag, Contexts, Density, Document, Edit,
    Extent, Finding, Fix, Gate, GenreId, GenreResult, GenreSource, KebabName, LineCol,
    NonEmptyString, Notice, NoticeLevel, PackId, Report, RuleId, RuleProvenance, RunSummary,
    SchemaVersion, Score, Severity, Span,
};

/// Invalid report fixtures whose only defect is an unknown member. Consumers ignore unknown
/// members by design, so Rust accepts them; the property is then that its output is valid.
const IGNORED_MEMBER_ONLY: &[&str] = &[
    "2x-notes-key.json",
    "document-2x-passed-key.json",
    "finding-2x-replacement.json",
    "finding-origin-range-extension-key.json",
];

fn decodes_as(name: &str, value: &Value) -> bool {
    let prefix = name.split("--").next().unwrap();
    let v = value.clone();
    match prefix {
        "Span" => serde_json::from_value::<Span>(v).is_ok(),
        "LineCol" => serde_json::from_value::<LineCol>(v).is_ok(),
        "Edit" => serde_json::from_value::<Edit>(v).is_ok(),
        "PackId" => serde_json::from_value::<PackId>(v).is_ok(),
        "RuleId" => serde_json::from_value::<RuleId>(v).is_ok(),
        "Severity" => serde_json::from_value::<Severity>(v).is_ok(),
        "RuleProvenance" => serde_json::from_value::<RuleProvenance>(v).is_ok(),
        "ConfigSource" => serde_json::from_value::<ConfigSource>(v).is_ok(),
        "ContextTag" => serde_json::from_value::<ContextTag>(v).is_ok(),
        "SchemaVersion" => serde_json::from_value::<SchemaVersion>(v).is_ok(),
        "ApiVersion" => serde_json::from_value::<ApiVersion>(v).is_ok(),
        other => panic!("no Rust type mapped for common fixture prefix {other}"),
    }
}

#[test]
fn every_invalid_common_fixture_is_rejected_by_deserialize() {
    let dir = corpus().join("common/invalid");
    let files = instances(&dir);
    assert!(files.len() >= 12, "only {} fixtures found", files.len());
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            !decodes_as(&name, &load(&path)),
            "{name} must be rejected by Deserialize"
        );
    }
}

#[test]
fn every_valid_common_fixture_is_accepted_by_deserialize() {
    for path in instances(&corpus().join("common/valid")) {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            decodes_as(&name, &load(&path)),
            "{name} must be accepted by Deserialize"
        );
    }
}

#[test]
fn every_invalid_report_fixture_is_rejected_or_harmlessly_ignored() {
    let validator = published_lint_report();
    let files = instances(&corpus().join("lint-report/invalid"));
    assert!(files.len() >= 25, "only {} fixtures found", files.len());
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let mut value = load(&path);
        patch_layer(&mut value);
        match serde_json::from_value::<Report>(value) {
            Err(_) => assert!(
                !IGNORED_MEMBER_ONLY.contains(&name.as_str()),
                "{name} was expected to decode with the unknown member ignored"
            ),
            Ok(report) => {
                assert!(
                    IGNORED_MEMBER_ONLY.contains(&name.as_str()),
                    "{name} must be rejected by Deserialize"
                );
                let out = serde_json::to_value(&report).unwrap();
                assert!(validator.is_valid(&out), "{name} re-serialized invalid");
            }
        }
    }
}

fn origins(report: &Value) -> Vec<Option<Value>> {
    report["documents"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|d| d["findings"].as_array().unwrap())
        .map(|f| f.get("origin").cloned())
        .collect()
}

#[test]
fn every_valid_report_fixture_decodes_and_reserializes_schema_valid() {
    let validator = published_lint_report();
    for path in instances(&corpus().join("lint-report/valid")) {
        let mut value = load(&path);
        patch_layer(&mut value);
        let report: Report = serde_json::from_value(value.clone()).unwrap();
        let out = serde_json::to_value(&report).unwrap();
        assert!(
            validator.is_valid(&out),
            "{} re-serialized invalid",
            path.display()
        );
        // `origin` must survive the round trip exactly, not just stay schema-valid.
        assert_eq!(
            origins(&value),
            origins(&out),
            "{} lost or changed an origin",
            path.display()
        );
    }
}

// ---- mutation property -------------------------------------------------------------

fn fixtures() -> Vec<Value> {
    instances(&corpus().join("lint-report/valid"))
        .into_iter()
        .map(|p| {
            let mut v = load(&p);
            patch_layer(&mut v);
            v
        })
        .collect()
}

fn paths(value: &Value, here: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    out.push(here.clone());
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                here.push(k.clone());
                paths(v, here, out);
                here.pop();
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                here.push(i.to_string());
                paths(v, here, out);
                here.pop();
            }
        }
        _ => {}
    }
}

fn at<'a>(value: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    let mut cur = value;
    for key in path {
        cur = match cur {
            Value::Object(map) => map.get_mut(key)?,
            Value::Array(items) => items.get_mut(key.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

#[derive(Debug, Clone, Copy)]
enum Mutation {
    Null,
    Remove,
    Negative,
    Zero,
    Over100,
    Huge,
    EmptyString,
    NotKebab,
    WrongTypeString,
    WrongTypeNumber,
    WrongTypeBool,
    WrongTypeArray,
    WrongTypeObject,
    DuplicateItem,
    EmptyArray,
}

fn apply(root: &mut Value, path: &[String], mutation: Mutation) {
    let Some((last, parent_path)) = path.split_last() else {
        // The root itself: replace it wholesale.
        *root = replacement(mutation, root);
        return;
    };
    let Some(parent) = at(root, parent_path) else {
        return;
    };
    match parent {
        Value::Object(map) => {
            if matches!(mutation, Mutation::Remove) {
                map.remove(last);
            } else if let Some(slot) = map.get_mut(last) {
                *slot = replacement(mutation, slot);
            }
        }
        Value::Array(items) => {
            let Ok(i) = last.parse::<usize>() else { return };
            if matches!(mutation, Mutation::Remove) {
                if i < items.len() {
                    items.remove(i);
                }
            } else if let Some(slot) = items.get_mut(i) {
                if matches!(mutation, Mutation::DuplicateItem) {
                    let copy = slot.clone();
                    items.insert(i, copy);
                } else {
                    *slot = replacement(mutation, slot);
                }
            }
        }
        _ => {}
    }
}

fn replacement(mutation: Mutation, current: &Value) -> Value {
    match mutation {
        Mutation::Null | Mutation::Remove | Mutation::DuplicateItem => Value::Null,
        Mutation::Negative => json!(-1),
        Mutation::Zero => json!(0),
        Mutation::Over100 => json!(101),
        Mutation::Huge => json!(1.0e308),
        Mutation::EmptyString => json!(""),
        Mutation::NotKebab => json!("Not Kebab_Case"),
        Mutation::WrongTypeString => json!("x"),
        Mutation::WrongTypeNumber => json!(7),
        Mutation::WrongTypeBool => json!(true),
        Mutation::WrongTypeArray => json!([]),
        Mutation::WrongTypeObject => json!({}),
        Mutation::EmptyArray => match current {
            Value::Array(_) => json!([]),
            _ => Value::Null,
        },
    }
}

fn mutation() -> impl Strategy<Value = Mutation> {
    prop_oneof![
        Just(Mutation::Null),
        Just(Mutation::Remove),
        Just(Mutation::Negative),
        Just(Mutation::Zero),
        Just(Mutation::Over100),
        Just(Mutation::Huge),
        Just(Mutation::EmptyString),
        Just(Mutation::NotKebab),
        Just(Mutation::WrongTypeString),
        Just(Mutation::WrongTypeNumber),
        Just(Mutation::WrongTypeBool),
        Just(Mutation::WrongTypeArray),
        Just(Mutation::WrongTypeObject),
        Just(Mutation::DuplicateItem),
        Just(Mutation::EmptyArray),
    ]
}

proptest! {
    /// Whatever Rust decodes, it must write back in a shape the published schema accepts.
    #[test]
    fn decoded_mutations_reserialize_schema_valid(
        fixture in any::<Index>(),
        edits in prop::collection::vec((any::<Index>(), mutation()), 1..4),
    ) {
        let validator = published_lint_report();
        let bases = fixtures();
        let mut value = bases[fixture.index(bases.len())].clone();
        for (target, kind) in edits {
            let mut all = Vec::new();
            paths(&value, &mut Vec::new(), &mut all);
            let path = all[target.index(all.len())].clone();
            apply(&mut value, &path, kind);
        }
        if let Ok(report) = serde_json::from_value::<Report>(value.clone()) {
            let out = serde_json::to_value(&report).unwrap();
            let errors: Vec<String> = validator
                .iter_errors(&out)
                .map(|e| format!("{e} at {}", e.instance_path()))
                .collect();
            prop_assert!(
                errors.is_empty(),
                "Rust accepted {value} but wrote invalid output: {errors:?}"
            );
        }
    }
}

// ---- constructor-built reports -----------------------------------------------------

fn summary(density: Density, count: usize) -> AiRegisterSummary {
    AiRegisterSummary {
        findings: count,
        errors: 0,
        warnings: count,
        per_100_words: density,
    }
}

#[derive(Debug, Clone)]
struct Params {
    density: f64,
    score: f64,
    count: usize,
    with_extent: bool,
    failed: bool,
    reason: String,
    layer: ConfigLayer,
    budget: Option<f64>,
}

fn built_finding(with_extent: bool) -> Finding {
    Finding {
        rule_id: RuleId::new("core:x.y").unwrap(),
        pack: PackId::new("core").unwrap(),
        category: CategoryId::new("orwell").unwrap(),
        severity: Severity::Warning,
        message: NonEmptyString::new("m").unwrap(),
        start: LineCol::new(1, 1).unwrap(),
        extent: with_extent.then(|| Extent {
            span: Span::new(0, 4).unwrap(),
            end: LineCol::new(1, 5).unwrap(),
        }),
        contexts: Contexts::new(vec![ContextTag::new("prose").unwrap()]).unwrap(),
        ai_signal: AiSignal::Weak,
        matched_text: "text".into(),
        fix: Some(
            Fix::new(vec![Edit {
                span: Span::new(0, 4).unwrap(),
                text: "t".into(),
            }])
            .unwrap(),
        ),
        provenance: None,
        origin: None,
    }
}

fn built_report(p: &Params) -> Report {
    let Params {
        count,
        with_extent,
        failed,
        layer,
        ..
    } = *p;
    let (reason, budget) = (&p.reason, p.budget);
    let density = Density::new(p.density).unwrap();
    let score = Score::new(p.score).unwrap();
    let register = AiRegister {
        strong: summary(density, count),
        weak: summary(density, 0),
    };
    let finding = built_finding(with_extent);
    let gate = if failed {
        Gate::failed(vec![reason.clone()]).unwrap()
    } else {
        Gate::passed()
    };
    Report {
        schema_version: SchemaVersion,
        slopvac_version: NonEmptyString::new("3.0.0").unwrap(),
        summary: RunSummary {
            documents: 1,
            words: 100,
            findings: count,
            errors: 0,
            warnings: count,
            suggestions: 0,
            per_100_words: density,
            score,
            passed: !failed,
            categories: vec![CategorySummary {
                category: CategoryId::new("orwell").unwrap(),
                findings: count,
                errors: 0,
                warnings: count,
                suggestions: 0,
                per_100_words: density,
                score,
            }],
            ai_register: register.clone(),
            prose: summary(density, count),
        },
        documents: vec![Document {
            path: NonEmptyString::new("a.md").unwrap(),
            profile: NonEmptyString::new("strict").unwrap(),
            genre: GenreResult {
                id: GenreId::new("internal").unwrap(),
                source: GenreSource::Detected,
                reasons: vec![slopvac_model::GenreReason {
                    signal: KebabName::new("path-glob").unwrap(),
                    detail: String::new(),
                }],
            },
            contexts_seen: Contexts::new(vec![ContextTag::new("prose").unwrap()]).unwrap(),
            config_sources: vec![ConfigSource {
                layer,
                file: None,
                line: NonZeroUsize::new(2),
                column: None,
            }],
            words: 100,
            sentences: 5,
            paragraphs: 2,
            findings: vec![finding],
            categories: vec![CategoryScore {
                category: CategoryId::new("orwell").unwrap(),
                findings: count,
                errors: 0,
                warnings: count,
                suggestions: 0,
                per_100_words: density,
                gating_per_100_words: density,
                budget: budget.map(|b| Density::new(b).unwrap()),
                score,
                over_budget: false,
            }],
            total_findings: count,
            errors: 0,
            warnings: count,
            suggestions: 0,
            per_100_words: density,
            score,
            gate,
            unchecked: vec![Notice {
                code: KebabName::new("vale-not-found").unwrap(),
                level: NoticeLevel::Info,
                message: NonEmptyString::new("n").unwrap(),
                path: None,
            }],
            ai_register: register,
            prose: summary(density, count),
        }],
        notices: vec![],
    }
}

fn layer() -> impl Strategy<Value = ConfigLayer> {
    prop_oneof![
        Just(ConfigLayer::Core),
        Just(ConfigLayer::FrontMatter),
        Just(ConfigLayer::Cli),
        Just(ConfigLayer::Env),
    ]
}

proptest! {
    #[test]
    fn constructor_built_reports_serialize_schema_valid(
        density in 0.0f64..1.0e9,
        score in 0.0f64..=100.0,
        count in 0usize..1000,
        with_extent in any::<bool>(),
        failed in any::<bool>(),
        reason in "[a-z ]{1,20}",
        layer in layer(),
        budget in prop::option::of(0.0f64..1.0e6),
    ) {
        let report = built_report(&Params {
            density, score, count, with_extent, failed, reason, layer, budget,
        });
        let out = serde_json::to_value(&report).unwrap();
        let validator = published_lint_report();
        let errors: Vec<String> = validator
            .iter_errors(&out)
            .map(|e| format!("{e} at {}", e.instance_path()))
            .collect();
        prop_assert!(errors.is_empty(), "{errors:?}");
        let back: Report = serde_json::from_value(out).unwrap();
        prop_assert_eq!(back, report);
    }
}

#[test]
fn scalar_constructors_reject_what_the_schema_rejects() {
    for bad in [-0.1, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(Density::new(bad).is_err(), "density {bad}");
    }
    for bad in [-0.1, 100.1, f64::NAN, f64::INFINITY] {
        assert!(Score::new(bad).is_err(), "score {bad}");
    }
    assert!(Score::new(0.0).is_ok() && Score::new(100.0).is_ok());
    assert!(NonEmptyString::new("").is_err());
    assert!(CategoryId::new("Orwell").is_err() && GenreId::new("1x").is_err());
    for bad in ["", "Code", "a--b", "a-", "-a", "a_b"] {
        assert!(KebabName::new(bad).is_err(), "kebab {bad:?}");
    }
    assert!(KebabName::new("vale-not-found").is_ok());
}
