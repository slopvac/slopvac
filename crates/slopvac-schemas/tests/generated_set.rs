//! Pins which `schemas/v3/*.schema.json` files are generated. A file is either in `ARTIFACTS`
//! or on `NOT_GENERATED` with the bead that will generate it, so dropping an `ARTIFACTS` entry
//! cannot quietly turn a generated schema back into a hand-written one.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use slopvac_schemas::ARTIFACTS;

/// Schema files that are still hand-written, each with the bead that will generate it.
/// A file that is generated now must leave this list, so the list only shrinks.
const NOT_GENERATED: &[(&str, &str)] = &[
    ("blocklist.schema.json", "slopvac-8l4.8.14"),
    ("config.schema.json", "slopvac-8l4.4.18"),
    ("frontmatter.schema.json", "slopvac-8l4.4.18"),
    ("genre.schema.json", "slopvac-8l4.5.57"),
    ("plugin-protocol.schema.json", "slopvac-8l4.2.95"),
    ("rules-output.schema.json", "slopvac-8l4.12.88"),
    ("style.schema.json", "slopvac-8l4.8.14"),
    ("terms.schema.json", "slopvac-8l4.8.14"),
];

fn on_disk() -> BTreeSet<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3");
    fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".schema.json"))
        .collect()
}

#[test]
fn every_schema_file_is_generated_or_owned() {
    let disk = on_disk();
    let generated: BTreeSet<&str> = ARTIFACTS.iter().map(|a| a.file).collect();
    let mut problems = Vec::new();

    for file in &disk {
        let listed = NOT_GENERATED.iter().any(|(name, _)| name == file);
        if !generated.contains(file.as_str()) && !listed {
            problems.push(format!(
                "{file} is neither in ARTIFACTS nor in NOT_GENERATED: generate it, or list it \
                 in NOT_GENERATED with the bead that will"
            ));
        }
    }
    for (file, bead) in NOT_GENERATED {
        if generated.contains(file) {
            problems.push(format!(
                "{file} is generated now: drop it from NOT_GENERATED (it named {bead})"
            ));
        }
        if !disk.contains::<str>(file) {
            problems.push(format!(
                "{file} is in NOT_GENERATED but not on disk: remove the entry"
            ));
        }
    }
    for file in &generated {
        if !disk.contains::<str>(file) {
            problems.push(format!(
                "{file} is in ARTIFACTS but not on disk: write the schemas again"
            ));
        }
    }

    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}
