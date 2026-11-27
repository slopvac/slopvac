//! The real output of `slopvac claims` and of `slopvac review` validates against the generated
//! schemas, and a document that breaks the contract does not. The contract includes the
//! invariants that the producers keep: what a changed claim carries, which payload goes with
//! which review type, and which members come and go together.

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

/// The dry-run plan of the fixture: an `update` review with a notice.
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

fn assert_valid(validator: &jsonschema::Validator, instance: &Value) {
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
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

#[rstest]
#[case::missing_summary(|d: &mut Value| remove(d, "", "summary"))]
#[case::count_as_text(|d: &mut Value| d["summary"]["changed"] = json!("two"))]
#[case::negative_line(|d: &mut Value| d["claims"][0]["line"] = json!(-1))]
#[case::unknown_change(|d: &mut Value| d["claims"][0]["change"] = json!("moved"))]
#[case::similarity_as_text(|d: &mut Value| d["claims"][0]["similarity"] = json!("high"))]
#[case::literal_without_text(|d: &mut Value| d["claims"][0]["literals_added"] = json!([{"kind": "number"}]))]
#[case::old_version(|d: &mut Value| d["schema_version"] = json!(1))]
#[case::dropped_not_a_list(|d: &mut Value| d["dropped"] = json!({}))]
fn a_claims_report_that_breaks_the_contract_is_refused(
    #[case] break_it: fn(&mut Value),
) -> TestResult {
    let validator = validator("claims.schema.json")?;
    let mut document = claims_output()?;
    assert!(
        document["claims"][0]["change"].is_string(),
        "the fixture has no claim"
    );
    break_it(&mut document);
    assert!(!validator.is_valid(&document), "{document}");
    Ok(())
}

/// Each case breaks one invariant that the claims producer keeps. The fixture holds two
/// `changed` claims (0 and 1), one `added` claim (2) and three dropped sentences, all with
/// `fact_candidate` set.
#[rstest]
#[case::changed_without_original(|d: &mut Value| remove(d, "/claims/0", "original"))]
#[case::changed_without_original_line(|d: &mut Value| remove(d, "/claims/0", "original_line"))]
#[case::changed_below_the_alignment_floor(|d: &mut Value| d["claims"][0]["similarity"] = json!(0.1))]
#[case::added_with_original(|d: &mut Value| d["claims"][2]["original"] = json!("A sentence."))]
#[case::added_with_original_line(|d: &mut Value| d["claims"][2]["original_line"] = json!(3))]
#[case::added_with_similarity(|d: &mut Value| d["claims"][2]["similarity"] = json!(0.5))]
#[case::added_with_removed_literals(|d: &mut Value| d["claims"][2]["literals_removed"] = json!([{"kind": "number", "text": "1"}]))]
#[case::added_with_original_negations(|d: &mut Value| d["claims"][2]["negations_original"] = json!(1))]
#[case::added_with_negation_change(|d: &mut Value| d["claims"][2]["negation_changed"] = json!(true))]
#[case::similarity_above_one(|d: &mut Value| d["claims"][0]["similarity"] = json!(1.5))]
#[case::line_zero(|d: &mut Value| d["claims"][0]["line"] = json!(0))]
#[case::lost_literal_without_fact_candidate(|d: &mut Value| d["dropped"][0]["fact_candidate"] = json!(false))]
#[case::low_coverage_without_fact_candidate(|d: &mut Value| d["dropped"][1]["fact_candidate"] = json!(false))]
#[case::coverage_above_one(|d: &mut Value| d["dropped"][1]["token_coverage"] = json!(1.5))]
fn a_claims_report_that_breaks_an_output_invariant_is_refused(
    #[case] break_it: fn(&mut Value),
) -> TestResult {
    let validator = validator("claims.schema.json")?;
    let mut document = claims_output()?;
    assert_eq!(document["claims"][0]["change"], "changed");
    assert_eq!(document["claims"][2]["change"], "added");
    break_it(&mut document);
    assert!(!validator.is_valid(&document), "{document}");
    Ok(())
}

#[rstest]
#[case::missing_scope(|d: &mut Value| remove(d, "", "scope"))]
#[case::missing_version(|d: &mut Value| remove(d, "", "schema_version"))]
#[case::unknown_review_type(|d: &mut Value| d["review_type"] = json!("deep"))]
#[case::uses_provider_as_text(|d: &mut Value| d["uses_provider"] = json!("yes"))]
#[case::payload_item_without_line(|d: &mut Value| remove(d, "/payload/items/0", "line"))]
#[case::unknown_payload_role(|d: &mut Value| d["payload"]["items"][0]["role"] = json!("guess"))]
#[case::notice_without_code(|d: &mut Value| remove(d, "/notices/0", "code"))]
#[case::stats_count_as_text(|d: &mut Value| d["stats"]["churn"] = json!("many"))]
fn a_plan_that_breaks_the_contract_is_refused(#[case] break_it: fn(&mut Value)) -> TestResult {
    let validator = validator("review-plan.schema.json")?;
    let mut document = dry_run_plan()?;
    assert!(
        document["payload"]["items"][0]["role"].is_string() && document["notices"][0].is_object(),
        "the fixture plan has no payload item or notice: {document}"
    );
    break_it(&mut document);
    assert!(!validator.is_valid(&document), "{document}");
    Ok(())
}

/// Each case breaks one invariant that the review producer keeps, on the `update` plan of the
/// fixture (items 0 and 1 are `changed`, 2 is `added`, 3 to 5 are `dropped`).
#[rstest]
#[case::update_with_the_mechanical_payload(|d: &mut Value| d["payload"] = json!({"kind": "none", "items": []}))]
#[case::update_with_the_document_payload(|d: &mut Value| d["payload"] = json!({"kind": "document", "items": [
    {"role": "original_document", "line": 1, "text": "a"},
    {"role": "revised_document", "line": 1, "text": "b"},
]}))]
#[case::rewrite_with_passages(|d: &mut Value| d["review_type"] = json!("rewrite"))]
#[case::mechanical_with_passages(|d: &mut Value| d["review_type"] = json!("mechanical"))]
#[case::update_without_a_provider(|d: &mut Value| d["uses_provider"] = json!(false))]
#[case::none_payload_with_items(|d: &mut Value| d["payload"]["kind"] = json!("none"))]
#[case::passages_with_a_document_item(|d: &mut Value| d["payload"]["items"][0]["role"] = json!("revised_document"))]
#[case::changed_item_without_original(|d: &mut Value| remove(d, "/payload/items/0", "original"))]
#[case::added_item_with_original(|d: &mut Value| d["payload"]["items"][2]["original"] = json!("x"))]
#[case::dropped_item_with_original(|d: &mut Value| d["payload"]["items"][3]["original"] = json!("x"))]
#[case::checked_without_the_author_model(|d: &mut Value| remove(d, "", "author_model"))]
#[case::checked_without_the_reviewer_model(|d: &mut Value| remove(d, "", "reviewer_model"))]
#[case::item_line_zero(|d: &mut Value| d["payload"]["items"][0]["line"] = json!(0))]
fn an_update_plan_that_breaks_an_output_invariant_is_refused(
    #[case] break_it: fn(&mut Value),
) -> TestResult {
    let validator = validator("review-plan.schema.json")?;
    let mut document = dry_run_plan()?;
    assert_eq!(document["review_type"], "update");
    assert_eq!(document["reviewer_model_checked"], true);
    break_it(&mut document);
    assert!(!validator.is_valid(&document), "{document}");
    Ok(())
}

