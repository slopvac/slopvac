//! The shared conformance corpus (`schemas/v3/conformance/`) run through the validator.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use slopvac_config::tree::{parse_toml, parse_yaml};
use slopvac_config::{Diagnostic, validate_config, validate_frontmatter};

fn corpus(kind: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/v3/conformance")
        .join(kind)
}

#[expect(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "toml" || x == "yaml") && !p.ends_with("cases.toml")
        })
        .collect();
    out.sort();
    out
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn cases(dir: &Path) -> BTreeMap<String, String> {
    let text = fs::read_to_string(dir.join("cases.toml")).unwrap();
    let root = parse_toml(&text).unwrap();
    let json = root.to_json();
    json.as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
        .collect()
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn run(kind: &str, path: &Path) -> Vec<Diagnostic> {
    let text = fs::read_to_string(path).unwrap();
    match kind {
        "config" => validate_config(&parse_toml(&text).unwrap(), &text),
        _ => validate_frontmatter(&parse_yaml(&text).unwrap(), &text),
    }
}

fn check_valid(kind: &str) {
    for path in files(&corpus(kind).join("valid")) {
        let diags = run(kind, &path);
        assert!(
            diags.is_empty(),
            "{} should be valid, got {diags:#?}",
            path.display()
        );
    }
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn check_invalid(kind: &str) {
    let dir = corpus(kind).join("invalid");
    let expected = cases(&dir);
    let found = files(&dir);
    assert_eq!(
        found.len(),
        expected.len(),
        "cases.toml lists every invalid file"
    );
    for path in found {
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let pointer = &expected[&name];
        let diags = run(kind, &path);
        assert!(
            diags
                .iter()
                .any(|d| d.pointer().starts_with(pointer.as_str())),
            "{name}: expected a diagnostic at `{pointer}`, got {:?}",
            diags.iter().map(Diagnostic::pointer).collect::<Vec<_>>()
        );
    }
}

#[test]
fn valid_configs_pass() {
    check_valid("config");
}

#[test]
fn invalid_configs_fail_at_the_listed_pointer() {
    check_invalid("config");
}

#[test]
fn valid_front_matter_passes() {
    check_valid("frontmatter");
}

#[test]
fn invalid_front_matter_fails_at_the_listed_pointer() {
    check_invalid("frontmatter");
}
