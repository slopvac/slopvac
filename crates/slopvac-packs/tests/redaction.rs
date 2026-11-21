//! Errors never repeat text from the manifest. A value that is refused can carry a credential
//! (a homepage with URL user-info) or hostile text, so a diagnostic holds the pointer and a
//! fixed reason only (docs/contract/policies.md section 1.7).
#![allow(clippy::panic, reason = "test code: a panic is a failed test")]

use std::error::Error;

use rstest::rstest;
use slopvac_packs::Manifest;

const SENTINEL: &str = "SENTINEL-t0ken";

const BASE: &str =
    "schema_version = 3\nid = \"tiny\"\nversion = \"0.1.0\"\nslopvac_api = 1\nkind = \"data\"\n";

const PLUGIN_BASE: &str =
    "schema_version = 3\nid = \"tiny\"\nversion = \"0.1.0\"\nslopvac_api = 1\nkind = \"code\"\n";

/// Every text that holds the sentinel somewhere a validator looks at it.
fn hostile(case: &str) -> String {
    let s = SENTINEL;
    match case {
        "id" => format!(
            "schema_version = 3\nid = \"{s}\"\nversion = \"0.1.0\"\nslopvac_api = 1\nkind = \"data\"\n"
        ),
        "version" => format!(
            "schema_version = 3\nid = \"tiny\"\nversion = \"{s}\"\nslopvac_api = 1\nkind = \"data\"\n"
        ),
        "kind" => format!(
            "schema_version = 3\nid = \"tiny\"\nversion = \"0.1.0\"\nslopvac_api = 1\nkind = \"{s}\"\n"
        ),
        "unknown-key" => format!("{BASE}{s} = 1\n"),
        "homepage-credentials" => {
            format!("{BASE}homepage = \"https://user:{s}@host/ not a uri\"\n")
        }
        "homepage-valid-credentials" => {
            format!("{BASE}homepage = \"https://user:{s}@host/path\"\n")
        }
        "provides-path" => format!("{BASE}[provides]\nstyles = [\"../{s}\"]\n"),
        "requires-id" => format!("{BASE}[[requires]]\nid = \"{s}\"\nversion = \"1\"\n"),
        "requires-version" => format!("{BASE}[[requires]]\nid = \"core\"\nversion = \"{s}\"\n"),
        "env" => format!(
            "{PLUGIN_BASE}[plugin]\ncommand = \"x\"\nnetwork = false\nhooks = []\nenv = [\"{s}\"]\nfeatures = {{ required = [], optional = [] }}\n"
        ),
        "hook-kind" => format!(
            "{PLUGIN_BASE}[plugin]\ncommand = \"x\"\nnetwork = false\nhooks = [{{ kind = \"{s}\", id = \"a\" }}]\nfeatures = {{ required = [], optional = [] }}\n"
        ),
        "hook-id" => format!(
            "{PLUGIN_BASE}[plugin]\ncommand = \"x\"\nnetwork = false\nhooks = [{{ kind = \"tagger\", id = \"{s}\" }}]\nfeatures = {{ required = [], optional = [] }}\n"
        ),
        "feature" => format!(
            "{PLUGIN_BASE}[plugin]\ncommand = \"x\"\nnetwork = false\nhooks = []\nfeatures = {{ required = [\"{s}\"], optional = [] }}\n"
        ),
        "syntax" => format!("{BASE}key = \"{s}\" oops\n"),
        "syntax-key" => format!("{BASE}{s} {s}\n"),
        "wrong-type" => format!("{BASE}description = {{ {s} = 1 }}\n"),
        other => panic!("no case {other}"),
    }
}

fn chain_text(error: &(dyn Error + 'static)) -> Vec<String> {
    let mut texts = Vec::new();
    let mut current: Option<&(dyn Error + 'static)> = Some(error);
    while let Some(e) = current {
        texts.push(e.to_string());
        texts.push(format!("{e:?}"));
        current = e.source();
    }
    texts
}

#[rstest]
#[case::id("id")]
#[case::version("version")]
#[case::kind("kind")]
#[case::unknown_key("unknown-key")]
#[case::homepage_credentials("homepage-credentials")]
#[case::homepage_valid_credentials("homepage-valid-credentials")]
#[case::provides_path("provides-path")]
#[case::requires_id("requires-id")]
#[case::requires_version("requires-version")]
#[case::env("env")]
#[case::hook_kind("hook-kind")]
#[case::hook_id("hook-id")]
#[case::feature("feature")]
#[case::syntax("syntax")]
#[case::syntax_key("syntax-key")]
#[case::wrong_type("wrong-type")]
fn an_error_never_repeats_the_manifest_text(#[case] name: &str) {
    let error = Manifest::parse(&hostile(name)).expect_err("the manifest is invalid");
    for text in chain_text(&error) {
        assert!(!text.contains(SENTINEL), "{name}: {text}");
    }
}

#[test]
fn a_reader_error_never_repeats_the_manifest_text() {
    let error = Manifest::from_reader(hostile("syntax").as_bytes()).expect_err("invalid");
    for text in chain_text(&error) {
        assert!(!text.contains(SENTINEL), "{text}");
    }
}
