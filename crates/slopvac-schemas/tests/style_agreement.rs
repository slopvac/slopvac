//! The style readers and the JSON Schema for a style preset give the same verdict on every
//! case of the conformance corpus. Two readers judge a file: `validate_style` in
//! `slopvac-config`, which checks the shape and keeps spans, and `Style::parse` in
//! `slopvac-styles`, which loads the preset. The schema is what editors and other tools read.
//! This test holds all three to one verdict, for the generated file and the committed one.
#![allow(
    clippy::unwrap_used,
    reason = "clippy.toml exempts only code inside a test function, and the helpers here sit outside one"
)]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;
use serde_json::Value;
use slopvac_config::tree::parse_toml;
use slopvac_config::{Level, validate_style};
use slopvac_model::SchemaVersion;
use slopvac_schemas::generate;
use slopvac_styles::Style;

const BASE: &str = "https://slopvac.github.io/slopvac/schemas/v3/";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn committed(file: &str) -> Value {
    let text = fs::read_to_string(root().join("schemas/v3").join(file)).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn generated(file: &str) -> Value {
    let files = generate().unwrap();
    let found = files.iter().find(|g| g.file == file);
    assert!(found.is_some(), "{file} is not generated");
    serde_json::from_str(&found.unwrap().text).unwrap()
}

/// A validator for `schema` that resolves its references to the other two files.
fn validator(schema: &Value, common: &Value, config: &Value) -> jsonschema::Validator {
    let refs = jsonschema::Registry::new()
        .add(format!("{BASE}common.schema.json"), common)
        .unwrap()
        .add(format!("{BASE}config.schema.json"), config)
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&refs)
        .build(schema)
        .unwrap()
}

fn cases(outcome: &str) -> Vec<PathBuf> {
    let dir = root().join("schemas/v3/conformance/style").join(outcome);
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| !p.ends_with("cases.toml"))
        .collect();
    assert!(!found.is_empty(), "no style/{outcome} corpus file");
    found.sort();
    found
}

/// Every case on which a reader and the schema differ, or on which a reader differs from the
/// directory (`valid` or `invalid`) that holds the file.
fn disagreements(schema: &jsonschema::Validator) -> Vec<String> {
    let mut found = Vec::new();
    for outcome in ["valid", "invalid"] {
        let want = outcome == "valid";
        for path in cases(outcome) {
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let id = format!("style/{outcome}/{name}");
            let text = fs::read_to_string(&path).unwrap();
            let shape = validate_style(&parse_toml(&text).unwrap(), &text).is_empty();
            let loads = Style::parse(&name, &text).is_ok();
            assert_eq!(
                shape, want,
                "{id}: validate_style disagrees with the corpus"
            );
            assert_eq!(loads, want, "{id}: Style::parse disagrees with the corpus");
            let by_schema = schema.is_valid(&toml::from_str::<Value>(&text).unwrap());
            if by_schema != want {
                found.extend(Some(format!("{id}: readers {want}, schema {by_schema}")));
            }
        }
    }
    found
}

#[rstest]
#[case::generated(generated)]
#[case::committed(committed)]
fn the_schema_gives_the_readers_verdict_on_every_case(#[case] source: fn(&str) -> Value) {
    let schema = validator(
        &source("style.schema.json"),
        &source("common.schema.json"),
        &source("config.schema.json"),
    );
    assert_eq!(disagreements(&schema), Vec::<String>::new());
}

fn config_accepts(text: &str) -> bool {
    validate_style(&parse_toml(text).unwrap(), text).is_empty()
}

fn styles_accept(text: &str) -> bool {
    Style::parse("probe", text).is_ok()
}

/// `schema_version` takes one value in the schema, the config-side reader and the style
/// loader, and that value is `SchemaVersion::VALUE`.
#[test]
fn both_readers_accept_only_the_shared_schema_version() {
    for n in 0..=10_i64 {
        let text = format!("schema_version = {n}\n");
        let want = n == i64::from(SchemaVersion::VALUE);
        assert_eq!(config_accepts(&text), want, "validate_style on version {n}");
        assert_eq!(styles_accept(&text), want, "Style::parse on version {n}");
    }
}

/// The `minLength` of the field at `pointer`, followed through one `$ref` if it has one.
fn min_length(schema: &Value, pointer: &str) -> usize {
    let node = schema.pointer(pointer).unwrap();
    let node = match node.get("$ref").and_then(Value::as_str) {
        Some(target) => schema.pointer(target.trim_start_matches('#')).unwrap(),
        None => node,
    };
    usize::try_from(node["minLength"].as_u64().unwrap()).unwrap()
}

/// The three string fields of a style have one floor. The schema states it three times, and
/// the readers check it, so a string one character short of it fails every reader.
#[rstest]
#[case::description("/properties/description", "description = \"{}\"\n")]
#[case::allow_entry(
    "/$defs/StyleTerms/properties/allow/items",
    "[terms]\nallow = [\"{}\"]\n"
)]
#[case::blocklist_path(
    "/$defs/StyleTerms/properties/blocklists/items",
    "[terms]\nblocklists = [\"{}\"]\n"
)]
fn the_string_fields_share_one_floor_with_the_readers(#[case] pointer: &str, #[case] body: &str) {
    let schema = generated("style.schema.json");
    let floor = min_length(&schema, "/properties/description");
    assert_eq!(
        min_length(&schema, pointer),
        floor,
        "schema floor at {pointer}"
    );
    for (len, want) in [(floor - 1, false), (floor, true)] {
        let text = format!(
            "schema_version = {}\n{}",
            SchemaVersion::VALUE,
            body.replace("{}", &"a".repeat(len))
        );
        assert_eq!(
            config_accepts(&text),
            want,
            "validate_style, {len} characters"
        );
        assert_eq!(styles_accept(&text), want, "Style::parse, {len} characters");
    }
}

/// The level enum in the generated schema is the `Level` declaration, in its order. Both style
/// readers take the names from the same declaration, so this is the one place the schema side
/// is compared with it.
#[rstest]
#[case::generated(generated)]
#[case::committed(committed)]
fn the_schema_level_enum_is_the_level_declaration(#[case] source: fn(&str) -> Value) {
    let schema = source("config.schema.json");
    let listed: Vec<&str> = schema
        .pointer("/$defs/RuleLevel/enum")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let declared: Vec<&str> = Level::ALL.iter().map(|level| level.as_str()).collect();
    assert_eq!(listed, declared);
}
