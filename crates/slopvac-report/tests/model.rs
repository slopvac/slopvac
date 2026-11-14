//! The Rust report model against the lint-report schema and conformance corpus.

use jsonschema::Registry;
use serde_json::Value;
use slopvac_report::model::Report;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schemas/v3");

fn read(path: &str) -> Value {
    let text = std::fs::read_to_string(format!("{ROOT}/{path}")).expect("file");
    serde_json::from_str(&text).expect("json")
}

fn validator() -> jsonschema::Validator {
    let common = read("common.schema.json");
    let report = read("lint-report.schema.json");
    let common_id = common["$id"].as_str().unwrap().to_owned();
    let registry = Registry::new()
        .add(common_id, common)
        .expect("register common")
        .prepare()
        .expect("registry");
    jsonschema::options()
        .with_registry(&registry)
        .build(&report)
        .expect("schema compiles")
}

fn normalise(value: &mut Value) {
    match value {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                *value = Value::from(f);
            }
        }
        Value::Object(map) => {
            map.retain(|k, _| !k.starts_with("x-"));
            map.values_mut().for_each(normalise);
        }
        Value::Array(items) => items.iter_mut().for_each(normalise),
        _ => {}
    }
}

#[test]
fn valid_corpus_round_trips_and_still_validates() {
    let validator = validator();
    let dir = format!("{ROOT}/conformance/lint-report/valid");
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).expect("corpus dir") {
        let path = entry.expect("entry").path();
        let mut original: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let report: Report = serde_json::from_value(original.clone())
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut written: Value = serde_json::from_str(&report.to_json()).unwrap();
        assert!(validator.is_valid(&written), "{}", path.display());
        normalise(&mut written);
        normalise(&mut original);
        assert_eq!(
            written,
            original,
            "{} changed on round trip",
            path.display()
        );
        seen += 1;
    }
    assert!(seen >= 3, "expected the corpus, found {seen} files");
}

#[test]
fn schema_version_other_than_3_does_not_parse() {
    let mut report = read("conformance/lint-report/valid/empty-run.json");
    report["schema_version"] = Value::from(2);
    assert!(serde_json::from_value::<Report>(report).is_err());
}

#[test]
fn json_output_ends_with_a_newline() {
    let report: Report =
        serde_json::from_value(read("conformance/lint-report/valid/empty-run.json")).unwrap();
    assert!(report.to_json().ends_with("}\n"));
}
