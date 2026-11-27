//! The real output of `slopvac claims` and of `slopvac review` validates against the generated
//! schemas, and a document that breaks the contract does not.
//!
//! Every negative case starts from a document that validates and changes exactly one thing. The
//! test asserts the exact validation errors, by keyword and instance path, so a case cannot pass
//! because of some other defect. The contract includes the invariants that the producers keep:
//! what a changed claim carries, which payload goes with which review type, and which members
//! come and go together.

use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;
use serde_json::{Value, json};
use slopvac_agents::claims;
use slopvac_agents::error::AgentsError;
use slopvac_agents::review::{
    self, ModelProvider, Payload, ProviderReply, ReportedItem, ReviewEnv,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const BASE: &str = "https://slopvac.github.io/slopvac/schemas/v3";
const ORIGINAL: &str = include_str!("../../slopvac-agents/tests/fixtures/claims/original.md");
const REVISED: &str = include_str!("../../slopvac-agents/tests/fixtures/claims/revised.md");

/// A provider that declares one model and reports the author's model for the first two items,
/// so the plan carries notices with item locations.
struct Reporting;

impl ModelProvider for Reporting {
    fn name(&self) -> &'static str {
        "reporting"
    }
    fn model(&self) -> &'static str {
        "claude-x"
    }
    fn review(&self, _: &Payload) -> Result<ProviderReply, AgentsError> {
        Ok(ProviderReply::new(
            "reported",
            vec![ReportedItem::new(0, "gpt-5"), ReportedItem::new(1, "gpt-5")],
        ))
    }
}

fn schemas() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3")
}

/// A validator for one published schema. Its `$ref`s reach the other schemas by URL, so every
/// schema file is registered. A missing file fails here with the file name.
fn validator(file: &str) -> Result<jsonschema::Validator, Box<dyn std::error::Error>> {
    let path = schemas().join(file);
    assert!(path.is_file(), "{file} is not generated under schemas/v3");
    let mut registry = jsonschema::Registry::new();
    for entry in fs::read_dir(schemas())? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "json") {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let schema: Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
            registry = registry.add(format!("{BASE}/{name}"), schema)?;
        }
    }
    let registry = registry.prepare()?;
    Ok(jsonschema::options()
        .with_registry(&registry)
        .build(&json!({ "$ref": format!("{BASE}/{file}") }))?)
}

fn files(dir: &Path) -> Result<(PathBuf, PathBuf), Box<dyn std::error::Error>> {
    let (old, new) = (dir.join("old.md"), dir.join("new.md"));
    fs::write(&old, ORIGINAL)?;
    fs::write(&new, REVISED)?;
    Ok((old, new))
}

fn claims_output() -> Result<Value, Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let (old, new) = files(dir.path())?;
    let matches = claims::command().try_get_matches_from([
        "claims".to_owned(),
        new.display().to_string(),
        "--original".to_owned(),
        old.display().to_string(),
    ])?;
    Ok(serde_json::from_str(&claims::run(&matches)?)?)
}

fn plan_output(extra: &[&str], env: &ReviewEnv<'_>) -> Result<Value, Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let (old, new) = files(dir.path())?;
    let mut argv = vec![
        "review".to_owned(),
        new.display().to_string(),
        "--original".to_owned(),
        old.display().to_string(),
    ];
    argv.extend(extra.iter().map(|s| (*s).to_owned()));
    let matches = review::command().try_get_matches_from(argv)?;
    Ok(serde_json::from_str(&review::run(&matches, env)?)?)
}

/// The dry-run plan of the fixture: an `update` review with a notice. Payload items 0 and 1
/// are `changed`, 2 is `added`, 3 to 5 are `dropped`.
fn dry_run_plan() -> Result<Value, Box<dyn std::error::Error>> {
    plan_output(
        &[
            "--dry-run",
            "--author-model",
            "gpt-5",
            "--reviewer-model",
            "gpt-5",
        ],
        &ReviewEnv::default(),
    )
}

/// A `rewrite` plan: the dry-run of the fixture with the type forced to `rewrite`, so the payload
/// is the one the producer builds.
fn rewrite_plan() -> Result<Value, Box<dyn std::error::Error>> {
    plan_output(
        &[
            "--dry-run",
            "--type",
            "rewrite",
            "--author-model",
            "gpt-5",
            "--reviewer-model",
            "gpt-5",
        ],
        &ReviewEnv::default(),
    )
}

