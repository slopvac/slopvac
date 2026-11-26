//! Inlining a definition, through the public generation boundary.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "an integration test crate has no public API")]

use rstest::rstest;
use serde_json::{Value, json};
use slopvac_schemas::{Artifact, Error, generate_from};

const FILE: &str = "x.schema.json";

fn artifact(inline: &'static [&'static str], build: fn() -> Value) -> Artifact {
    Artifact {
        file: FILE,
        title: "x",
        description: None,
        imports: &[],
        inline,
        keep: &[],
        build,
    }
}

fn generated(artifact: Artifact) -> Value {
    let files = generate_from(&[artifact]).unwrap();
    serde_json::from_str(&files[0].text).unwrap()
}

#[test]
fn an_inlined_def_is_written_in_place_with_the_reference_siblings_winning() {
    let schema = generated(artifact(&["Id"], || {
        json!({
            "properties": {
                "a": {"$ref": "#/$defs/Id", "description": "A."},
                "b": {"$ref": "#/$defs/Other"},
            },
            "$defs": {
                "Id": {"type": "string", "description": "Id."},
                "Other": {"items": {"$ref": "#/$defs/Id"}},
            },
        })
    }));
    assert_eq!(
        schema["properties"]["a"],
        json!({"type": "string", "description": "A."})
    );
    assert_eq!(
        schema["$defs"],
        json!({"Other": {"items": {"type": "string", "description": "Id."}}})
    );
}

#[rstest]
#[case::not_generated(
    || json!({"type": "object", "$defs": {}}),
    |err: &Error| matches!(err, Error::UnknownInline { def, .. } if def == "Id")
)]
#[case::pointer_inside(
    || json!({"$ref": "#/$defs/Id/properties/n", "$defs": {"Id": {"properties": {"n": {}}}}}),
    |err: &Error| matches!(err, Error::InlinePointer { def, .. } if def == "Id")
)]
fn inlining_fails_on(#[case] build: fn() -> Value, #[case] expected: fn(&Error) -> bool) {
    let err = generate_from(&[artifact(&["Id"], build)]).unwrap_err();
    assert!(expected(&err), "{err:?}");
}

fn with_a_spare_def() -> Value {
    json!({
        "type": "object",
        "properties": {"a": {"$ref": "#/$defs/Used"}},
        "$defs": {"Used": {"type": "string"}, "Spare": {"type": "integer"}},
    })
}

#[rstest]
#[case::dropped_by_default(&[], &["Used"])]
#[case::kept_when_named(&["Spare"], &["Spare", "Used"])]
fn a_def_that_nothing_reaches_stays_only_when_the_artifact_keeps_it(
    #[case] keep: &'static [&'static str],
    #[case] defs: &[&str],
) {
    let artifact = Artifact {
        keep,
        ..artifact(&[], with_a_spare_def)
    };
    let schema = generated(artifact);
    let mut names: Vec<&str> = schema["$defs"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    names.sort_unstable();
    assert_eq!(names, defs);
}

#[test]
fn keeping_a_def_that_the_build_does_not_generate_fails() {
    let artifact = Artifact {
        keep: &["Missing"],
        ..artifact(&[], with_a_spare_def)
    };
    let err = generate_from(&[artifact]).unwrap_err();
    assert!(
        matches!(&err, Error::UnknownKeep { def, .. } if def == "Missing"),
        "{err:?}"
    );
}

fn spare_def_with_references() -> Value {
    json!({
        "type": "object",
        "properties": {"a": {"type": "string"}},
        "$defs": {
            "Spare": {"properties": {
                "own": {"$ref": "#/$defs/Helper"},
                "shared": {"$ref": "#/$defs/Shared"},
            }},
            "Helper": {"type": "integer"},
            "Shared": {"type": "boolean"},
        },
    })
}

fn the_shared_defs() -> Value {
    json!({"$defs": {"Shared": {"type": "boolean"}}})
}

#[test]
fn a_kept_def_is_a_root_so_its_references_resolve_and_qualify() {
    let owner = Artifact {
        file: "common.schema.json",
        ..artifact(&[], the_shared_defs)
    };
    let importer = Artifact {
        imports: &["common.schema.json"],
        keep: &["Spare"],
        ..artifact(&[], spare_def_with_references)
    };
    let files = generate_from(&[owner, importer]).unwrap();
    let schema: Value = serde_json::from_str(&files[1].text).unwrap();
    assert_eq!(
        schema["$defs"],
        json!({
            "Spare": {"properties": {
                "own": {"$ref": "#/$defs/Helper"},
                "shared": {"$ref": "common.schema.json#/$defs/Shared"},
            }},
            "Helper": {"type": "integer"},
        })
    );
}
