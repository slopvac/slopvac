//! A pack error never repeats text from a pack file. File text can hold a credential or hostile
//! text, so an error carries a location and a fixed reason only
//! (docs/contract/policies.md section 1.7).
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use std::error::Error;
use std::fs;
use std::path::Path;

use rstest::rstest;
use slopvac_packs::{DirSource, LoadedPack, MANIFEST_FILE, PackError, Source};

const SENTINEL: &str = "SENTINEL-t0ken";

fn pack(provides: &str, files: &[(&str, String)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let manifest = format!(
        "schema_version = 3\nid = \"acme\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"data\"\n\n[provides]\n{provides}\n"
    );
    fs::write(dir.path().join(MANIFEST_FILE), manifest).unwrap();
    for (name, text) in files {
        let target = dir.path().join(name);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, text).unwrap();
    }
    dir
}

fn load(dir: &Path) -> PackError {
    LoadedPack::load(&DirSource::new(dir), Source::Pack).expect_err("the pack is invalid")
}

fn every_text(error: &(dyn Error + 'static)) -> Vec<String> {
    let mut texts = Vec::new();
    let mut current: Option<&(dyn Error + 'static)> = Some(error);
    while let Some(e) = current {
        texts.push(e.to_string());
        texts.push(format!("{e:?}"));
        current = e.source();
    }
    texts
}

fn category(rules: &str) -> String {
    format!(
        "{{\"schema_version\": 3, \"id\": \"cat\", \"title\": \"t\", \"description\": \"d\", \"rules\": [{rules}]}}"
    )
}

fn tokens_rule(id: &str, examples: &str) -> String {
    format!(
        "{{\"id\": \"{id}\", \"name\": \"n\", \"message\": \"m\", \"match\": {{\"kind\": \"tokens\", \"tokens\": [\"leverage\"]}}, \"examples\": {examples}, \"citation\": {{\"source\": \"s\"}}}}"
    )
}

#[rstest]
#[case::syntax(format!("{{\"id\": \"{SENTINEL}\" oops"))]
#[case::unknown_field(category(&format!("{{\"{SENTINEL}\": 1}}")))]
#[case::bad_example_does_not_match(category(&tokens_rule(
    "dull",
    &format!("[{{\"bad\": \"{SENTINEL}\", \"good\": \"Use it.\"}}]")
)))]
#[case::good_example_matches(category(&tokens_rule(
    "dull",
    &format!("[{{\"bad\": \"We leverage it.\", \"good\": \"We leverage {SENTINEL}.\"}}]")
)))]
#[case::duplicate_rule(category(&format!(
    "{},{}",
    tokens_rule(SENTINEL, "[{\"bad\": \"We leverage it.\", \"good\": \"Use it.\"}]"),
    tokens_rule(SENTINEL, "[{\"bad\": \"We leverage it.\", \"good\": \"Use it.\"}]")
)))]
fn a_rules_error_never_repeats_the_category_text(#[case] text: String) {
    let dir = pack("categories = [\"c.json\"]", &[("c.json", text)]);

    let error = load(dir.path());

    assert!(matches!(error, PackError::Rules { .. }), "{error}");
    for text in every_text(&error) {
        assert!(!text.contains(SENTINEL), "{text}");
    }
}

#[rstest]
#[case::syntax(format!("id: [{SENTINEL}\n"))]
#[case::wrong_type(format!("schema_version: 3\nid: g\nprofile: {SENTINEL}\n"))]
#[case::wrong_shape(format!("schema_version: 3\nid: g\nextends: {SENTINEL}\n"))]
fn an_artifact_error_never_repeats_the_genre_text(#[case] text: String) {
    let dir = pack("genres = [\"g.yaml\"]", &[("g.yaml", text)]);

    let error = load(dir.path());

    assert!(matches!(error, PackError::Artifact { .. }), "{error}");
    for text in every_text(&error) {
        assert!(!text.contains(SENTINEL), "{text}");
    }
}

#[test]
fn a_rules_error_names_the_file_and_line() {
    let dir = pack(
        "categories = [\"c.json\"]",
        &[("c.json", "{\n\"id\": 1 oops\n}".to_owned())],
    );

    let shown = load(dir.path()).to_string();

    assert!(shown.contains("c.json"), "{shown}");
    assert!(shown.contains(":2"), "{shown}");
}

#[test]
fn an_artifact_syntax_error_names_the_file_and_line() {
    let dir = pack(
        "genres = [\"g.yaml\"]",
        &[("g.yaml", "schema_version: 3\nid: [oops\n".to_owned())],
    );

    let shown = load(dir.path()).to_string();

    assert!(shown.contains("g.yaml"), "{shown}");
    assert!(shown.contains("line"), "{shown}");
}
