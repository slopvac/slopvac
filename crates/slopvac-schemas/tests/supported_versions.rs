//! The `schema_versions` object of `slopvac --version --json` comes from one table in the
//! binary, `slopvac_cli::versions::supported`. This test ties that table to the artifacts under
//! `schemas/v3`: each key names a schema file, and a generated artifact that the binary writes
//! at a `schema_version` is listed unless it is named here as not yet listed.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;
use slopvac_schemas::ARTIFACTS;

/// Generated artifacts with a root `schema_version` that `docs/contract/policies.md`, section
/// 3.2, does not list yet: "the list grows when the binary starts to read or write another
/// versioned artifact, such as a plugin manifest, a plugin index or a standalone rule file".
const NOT_LISTED_YET: &[&str] = &["plugin-index", "plugin-manifest", "rule"];

fn stem(file: &str) -> &str {
    file.strip_suffix(".schema.json").unwrap_or(file)
}

fn on_disk(file: &str) -> bool {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/v3")
        .join(file)
        .is_file()
}

#[test]
fn every_listed_artifact_is_a_schema_file() {
    let supported = slopvac_cli::versions::supported();
    let missing: Vec<_> = supported
        .schema_versions
        .keys()
        .filter(|name| !on_disk(&format!("{name}.schema.json")))
        .collect();
    assert!(missing.is_empty(), "no schema file for {missing:?}");
}

#[test]
fn every_generated_artifact_that_requires_schema_version_is_listed() {
    let supported = slopvac_cli::versions::supported();
    let listed: BTreeSet<&str> = supported.schema_versions.keys().copied().collect();
    let mut unlisted = Vec::new();
    for artifact in ARTIFACTS {
        let schema = (artifact.build)();
        let required = schema
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|names| names.iter().any(|n| n == "schema_version"));
        let name = stem(artifact.file);
        if required && !listed.contains(name) && !NOT_LISTED_YET.contains(&name) {
            unlisted.push(name);
        }
    }
    assert!(
        unlisted.is_empty(),
        "these artifacts require schema_version but are not in slopvac_cli::versions::supported: {unlisted:?}"
    );
}

#[test]
fn the_exemptions_are_generated_artifacts_that_are_still_unlisted() {
    let supported = slopvac_cli::versions::supported();
    for name in NOT_LISTED_YET {
        assert!(
            ARTIFACTS.iter().any(|a| stem(a.file) == *name),
            "{name} is not a generated artifact: drop it from NOT_LISTED_YET"
        );
        assert!(
            !supported.schema_versions.keys().any(|key| key == name),
            "{name} is listed now: drop it from NOT_LISTED_YET"
        );
    }
}
