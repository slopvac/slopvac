//! The `schemas/v3/conformance/pack-manifest` corpus: every `valid/` case parses and every
//! `invalid/` case fails at the JSON Pointer that `cases.toml` names.

use std::fs;
use std::path::PathBuf;

use slopvac_packs::Manifest;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3/conformance/pack-manifest")
}

fn files(dir: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(corpus().join(dir))
        .expect("corpus directory")
        .map(|e| e.expect("entry").path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "toml")
                && p.file_name() != Some("cases.toml".as_ref())
        })
        .collect();
    found.sort();
    found
}

#[test]
fn every_valid_case_parses() {
    let cases = files("valid");
    assert_ne!(cases.len(), 0);
    for path in cases {
        let text = fs::read_to_string(&path).expect("case");
        if let Err(e) = Manifest::parse(&text) {
            panic!("{} should parse: {e}", path.display());
        }
    }
}

#[test]
fn every_invalid_case_fails_at_its_pointer() {
    let expected: toml::Table =
        toml::from_str(&fs::read_to_string(corpus().join("invalid/cases.toml")).expect("cases"))
            .expect("cases.toml");
    let cases = files("invalid");
    assert_ne!(cases.len(), 0, "invalid/ holds no cases");
    assert_ne!(expected.len(), 0, "cases.toml holds no cases");
    assert_eq!(
        cases.len(),
        expected.len(),
        "cases.toml and invalid/ differ"
    );
    for path in cases {
        let name = path
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        let want = expected
            .get(&name)
            .and_then(toml::Value::as_str)
            .unwrap_or_else(|| panic!("{name} is missing from cases.toml"));
        let text = fs::read_to_string(&path).expect("case");
        match Manifest::parse(&text) {
            Ok(_) => panic!("{name} should fail"),
            Err(e) => assert_eq!(e.pointer(), Some(want), "{name}: {e}"),
        }
    }
}
