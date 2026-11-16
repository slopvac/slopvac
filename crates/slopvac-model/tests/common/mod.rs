//! Shared helpers for the conformance tests.

#![allow(
    dead_code,
    unreachable_pub,
    reason = "each test crate that includes this module uses a different subset"
)]

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3/conformance")
}

pub fn schemas() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3")
}

pub fn load(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap();
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => serde_json::from_str(&text).unwrap(),
        Some("toml") => serde_json::to_value(text.parse::<toml::Table>().unwrap()).unwrap(),
        Some("yaml") => serde_yaml_ng::from_str(&text).unwrap(),
        other => panic!("unknown corpus extension {other:?} in {}", path.display()),
    }
}

/// Instance files in `dir`, sorted, without the `cases.toml` index.
pub fn instances(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = read
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap() != "cases.toml")
        .collect();
    files.sort();
    files
}

/// Several lint-report fixtures still name the retired `defaults` layer, which the
/// published `ConfigSource` schema no longer allows. Until the fixtures are fixed, patch the
/// copy in memory so each case is judged on its intended defect alone.
pub fn patch_layer(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.get("layer").and_then(Value::as_str) == Some("defaults") {
                map.insert("layer".into(), Value::String("core".into()));
            }
            map.values_mut().for_each(patch_layer);
        }
        Value::Array(items) => items.iter_mut().for_each(patch_layer),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// A validator for the published `lint-report.schema.json`, with its `common` reference
/// resolved from the published `common.schema.json`.
pub fn published_lint_report() -> jsonschema::Validator {
    let read = |name: &str| -> Value {
        serde_json::from_str(&fs::read_to_string(schemas().join(name)).unwrap()).unwrap()
    };
    let common = read("common.schema.json");
    let report = read("lint-report.schema.json");
    let registry = jsonschema::Registry::new()
        .add(
            "https://srobroek.github.io/slopvac/schemas/v3/common.schema.json",
            common,
        )
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(&report)
        .unwrap()
}
