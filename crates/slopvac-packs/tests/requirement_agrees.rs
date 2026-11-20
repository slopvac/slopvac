//! The parser and the schema agree on which version requirements are valid. The schema's
//! `VersionReq` pattern is read from `schemas/v3/plugin-manifest.schema.json`, not copied here.

use std::fs;
use std::path::PathBuf;

use regex::Regex;
use slopvac_packs::Manifest;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The schema pattern as the `regex` crate can run it. The schema ends the pattern with the
/// ECMAScript anchor `(?![\s\S])`, a lookahead that `regex` lacks; `\z` means the same.
fn schema_pattern() -> Regex {
    let schema: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(repo().join("schemas/v3/plugin-manifest.schema.json")).unwrap(),
    )
    .unwrap();
    let pattern = schema["$defs"]["VersionReq"]["pattern"]
        .as_str()
        .expect("VersionReq pattern");
    Regex::new(&pattern.replace(r"(?![\s\S])", r"\z")).expect("the pattern compiles")
}

/// Every `[[requires]]` version in the plugin-manifest corpus.
fn corpus_requirements() -> Vec<String> {
    let mut found = Vec::new();
    for dir in ["valid", "invalid"] {
        let dir = repo()
            .join("schemas/v3/conformance/plugin-manifest")
            .join(dir);
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.file_name().is_some_and(|n| n == "cases.toml") {
                continue;
            }
            let table: toml::Table = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
            let requires = table.get("requires").and_then(toml::Value::as_array);
            for item in requires.into_iter().flatten() {
                if let Some(v) = item.get("version").and_then(toml::Value::as_str) {
                    found.push(v.to_owned());
                }
            }
        }
    }
    found
}

/// Forms that mark the edges of the grammar, in addition to the corpus.
const PROBES: &[&str] = &[
    "1",
    "1.0",
    "1.0.0",
    "^1.2",
    ">=3.0, <4",
    "^1.0.0-rc.1",
    "1.0.0-rc",
    "1.0-rc",
    "1-rc",
    "1\n",
    " 1",
    "1 ",
    ">= 1.2",
    "^1 , <2",
    ">=1.0.0-beta, <2",
    "1.0\n",
    "01",
    "1.0.0-rc..1",
    "1.0.0-rc.0",
    "1.00.0",
    "",
    "*",
    "1.2.3.4",
    ">=1,",
    "latest",
];

fn parser_accepts(version: &str) -> bool {
    let text = format!(
        "schema_version = 3\nid = \"tiny\"\nversion = \"0.1.0\"\nslopvac_api = 1\nkind = \"data\"\n\
         [[requires]]\nid = \"core\"\nversion = {}\n",
        toml::Value::String(version.to_owned())
    );
    Manifest::parse(&text).is_ok()
}

#[test]
fn the_parser_accepts_exactly_what_the_schema_pattern_matches() {
    let pattern = schema_pattern();
    let mut disagreements = Vec::new();
    let corpus = corpus_requirements();
    for version in corpus
        .iter()
        .map(String::as_str)
        .chain(PROBES.iter().copied())
    {
        let (schema, parser) = (pattern.is_match(version), parser_accepts(version));
        if schema != parser {
            disagreements.push(format!("{version:?}: schema {schema}, parser {parser}"));
        }
    }
    assert!(disagreements.is_empty(), "{disagreements:#?}");
}
