//! The schemas generated from the Rust types must accept and reject exactly what the
//! shared conformance corpus (`schemas/v3/conformance`) says. The plugin-protocol corpus
//! belongs to the plugin host's types, not to this crate.
#![cfg(test)]

mod common;

use std::path::Path;

use common::{corpus, instances, load, patch_layer};
use schemars::generate::SchemaSettings;
use schemars::{Schema, schema_for};
use slopvac_model::{
    ApiVersion, ConfigSource, ContextTag, Edit, LineCol, PackId, Report, RuleId, RuleProvenance,
    SchemaVersion, Severity, Span,
};

fn check(schema: &Schema, dir: &Path, only_prefix: Option<&str>) -> usize {
    let value = serde_json::to_value(schema).unwrap();
    let validator = jsonschema::draft202012::new(&value).unwrap();
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
                "{} should be {sub} for the generated schema",
                path.display()
            );
            checked += 1;
        }
    }
    checked
}

#[test]
fn generated_lint_report_schema_matches_the_corpus() {
    // The producer contract: what `Report` serializes, so optional-on-input fields that the
    // producer always writes (`budget`) stay required and `Option` fields are not nullable
    // unless the published schema says so.
    let schema = SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator()
        .into_root_schema_for::<Report>();
    let n = check(&schema, &corpus().join("lint-report"), None);
    assert!(n >= 20, "only {n} lint-report instances ran");
}

#[test]
fn generated_common_schemas_match_the_corpus() {
    let common = corpus().join("common");
    let defs: Vec<(&str, Schema)> = vec![
        ("Span", schema_for!(Span)),
        ("LineCol", schema_for!(LineCol)),
        ("Edit", schema_for!(Edit)),
        ("PackId", schema_for!(PackId)),
        ("RuleId", schema_for!(RuleId)),
        ("Severity", schema_for!(Severity)),
        ("RuleProvenance", schema_for!(RuleProvenance)),
        ("ConfigSource", schema_for!(ConfigSource)),
        ("ContextTag", schema_for!(ContextTag)),
        ("SchemaVersion", schema_for!(SchemaVersion)),
        ("ApiVersion", schema_for!(ApiVersion)),
    ];
    let mut total = 0;
    for (name, schema) in &defs {
        let n = check(schema, &common, Some(name));
        assert!(n > 0, "no corpus instance for {name}");
        total += n;
    }
    let all: usize = ["valid", "invalid"]
        .iter()
        .map(|s| instances(&common.join(s)).len())
        .sum();
    assert_eq!(
        total, all,
        "every common corpus instance must map to a generated schema"
    );
}
