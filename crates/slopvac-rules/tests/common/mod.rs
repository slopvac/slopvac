//! Shared helpers for the integration tests.

#![allow(
    dead_code,
    unreachable_pub,
    reason = "each test crate that includes this module uses a different subset"
)]

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

const BASE: &str = "https://srobroek.github.io/slopvac/schemas/v3";

pub fn manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// The repository's published `schemas/v3` directory: the schemas and their conformance
/// corpora.
pub fn schemas() -> PathBuf {
    manifest().join("../../schemas/v3")
}

pub fn load(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap();
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => serde_json::from_str(&text).unwrap(),
        Some("toml") => serde_json::to_value(text.parse::<toml::Table>().unwrap()).unwrap(),
        Some("yaml" | "yml") => serde_saphyr::from_str(&text).unwrap(),
        other => panic!("unknown corpus extension {other:?} in {}", path.display()),
    }
}

/// Instance files in `dir`, sorted, without the `cases.toml` index.
pub fn instances(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap() != "cases.toml")
        .collect();
    files.sort();
    files
}

pub fn file_name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

/// A registry holding the published `common`, `rule` and `category` schemas.
fn registry() -> jsonschema::Registry<'static> {
    let read = |path: PathBuf| -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    };
    let common = read(schemas().join("common.schema.json"));
    let rule = read(schemas().join("rule.schema.json"));
    let category = read(schemas().join("category.schema.json"));
    jsonschema::Registry::new()
        .add(format!("{BASE}/common.schema.json"), common)
        .unwrap()
        .add(format!("{BASE}/rule.schema.json"), rule)
        .unwrap()
        .add(format!("{BASE}/category.schema.json"), category)
        .unwrap()
        .prepare()
        .unwrap()
}

/// A validator for a published schema, or for one of its `$defs` (`Rule`).
pub fn published(file: &str, def: Option<&str>) -> jsonschema::Validator {
    let root = serde_json::json!({
        "$ref": def.map_or_else(
            || format!("{BASE}/{file}"),
            |d| format!("{BASE}/{file}#/$defs/{d}"),
        )
    });
    jsonschema::options()
        .with_registry(&registry())
        .build(&root)
        .unwrap()
}

/// A validator for a schema the crate generates.
pub fn generated(schema: &schemars::Schema) -> jsonschema::Validator {
    jsonschema::draft202012::new(&serde_json::to_value(schema).unwrap()).unwrap()
}