/// A plan the way a `mechanical` review prints it: no payload, no models, no notices. The
/// fixture change is too large to ask for `mechanical`, so the plan is built from the
/// `update` plan.
fn mechanical_plan() -> Result<Value, Box<dyn std::error::Error>> {
    let mut plan = dry_run_plan()?;
    plan["review_type"] = json!("mechanical");
    plan["payload"] = json!({"kind": "none", "items": []});
    plan["uses_provider"] = json!(false);
    plan["reviewer_model_checked"] = json!(false);
    plan["notices"] = json!([]);
    let object = plan.as_object_mut().ok_or("the plan is not an object")?;
    object.remove("author_model");
    object.remove("reviewer_model");
    Ok(plan)
}

/// The claims report of the fixture, with a dropped sentence that loses a literal and that the
/// revision covers well: only the lost literal makes it a fact candidate.
fn claims_lost_literal_only() -> Result<Value, Box<dyn std::error::Error>> {
    let mut report = claims_output()?;
    report["dropped"][0]["token_coverage"] = json!(0.9);
    Ok(report)
}

fn assert_valid(validator: &jsonschema::Validator, instance: &Value) {
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

/// Set the value at a JSON pointer: replace it, or add the member when the parent object lacks it.
/// A pointer into nothing leaves the document as it was, which the case then reports.
fn set(document: &mut Value, pointer: &str, value: Value) {
    if let Some(slot) = document.pointer_mut(pointer) {
        *slot = value;
    } else if let Some((parent, key)) = pointer.rsplit_once('/')
        && let Some(object) = document.pointer_mut(parent).and_then(Value::as_object_mut)
    {
        object.insert(key.to_owned(), value);
    }
}

/// Remove an object member.
fn remove(document: &mut Value, pointer: &str, key: &str) {
    if let Some(object) = document.pointer_mut(pointer).and_then(Value::as_object_mut) {
        object.remove(key);
    }
}

#[test]
fn the_claims_report_validates() -> TestResult {
    assert_valid(&validator("claims.schema.json")?, &claims_output()?);
    Ok(())
}

#[test]
fn a_dry_run_plan_validates() -> TestResult {
    assert_valid(&validator("review-plan.schema.json")?, &dry_run_plan()?);
    Ok(())
}

#[test]
fn a_rewrite_plan_validates() -> TestResult {
    assert_valid(&validator("review-plan.schema.json")?, &rewrite_plan()?);
    Ok(())
}

#[test]
fn a_mechanical_plan_validates() -> TestResult {
    assert_valid(&validator("review-plan.schema.json")?, &mechanical_plan()?);
    Ok(())
}

#[test]
fn a_plan_with_a_provider_report_and_located_items_validates() -> TestResult {
    let env = ReviewEnv {
        provider: Some(&Reporting),
        ..ReviewEnv::default()
    };
    let plan = plan_output(&["--author-model", "gpt-5"], &env)?;
    assert!(plan["report"].is_string(), "{plan}");
    assert!(
        plan["notices"]
            .as_array()
            .ok_or("no notices")?
            .iter()
            .any(|n| n["data"]["items"][0]["role"].is_string()),
        "the plan carries no located items: {plan}"
    );
    assert_valid(&validator("review-plan.schema.json")?, &plan);
    Ok(())
}

/// The exact errors of a document, as sorted `(keyword, instance path)` pairs.
fn errors_of(validator: &jsonschema::Validator, document: &Value) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = validator
        .iter_errors(document)
        .map(|e| (e.kind().keyword().to_owned(), e.instance_path().to_string()))
        .collect();
    found.sort();
    found
}

const CLAIMS: &str = "claims.schema.json";
const PLAN: &str = "review-plan.schema.json";

/// Check that `start` validates, apply one `change`, and require exactly the `expect` errors,
/// each as `(keyword, instance path)`.
fn assert_exact(
    file: &str,
    mut start: Value,
    change: fn(&mut Value),
    expect: &[(&str, &str)],
) -> TestResult {
    let validator = validator(file)?;
    let before = errors_of(&validator, &start);
    assert!(before.is_empty(), "the start already fails: {before:?}");
    change(&mut start);
    let mut expected: Vec<(String, String)> = expect
        .iter()
        .map(|(k, p)| ((*k).to_owned(), (*p).to_owned()))
        .collect();
    expected.sort();
    assert_eq!(errors_of(&validator, &start), expected, "{start}");
    Ok(())
}

