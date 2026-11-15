//! The genre and tagger structs agree with the published schemas, judged by the conformance
//! corpus (`schemas/v3/conformance/{genre,tagger}`):
//!
//! * the published schema accepts every valid file and rejects every invalid one at the
//!   pointer that `cases.toml` names;
//! * the structs load every valid file and reject every invalid one;
//! * the built-in genre and tagger files satisfy the same schemas.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use slopvac_context::{GenreSpec, TaggerSpec};

const BASE: &str = "https://srobroek.github.io/slopvac/schemas/v3";

fn schemas() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// A validator for one published schema. Its `$ref`s reach the other schemas by URL, so
/// every schema file is registered.
fn validator(file: &str) -> jsonschema::Validator {
    let mut registry = jsonschema::Registry::new();
    for entry in fs::read_dir(schemas()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "json") {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            registry = registry
                .add(format!("{BASE}/{name}"), read_json(&path))
                .unwrap();
        }
    }
    let registry = registry.prepare().unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(&serde_json::json!({ "$ref": format!("{BASE}/{file}") }))
        .unwrap()
}

fn load(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap();
    match path.extension().and_then(|e| e.to_str()) {
        Some("toml") => serde_json::to_value(text.parse::<toml::Table>().unwrap()).unwrap(),
        Some("json") => serde_json::from_str(&text).unwrap(),
        other => panic!("unknown corpus extension {other:?} in {}", path.display()),
    }
}

fn files(artifact: &str, sub: &str) -> Vec<PathBuf> {
    let dir = schemas().join("conformance").join(artifact).join(sub);
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap() != "cases.toml")
        .collect();
    out.sort();
    assert!(!out.is_empty(), "{artifact}/{sub} has no files");
    out
}

fn name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

/// File name to the JSON pointer of the defect, from `invalid/cases.toml`.
fn cases(artifact: &str) -> toml::Table {
    let path = schemas()
        .join("conformance")
        .join(artifact)
        .join("invalid/cases.toml");
    fs::read_to_string(path).unwrap().parse().unwrap()
}

/// Everything the crate checks when it loads a genre file.
fn load_genre(v: Value) -> Result<(), String> {
    let spec: GenreSpec = serde_json::from_value(v).map_err(|e| e.to_string())?;
    spec.validate().map_err(|e| e.to_string())
}

/// Everything the crate checks when it loads a tagger file.
fn load_tagger(v: Value) -> Result<(), String> {
    let spec: TaggerSpec = serde_json::from_value(v).map_err(|e| e.to_string())?;
    spec.compile().map(|_| ()).map_err(|e| e.to_string())
}

fn check_corpus(artifact: &str, schema: &str, load_one: fn(Value) -> Result<(), String>) {
    let v = validator(schema);
    for path in files(artifact, "valid") {
        let value = load(&path);
        assert!(v.is_valid(&value), "schema rejects {}", path.display());
        let loaded = load_one(value);
        assert!(loaded.is_ok(), "{}: {loaded:?}", name(&path));
    }
    let want = cases(artifact);
    let invalid = files(artifact, "invalid");
    assert_eq!(invalid.len(), want.len(), "cases.toml lists every file");
    for path in invalid {
        let value = load(&path);
        let pointer = want[&name(&path)].as_str().unwrap();
        let at: Vec<String> = v
            .iter_errors(&value)
            .map(|e| e.instance_path().to_string())
            .collect();
        assert!(
            at.iter().any(|p| p == pointer),
            "{}: schema errors at {at:?}, expected {pointer}",
            name(&path)
        );
        assert!(load_one(value).is_err(), "{} loads", name(&path));
    }
}

#[test]
fn genre_corpus_agrees_with_the_schema_and_the_struct() {
    check_corpus("genre", "genre.schema.json", load_genre);
}

#[test]
fn tagger_corpus_agrees_with_the_schema_and_the_struct() {
    check_corpus("tagger", "tagger.schema.json", load_tagger);
}

fn builtin(file: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/builtin")
        .join(file);
    serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn every_built_in_genre_satisfies_the_genre_schema() {
    let v = validator("genre.schema.json");
    let list = builtin("genres.yaml");
    let genres = list.as_array().unwrap();
    assert!(genres.len() >= 12, "{} genres", genres.len());
    for g in genres {
        let errors: Vec<String> = v.iter_errors(g).map(|e| e.to_string()).collect();
        assert!(errors.is_empty(), "{}: {errors:?}", g["id"]);
    }
}

#[test]
fn every_built_in_tagger_satisfies_the_tagger_schema() {
    let v = validator("tagger.schema.json");
    let list = builtin("taggers.yaml");
    let taggers = list.as_array().unwrap();
    assert_eq!(taggers.len(), 1);
    for t in taggers {
        let errors: Vec<String> = v.iter_errors(t).map(|e| e.to_string()).collect();
        assert!(errors.is_empty(), "{}: {errors:?}", t["id"]);
    }
}