/// A `rewrite` plan carries the two documents, in that order.
#[rstest]
#[case::one_document(|d: &mut Value| remove_item(d, 1))]
#[case::documents_swapped(|d: &mut Value| reverse_items(d))]
#[case::a_third_item(|d: &mut Value| push_item(d))]
fn a_rewrite_plan_that_breaks_an_output_invariant_is_refused(
    #[case] break_it: fn(&mut Value),
) -> TestResult {
    let validator = validator("review-plan.schema.json")?;
    let mut document = dry_run_plan()?;
    document["review_type"] = json!("rewrite");
    document["payload"] = json!({"kind": "document", "items": [
        {"role": "original_document", "line": 1, "text": "a"},
        {"role": "revised_document", "line": 1, "text": "b"},
    ]});
    assert_valid(&validator, &document);
    break_it(&mut document);
    assert!(!validator.is_valid(&document), "{document}");
    Ok(())
}

fn reverse_items(document: &mut Value) {
    if let Some(items) = document["payload"]["items"].as_array_mut() {
        items.reverse();
    }
}

fn push_item(document: &mut Value) {
    if let Some(items) = document["payload"]["items"].as_array_mut() {
        items.push(json!({"role": "revised_document", "line": 1, "text": "c"}));
    }
}

fn remove_item(document: &mut Value, index: usize) {
    if let Some(items) = document["payload"]["items"].as_array_mut() {
        items.remove(index);
    }
}

/// A `mechanical` plan calls no provider, names no model, and carries no notice or report.
#[rstest]
#[case::with_a_provider(|d: &mut Value| d["uses_provider"] = json!(true))]
#[case::with_an_author_model(|d: &mut Value| d["author_model"] = json!("gpt-5"))]
#[case::with_a_reviewer_model(|d: &mut Value| d["reviewer_model"] = json!("gpt-5"))]
#[case::checked(|d: &mut Value| d["reviewer_model_checked"] = json!(true))]
#[case::with_a_notice(|d: &mut Value| d["notices"] = json!([{"code": "review-model-same", "level": "warning", "message": "m"}]))]
#[case::with_a_report(|d: &mut Value| d["report"] = json!("r"))]
#[case::with_a_payload_item(|d: &mut Value| d["payload"]["items"] = json!([{"role": "dropped", "line": 1, "text": "t"}]))]
fn a_mechanical_plan_that_breaks_an_output_invariant_is_refused(
    #[case] break_it: fn(&mut Value),
) -> TestResult {
    let validator = validator("review-plan.schema.json")?;
    let mut document = mechanical_plan()?;
    assert_valid(&validator, &document);
    break_it(&mut document);
    assert!(!validator.is_valid(&document), "{document}");
    Ok(())
}
