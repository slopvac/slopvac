//! The `schema_versions` object of `slopvac --version --json` comes from one table in the
//! binary, `slopvac_cli::versions::supported`. This test ties that table to the schemas under
//! `schemas/v3` both ways: each key names a schema whose root requires `schema_version`, and
//! each such schema is listed unless it is named here as not yet listed. A schema counts
//! whether it is generated or hand-written.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::Value;

/// Schemas with a root `schema_version` that `docs/contract/policies.md`, section 3.2, does not
/// list yet: "the list grows when the binary starts to read or write another versioned
/// artifact, such as a plugin manifest, a plugin index or a standalone rule file".
const NOT_LISTED_YET: &[&str] = &["plugin-index", "rule"];

const SUFFIX: &str = ".schema.json";

/// The names, without the suffix, of the schemas under `schemas/v3` whose root requires
/// `schema_version`.
fn versioned_schemas() -> BTreeSet<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3");
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let Some(name) = path
            .file_name()
            .and_then(|f| f.to_str())
            .and_then(|f| f.strip_suffix(SUFFIX))
        else {
            continue;
        };
        let schema: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let required = schema
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|names| names.iter().any(|n| n == "schema_version"));
        if required {
            names.insert(name.to_owned());
        }
    }
    names
}

#[test]
fn every_listed_key_is_a_schema_that_requires_schema_version() {
    let versioned = versioned_schemas();
    let supported = slopvac_cli::versions::supported();
    let wrong: Vec<_> = supported
        .schema_versions
        .keys()
        .filter(|name| !versioned.contains(**name))
        .collect();
    assert!(
        wrong.is_empty(),
        "these keys are not schemas under schemas/v3 that require schema_version: {wrong:?}"
    );
}

#[test]
fn every_schema_that_requires_schema_version_is_listed() {
    let supported = slopvac_cli::versions::supported();
    let unlisted: Vec<_> = versioned_schemas()
        .into_iter()
        .filter(|name| {
            !supported.schema_versions.contains_key(name.as_str())
                && !NOT_LISTED_YET.contains(&name.as_str())
        })
        .collect();
    assert!(
        unlisted.is_empty(),
        "these schemas require schema_version but are not in slopvac_cli::versions::supported: {unlisted:?}"
    );
}

#[test]
fn the_exemptions_are_versioned_schemas_that_are_still_unlisted() {
    let versioned = versioned_schemas();
    let supported = slopvac_cli::versions::supported();
    for name in NOT_LISTED_YET {
        assert!(
            versioned.contains(*name),
            "{name} is not a schema that requires schema_version: drop it from NOT_LISTED_YET"
        );
        assert!(
            !supported.schema_versions.keys().any(|key| key == name),
            "{name} is listed now: drop it from NOT_LISTED_YET"
        );
    }
}