/// The fixture claims report holds two `changed` claims (0 and 1), one `added` claim (2) and
/// three dropped sentences, all fact candidates. Dropped 0 loses a literal and has coverage 0.
/// Dropped 1 loses none and has coverage 0.2. Each case changes one thing.
#[rstest]
#[case::missing_summary(|d: &mut Value| remove(d, "", "summary"), &[("required", "")])]
#[case::a_count_as_text(|d: &mut Value| set(d, "/summary/changed", json!("two")), &[("type", "/summary/changed")])]
#[case::a_negative_line(|d: &mut Value| set(d, "/claims/0/line", json!(-1)), &[("minimum", "/claims/0/line")])]
#[case::an_unknown_change(|d: &mut Value| set(d, "/claims/0/change", json!("moved")), &[("enum", "/claims/0/change")])]
#[case::similarity_as_text(|d: &mut Value| set(d, "/claims/0/similarity", json!("high")), &[("type", "/claims/0/similarity")])]
#[case::a_literal_without_text(|d: &mut Value| set(d, "/claims/0/literals_added", json!([{"kind": "number"}])), &[("required", "/claims/0/literals_added/0")])]
#[case::the_old_schema_version(|d: &mut Value| set(d, "/schema_version", json!(1)), &[("const", "/schema_version")])]
#[case::dropped_is_not_a_list(|d: &mut Value| set(d, "/dropped", json!({})), &[("type", "/dropped")])]
#[case::a_changed_claim_without_its_original(|d: &mut Value| remove(d, "/claims/0", "original"), &[("required", "/claims/0")])]
#[case::a_changed_claim_without_its_original_line(|d: &mut Value| remove(d, "/claims/0", "original_line"), &[("required", "/claims/0")])]
#[case::a_changed_claim_below_the_alignment_floor(|d: &mut Value| set(d, "/claims/0/similarity", json!(0.1)), &[("minimum", "/claims/0/similarity")])]
#[case::similarity_above_one(|d: &mut Value| set(d, "/claims/0/similarity", json!(1.5)), &[("maximum", "/claims/0/similarity")])]
#[case::an_added_claim_with_an_original(|d: &mut Value| set(d, "/claims/2/original", json!("A sentence.")), &[("not", "/claims/2")])]
#[case::an_added_claim_with_an_original_line(|d: &mut Value| set(d, "/claims/2/original_line", json!(3)), &[("not", "/claims/2")])]
#[case::an_added_claim_with_a_similarity(|d: &mut Value| set(d, "/claims/2/similarity", json!(0.5)), &[("const", "/claims/2/similarity")])]
#[case::an_added_claim_with_removed_literals(|d: &mut Value| {
                set(
                    d,
                    "/claims/2/literals_removed",
                    json!([{"kind": "number", "text": "1"}]),
                );
            }, &[("maxItems", "/claims/2/literals_removed")])]
#[case::an_added_claim_with_original_negations(|d: &mut Value| set(d, "/claims/2/negations_original", json!(1)), &[("const", "/claims/2/negations_original")])]
#[case::an_added_claim_with_a_negation_change(|d: &mut Value| set(d, "/claims/2/negation_changed", json!(true)), &[("const", "/claims/2/negation_changed")])]
#[case::a_claim_line_of_zero(|d: &mut Value| set(d, "/claims/0/line", json!(0)), &[("minimum", "/claims/0/line")])]
#[case::an_original_line_of_zero(|d: &mut Value| set(d, "/claims/0/original_line", json!(0)), &[("minimum", "/claims/0/original_line")])]
#[case::a_dropped_line_of_zero(|d: &mut Value| set(d, "/dropped/0/line", json!(0)), &[("minimum", "/dropped/0/line")])]
#[case::coverage_above_one(|d: &mut Value| set(d, "/dropped/1/token_coverage", json!(1.5)), &[("maximum", "/dropped/1/token_coverage")])]
#[case::negative_coverage(|d: &mut Value| set(d, "/dropped/1/token_coverage", json!(-0.1)), &[("minimum", "/dropped/1/token_coverage")])]
#[case::low_coverage_without_a_fact_candidate(|d: &mut Value| set(d, "/dropped/1/fact_candidate", json!(false)), &[("const", "/dropped/1/fact_candidate")])]
fn a_claims_report_that_breaks_the_contract_fails_on_exactly_that(
    #[case] change: fn(&mut Value),
    #[case] expect: &[(&str, &str)],
) -> TestResult {
    assert_exact(CLAIMS, claims_output()?, change, expect)
}

