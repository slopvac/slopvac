//! The identifier and version decoders must reject what the shared corpus marks invalid,
//! without going through a generated schema.
#![cfg(test)]

use std::fs;
use std::path::Path;

use serde::de::DeserializeOwned;
use slopvac_model::{ApiVersion, ContextTag, PackId, RuleId, SchemaVersion};

fn corpus(kind: &str, name: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/v3/conformance/common")
        .join(kind)
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn rejects<T: DeserializeOwned>(file: &str) {
    assert!(
        serde_json::from_value::<T>(corpus("invalid", file)).is_err(),
        "{file} must be rejected by the decoder"
    );
}

#[test]
fn pack_id_rejects_uppercase() {
    rejects::<PackId>("PackId--uppercase.json");
}

#[test]
fn rule_id_rejects_unqualified() {
    rejects::<RuleId>("RuleId--unqualified.json");
}

#[test]
fn context_tag_rejects_spaces() {
    rejects::<ContextTag>("ContextTag--spaces.json");
}

#[test]
fn schema_version_rejects_one() {
    rejects::<SchemaVersion>("SchemaVersion--one.json");
}

#[test]
fn api_version_rejects_zero_and_strings() {
    rejects::<ApiVersion>("ApiVersion--zero.json");
    rejects::<ApiVersion>("ApiVersion--string.json");
}

#[test]
fn accepted_corpus_values_decode() {
    let pack: PackId = serde_json::from_value(corpus("valid", "PackId--core.json")).unwrap();
    assert_eq!(pack.as_str(), "core");
    let api: ApiVersion = serde_json::from_value(corpus("valid", "ApiVersion--one.json")).unwrap();
    assert_eq!(api.get(), 1);
    assert!(
        serde_json::from_value::<SchemaVersion>(corpus("valid", "SchemaVersion--three.json"))
            .is_ok()
    );
}

#[test]
fn schema_version_serializes_as_the_integer_three() {
    assert_eq!(serde_json::to_string(&SchemaVersion).unwrap(), "3");
}
