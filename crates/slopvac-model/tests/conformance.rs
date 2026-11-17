//! The published schemas accept and reject exactly what the shared conformance corpus
//! (`schemas/v3/conformance`) says, judged by the Rust jsonschema crate. `slopvac-schemas
//! check` keeps the published files equal to what the Rust types generate. The
//! plugin-protocol corpus belongs to the plugin host's types, not to this crate.

#![allow(missing_docs)]

mod common;

use std::fs;
use std::path::Path;

use common::{corpus, instances, load, patch_layer, published_lint_report, schemas};
use serde_json::{Value, json};

fn check(validator: &jsonschema::Validator, dir: &Path, only_prefix: Option<&str>) -> usize {
    let mut checked = 0;
    for (sub, want_valid) in [("valid", true), ("invalid", false)] {
        for path in instances(&dir.join(sub)) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if only_prefix.is_some_and(|p| !name.starts_with(&format!("{p}--"))) {
                continue;
            }
            let mut instance = load(&path);
            patch_layer(&mut instance);
            assert_eq!(
                validator.is_valid(&instance),
                want_valid,
                "{} should be {sub} for the published schema",
                path.display()
            );
            checked += 1;
        }
    }
    checked
}

/// A validator for one `$defs` entry of the published `common.schema.json`.
fn published_common_def(def: &str) -> jsonschema::Validator {
    let common: Value =
        serde_json::from_str(&fs::read_to_string(schemas().join("common.schema.json")).unwrap())
            .unwrap();
    let id = common["$id"].as_str().unwrap().to_owned();
    let registry = jsonschema::Registry::new()
        .add(id.clone(), common)
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(&json!({"$ref": format!("{id}#/$defs/{def}")}))
        .unwrap()
}

#[test]
fn the_published_lint_report_schema_judges_the_corpus() {
    let n = check(
        &published_lint_report(),
        &corpus().join("lint-report"),
        None,
    );
    assert!(n >= 20, "only {n} lint-report instances ran");
}

#[test]
fn the_published_common_schema_judges_the_corpus() {
    let common = corpus().join("common");
    let defs = [
        "Span",
        "LineCol",
        "Edit",
        "PackId",
        "RuleId",
        "Severity",
        "RuleProvenance",
        "ConfigSource",
        "ContextTag",
        "SchemaVersion",
        "ApiVersion",
    ];
    let mut total = 0;
    for name in defs {
        let n = check(&published_common_def(name), &common, Some(name));
        assert!(n > 0, "no corpus instance for {name}");
        total += n;
    }
    let all: usize = ["valid", "invalid"]
        .iter()
        .map(|s| instances(&common.join(s)).len())
        .sum();
    assert_eq!(
        total, all,
        "every common corpus instance must map to a published definition"
    );
}