/// A dropped sentence that loses a literal and that the revision covers well: only the lost
/// literal makes it a fact candidate.
#[rstest]
#[case::a_lost_literal_without_a_fact_candidate(
    |d: &mut Value| set(d, "/dropped/0/fact_candidate", json!(false)),
    &[("const", "/dropped/0/fact_candidate")],
)]
fn a_lost_literal_alone_makes_a_fact_candidate(
    #[case] change: fn(&mut Value),
    #[case] expect: &[(&str, &str)],
) -> TestResult {
    assert_exact(CLAIMS, claims_lost_literal_only()?, change, expect)
}

/// The fixture `update` plan holds payload items 0 and 1 (`changed`), 2 (`added`) and 3 to 5
/// (`dropped`), and the models are compared. Each case changes one thing.
#[rstest]
#[case::missing_scope(|d: &mut Value| remove(d, "", "scope"), &[("required", "")])]
#[case::missing_schema_version(|d: &mut Value| remove(d, "", "schema_version"), &[("required", "")])]
#[case::an_unknown_review_type(|d: &mut Value| set(d, "/review_type", json!("deep")), &[("enum", "/review_type")])]
#[case::models_compared_as_text(|d: &mut Value| set(d, "/reviewer_model_checked", json!("yes")), &[("type", "/reviewer_model_checked")])]
#[case::a_payload_item_without_a_line(|d: &mut Value| remove(d, "/payload/items/0", "line"), &[("required", "/payload/items/0")])]
#[case::an_unknown_payload_role(|d: &mut Value| set(d, "/payload/items/2/role", json!("guess")), &[
                ("enum", "/payload/items/2/role"),
                ("enum", "/payload/items/2/role"),
            ])]
#[case::stats_count_as_text(|d: &mut Value| set(d, "/stats/churn", json!("many")), &[("type", "/stats/churn")])]
#[case::an_update_with_the_mechanical_payload(|d: &mut Value| set(d, "/payload", json!({"kind": "none", "items": []})), &[("const", "/payload/kind")])]
#[case::an_update_with_the_document_payload(|d: &mut Value| {
                set(
                    d,
                    "/payload",
                    json!({"kind": "document", "items": [
                        {"role": "original_document", "line": 1, "text": "a"},
                        {"role": "revised_document", "line": 1, "text": "b"},
                    ]}),
                );
            }, &[("const", "/payload/kind")])]
#[case::a_rewrite_with_the_passages_payload(|d: &mut Value| set(d, "/review_type", json!("rewrite")), &[("const", "/payload/kind")])]
#[case::an_update_without_a_provider(|d: &mut Value| set(d, "/uses_provider", json!(false)), &[("const", "/uses_provider")])]
#[case::a_document_item_among_passages(|d: &mut Value| set(d, "/payload/items/2/role", json!("revised_document")), &[("enum", "/payload/items/2/role")])]
#[case::a_changed_item_without_its_original(|d: &mut Value| remove(d, "/payload/items/0", "original"), &[("required", "/payload/items/0")])]
#[case::an_added_item_with_an_original(|d: &mut Value| set(d, "/payload/items/2/original", json!("x")), &[("not", "/payload/items/2")])]
#[case::a_dropped_item_with_an_original(|d: &mut Value| set(d, "/payload/items/3/original", json!("x")), &[("not", "/payload/items/3")])]
#[case::checked_models_without_an_author_model(|d: &mut Value| remove(d, "", "author_model"), &[("required", "")])]
#[case::checked_models_without_a_reviewer_model(|d: &mut Value| remove(d, "", "reviewer_model"), &[("required", "")])]
#[case::an_item_line_of_zero(|d: &mut Value| set(d, "/payload/items/0/line", json!(0)), &[("minimum", "/payload/items/0/line")])]
#[case::a_notice_without_a_code(|d: &mut Value| remove(d, "/notices/0", "code"), &[("required", "/notices/0")])]
#[case::uses_provider_as_text(|d: &mut Value| set(d, "/uses_provider", json!("yes")), &[("const", "/uses_provider"), ("type", "/uses_provider")])]
fn an_update_plan_that_breaks_the_contract_fails_on_exactly_that(
    #[case] change: fn(&mut Value),
    #[case] expect: &[(&str, &str)],
) -> TestResult {
    assert_exact(PLAN, dry_run_plan()?, change, expect)
}

