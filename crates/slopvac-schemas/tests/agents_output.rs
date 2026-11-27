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

/// A `rewrite` plan: the two documents, in order, and a provider. The fixture change asks for
/// an `update`, so the plan is built from that one.
fn rewrite_plan() -> Result<Value, Box<dyn std::error::Error>> {
    let mut plan = dry_run_plan()?;
    plan["review_type"] = json!("rewrite");
    plan["payload"] = json!({"kind": "document", "items": [
        {"role": "original_document", "line": 1, "text": "a"},
        {"role": "revised_document", "line": 1, "text": "b"},
    ]});
    Ok(plan)
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

/// Set the value at a JSON pointer. A pointer that names nothing leaves the document as it was,
/// which the case then reports as a missing error.
fn set(document: &mut Value, pointer: &str, value: Value) {
    if let Some(slot) = document.pointer_mut(pointer) {
        *slot = value;
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

/// The valid document a case starts from.
#[derive(Clone, Copy)]
enum Start {
    Claims,
    ClaimsLostLiteralOnly,
    Update,
    Rewrite,
    Mechanical,
}

impl Start {
    fn document(self) -> Result<Value, Box<dyn std::error::Error>> {
        match self {
            Self::Claims => claims_output(),
            Self::ClaimsLostLiteralOnly => claims_lost_literal_only(),
            Self::Update => dry_run_plan(),
            Self::Rewrite => rewrite_plan(),
            Self::Mechanical => mechanical_plan(),
        }
    }
}

/// One negative case: a valid start, one change, and the exact errors it must give, each as
/// `(keyword, instance path)`.
struct Case {
    name: &'static str,
    file: &'static str,
    start: Start,
    change: fn(&mut Value),
    expect: &'static [(&'static str, &'static str)],
}

const CLAIMS: &str = "claims.schema.json";
const PLAN: &str = "review-plan.schema.json";

/// The fixture claims report holds two `changed` claims (0 and 1), one `added` claim (2) and
/// three dropped sentences, all fact candidates. Dropped 0 loses a literal and has coverage 0.
/// Dropped 1 loses none and has coverage 0.2.
fn claims_cases() -> Vec<Case> {
    let c = |name, change, expect| Case {
        name,
        file: CLAIMS,
        start: Start::Claims,
        change,
        expect,
    };
    vec![
        c(
            "claims: missing summary",
            |d| remove(d, "", "summary"),
            &[("required", "")],
        ),
        c(
            "claims: a count as text",
            |d| set(d, "/summary/changed", json!("two")),
            &[("type", "/summary/changed")],
        ),
        c(
            "claims: a negative line",
            |d| set(d, "/claims/0/line", json!(-1)),
            &[("minimum", "/claims/0/line")],
        ),
        c(
            "claims: an unknown change",
            |d| set(d, "/claims/0/change", json!("moved")),
            &[("enum", "/claims/0/change")],
        ),
        c(
            "claims: similarity as text",
            |d| set(d, "/claims/0/similarity", json!("high")),
            &[("type", "/claims/0/similarity")],
        ),
        c(
            "claims: a literal without text",
            |d| set(d, "/claims/0/literals_added", json!([{"kind": "number"}])),
            &[("required", "/claims/0/literals_added/0")],
        ),
        c(
            "claims: the old schema_version",
            |d| set(d, "/schema_version", json!(1)),
            &[("const", "/schema_version")],
        ),
        c(
            "claims: dropped is not a list",
            |d| set(d, "/dropped", json!({})),
            &[("type", "/dropped")],
        ),
        c(
            "claims: a changed claim without its original",
            |d| remove(d, "/claims/0", "original"),
            &[("required", "/claims/0")],
        ),
        c(
            "claims: a changed claim without its original line",
            |d| remove(d, "/claims/0", "original_line"),
            &[("required", "/claims/0")],
        ),
        c(
            "claims: a changed claim below the alignment floor",
            |d| set(d, "/claims/0/similarity", json!(0.1)),
            &[("minimum", "/claims/0/similarity")],
        ),
        c(
            "claims: similarity above one",
            |d| set(d, "/claims/0/similarity", json!(1.5)),
            &[("maximum", "/claims/0/similarity")],
        ),
        c(
            "claims: an added claim with an original",
            |d| set(d, "/claims/2/original", json!("A sentence.")),
            &[("falseSchema", "/claims/2/original")],
        ),
        c(
            "claims: an added claim with an original line",
            |d| set(d, "/claims/2/original_line", json!(3)),
            &[("falseSchema", "/claims/2/original_line")],
        ),
        c(
            "claims: an added claim with a similarity",
            |d| set(d, "/claims/2/similarity", json!(0.5)),
            &[("const", "/claims/2/similarity")],
        ),
        c(
            "claims: an added claim with removed literals",
            |d| {
                set(
                    d,
                    "/claims/2/literals_removed",
                    json!([{"kind": "number", "text": "1"}]),
                );
            },
            &[("maxItems", "/claims/2/literals_removed")],
        ),
        c(
            "claims: an added claim with original negations",
            |d| set(d, "/claims/2/negations_original", json!(1)),
            &[("const", "/claims/2/negations_original")],
        ),
        c(
            "claims: an added claim with a negation change",
            |d| set(d, "/claims/2/negation_changed", json!(true)),
            &[("const", "/claims/2/negation_changed")],
        ),
        c(
            "claims: a claim line of zero",
            |d| set(d, "/claims/0/line", json!(0)),
            &[("minimum", "/claims/0/line")],
        ),
        c(
            "claims: an original line of zero",
            |d| set(d, "/claims/0/original_line", json!(0)),
            &[("minimum", "/claims/0/original_line")],
        ),
        c(
            "claims: a dropped line of zero",
            |d| set(d, "/dropped/0/line", json!(0)),
            &[("minimum", "/dropped/0/line")],
        ),
        c(
            "claims: coverage above one",
            |d| set(d, "/dropped/1/token_coverage", json!(1.5)),
            &[("maximum", "/dropped/1/token_coverage")],
        ),
        c(
            "claims: negative coverage",
            |d| set(d, "/dropped/1/token_coverage", json!(-0.1)),
            &[("minimum", "/dropped/1/token_coverage")],
        ),
        c(
            "claims: low coverage without a fact candidate",
            |d| set(d, "/dropped/1/fact_candidate", json!(false)),
            &[("const", "/dropped/1/fact_candidate")],
        ),
        Case {
            name: "claims: a lost literal without a fact candidate",
            file: CLAIMS,
            start: Start::ClaimsLostLiteralOnly,
            change: |d| set(d, "/dropped/0/fact_candidate", json!(false)),
            expect: &[("const", "/dropped/0/fact_candidate")],
        },
    ]
}

/// The fixture `update` plan holds payload items 0 and 1 (`changed`), 2 (`added`) and 3 to 5
/// (`dropped`), and the models are compared.
fn update_cases() -> Vec<Case> {
    let c = |name, change, expect| Case {
        name,
        file: PLAN,
        start: Start::Update,
        change,
        expect,
    };
    vec![
        c(
            "plan: missing scope",
            |d| remove(d, "", "scope"),
            &[("required", "")],
        ),
        c(
            "plan: missing schema_version",
            |d| remove(d, "", "schema_version"),
            &[("required", "")],
        ),
        c(
            "plan: an unknown review type",
            |d| set(d, "/review_type", json!("deep")),
            &[("enum", "/review_type")],
        ),
        c(
            "plan: models compared as text",
            |d| set(d, "/reviewer_model_checked", json!("yes")),
            &[("type", "/reviewer_model_checked")],
        ),
        c(
            "plan: a payload item without a line",
            |d| remove(d, "/payload/items/0", "line"),
            &[("required", "/payload/items/0")],
        ),
        c(
            "plan: an unknown payload role",
            |d| set(d, "/payload/items/2/role", json!("guess")),
            &[
                ("enum", "/payload/items/2/role"),
                ("enum", "/payload/items/2/role"),
            ],
        ),
        c(
            "plan: stats count as text",
            |d| set(d, "/stats/churn", json!("many")),
            &[("type", "/stats/churn")],
        ),
        c(
            "plan: an update with the mechanical payload",
            |d| set(d, "/payload", json!({"kind": "none", "items": []})),
            &[("const", "/payload/kind")],
        ),
        c(
            "plan: an update with the document payload",
            |d| {
                set(
                    d,
                    "/payload",
                    json!({"kind": "document", "items": [
                        {"role": "original_document", "line": 1, "text": "a"},
                        {"role": "revised_document", "line": 1, "text": "b"},
                    ]}),
                );
            },
            &[("const", "/payload/kind")],
        ),
        c(
            "plan: a rewrite with the passages payload",
            |d| set(d, "/review_type", json!("rewrite")),
            &[("const", "/payload/kind")],
        ),
        c(
            "plan: an update without a provider",
            |d| set(d, "/uses_provider", json!(false)),
            &[("const", "/uses_provider")],
        ),
        c(
            "plan: a document item among passages",
            |d| set(d, "/payload/items/2/role", json!("revised_document")),
            &[("enum", "/payload/items/2/role")],
        ),
        c(
            "plan: a changed item without its original",
            |d| remove(d, "/payload/items/0", "original"),
            &[("required", "/payload/items/0")],
        ),
        c(
            "plan: an added item with an original",
            |d| set(d, "/payload/items/2/original", json!("x")),
            &[("falseSchema", "/payload/items/2/original")],
        ),
        c(
            "plan: a dropped item with an original",
            |d| set(d, "/payload/items/3/original", json!("x")),
            &[("falseSchema", "/payload/items/3/original")],
        ),
        c(
            "plan: checked models without an author model",
            |d| remove(d, "", "author_model"),
            &[("required", "")],
        ),
        c(
            "plan: checked models without a reviewer model",
            |d| remove(d, "", "reviewer_model"),
            &[("required", "")],
        ),
        c(
            "plan: an item line of zero",
            |d| set(d, "/payload/items/0/line", json!(0)),
            &[("minimum", "/payload/items/0/line")],
        ),
    ]
}

/// The `rewrite` plan carries the two documents, in order, and a provider.
fn rewrite_cases() -> Vec<Case> {
    let c = |name, change, expect| Case {
        name,
        file: PLAN,
        start: Start::Rewrite,
        change,
        expect,
    };
    vec![
        c(
            "plan: a rewrite with one document",
            |d| {
                if let Some(items) = d["payload"]["items"].as_array_mut() {
                    items.truncate(1);
                }
            },
            &[("minItems", "/payload/items")],
        ),
        c(
            "plan: a rewrite with the documents swapped",
            |d| {
                if let Some(items) = d["payload"]["items"].as_array_mut() {
                    items.reverse();
                }
            },
            &[
                ("const", "/payload/items/0/role"),
                ("const", "/payload/items/1/role"),
            ],
        ),
        c(
            "plan: a rewrite with a third item",
            |d| {
                if let Some(items) = d["payload"]["items"].as_array_mut() {
                    items.push(json!({"role": "revised_document", "line": 1, "text": "c"}));
                }
            },
            &[("maxItems", "/payload/items")],
        ),
        c(
            "plan: a rewrite without a provider",
            |d| set(d, "/uses_provider", json!(false)),
            &[("const", "/uses_provider")],
        ),
    ]
}

/// A `mechanical` plan calls no provider, names no model, and carries no notice or report.
fn mechanical_cases() -> Vec<Case> {
    let c = |name, change, expect| Case {
        name,
        file: PLAN,
        start: Start::Mechanical,
        change,
        expect,
    };
    vec![
        c(
            "plan: a mechanical review with a provider",
            |d| set(d, "/uses_provider", json!(true)),
            &[("const", "/uses_provider")],
        ),
        c(
            "plan: a mechanical review with an author model",
            |d| set(d, "/author_model", json!("gpt-5")),
            &[("falseSchema", "/author_model")],
        ),
        c(
            "plan: a mechanical review with a reviewer model",
            |d| set(d, "/reviewer_model", json!("gpt-5")),
            &[("falseSchema", "/reviewer_model")],
        ),
        c(
            "plan: a mechanical review that compared models",
            |d| set(d, "/reviewer_model_checked", json!(true)),
            &[("required", ""), ("required", "")],
        ),
        c(
            "plan: a mechanical review with a notice",
            |d| {
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
            },
            &[("maxItems", "/notices")],
        ),
        c(
            "plan: a mechanical review with a report",
            |d| set(d, "/report", json!("r")),
            &[("falseSchema", "/report")],
        ),
        c(
            "plan: a mechanical review with the passages payload",
            |d| {
                set(
                    d,
                    "/payload",
                    json!({"kind": "passages", "items": [
                        {"role": "dropped", "line": 1, "text": "t"},
                    ]}),
                );
            },
            &[("const", "/payload/kind")],
        ),
        c(
            "plan: a none payload with an item",
            |d| {
                set(
                    d,
                    "/payload/items",
                    json!([{"role": "dropped", "line": 1, "text": "t"}]),
                );
            },
            &[("maxItems", "/payload/items")],
        ),
    ]
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

#[test]
fn every_negative_case_changes_one_thing_and_fails_on_exactly_that() -> TestResult {
    let cases: Vec<Case> = claims_cases()
        .into_iter()
        .chain(update_cases())
        .chain(rewrite_cases())
        .chain(mechanical_cases())
        .collect();
    let mut problems = Vec::new();
    for case in cases {
        let validator = validator(case.file)?;
        let mut document = case.start.document()?;
        let before = errors_of(&validator, &document);
        if !before.is_empty() {
            problems.push(format!(
                "{}: the start already fails: {before:?}",
                case.name
            ));
            continue;
        }
        (case.change)(&mut document);
        let mut expected: Vec<(String, String)> = case
            .expect
            .iter()
            .map(|(k, p)| ((*k).to_owned(), (*p).to_owned()))
            .collect();
        expected.sort();
        let found = errors_of(&validator, &document);
        if found != expected {
            problems.push(format!(
                "{}: expected {expected:?}, found {found:?}",
                case.name
            ));
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
    Ok(())
}