/// The `rewrite` plan carries the two documents, in order, and a provider.
#[rstest]
#[case::a_rewrite_with_one_document(|d: &mut Value| {
                if let Some(items) = d["payload"]["items"].as_array_mut() {
                    items.truncate(1);
                }
            }, &[("minItems", "/payload/items")])]
#[case::a_rewrite_with_the_documents_swapped(|d: &mut Value| {
                if let Some(items) = d["payload"]["items"].as_array_mut() {
                    items.reverse();
                }
            }, &[
                ("const", "/payload/items/0/role"),
                ("const", "/payload/items/1/role"),
            ])]
#[case::a_rewrite_with_a_third_item(|d: &mut Value| {
                if let Some(items) = d["payload"]["items"].as_array_mut() {
                    items.push(json!({"role": "revised_document", "line": 1, "text": "c"}));
                }
            }, &[("maxItems", "/payload/items")])]
#[case::a_rewrite_without_a_provider(|d: &mut Value| set(d, "/uses_provider", json!(false)), &[("const", "/uses_provider")])]
fn a_rewrite_plan_that_breaks_the_contract_fails_on_exactly_that(
    #[case] change: fn(&mut Value),
    #[case] expect: &[(&str, &str)],
) -> TestResult {
    assert_exact(PLAN, rewrite_plan()?, change, expect)
}

/// A `mechanical` plan calls no provider, names no model, and carries no notice or report.
#[rstest]
#[case::a_mechanical_review_with_a_provider(|d: &mut Value| set(d, "/uses_provider", json!(true)), &[("const", "/uses_provider")])]
#[case::a_mechanical_review_with_an_author_model(|d: &mut Value| set(d, "/author_model", json!("gpt-5")), &[("not", "")])]
#[case::a_mechanical_review_with_a_reviewer_model(|d: &mut Value| set(d, "/reviewer_model", json!("gpt-5")), &[("not", "")])]
#[case::a_mechanical_review_that_compared_models(|d: &mut Value| set(d, "/reviewer_model_checked", json!(true)), &[("required", ""), ("required", "")])]
#[case::a_mechanical_review_with_a_notice(|d: &mut Value| {
                set(
                    d,
                    "/notices",
                    json!([{
                        "code": "review-model-same",
                        "level": "warning",
                        "message": "the reviewer model is the author's model",
                        "data": {
                            "author": "gpt-5",
                            "author_source": "flag",
                            "reviewer": "gpt-5",
                            "reviewer_source": "flag",
                        },
                    }]),
                );
            }, &[("maxItems", "/notices")])]
#[case::a_mechanical_review_with_a_report(|d: &mut Value| set(d, "/report", json!("r")), &[("not", "")])]
#[case::a_mechanical_review_with_the_passages_payload(|d: &mut Value| {
                set(
                    d,
                    "/payload",
                    json!({"kind": "passages", "items": [
                        {"role": "dropped", "line": 1, "text": "t"},
                    ]}),
                );
            }, &[("const", "/payload/kind")])]
#[case::a_none_payload_with_an_item(|d: &mut Value| {
                set(
                    d,
                    "/payload/items",
                    json!([{"role": "dropped", "line": 1, "text": "t"}]),
                );
            }, &[("maxItems", "/payload/items")])]
fn a_mechanical_plan_that_breaks_the_contract_fails_on_exactly_that(
    #[case] change: fn(&mut Value),
    #[case] expect: &[(&str, &str)],
) -> TestResult {
    assert_exact(PLAN, mechanical_plan()?, change, expect)
}
