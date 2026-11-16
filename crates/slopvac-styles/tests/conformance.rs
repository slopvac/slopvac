//! The shared style conformance corpus (`schemas/v3/conformance/style/`) through the loader.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use slopvac_config::tree::parse_toml;
use slopvac_styles::{Style, StyleError};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3/conformance/style")
}

fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml") && !p.ends_with("cases.toml"))
        .collect();
    out.sort();
    out
}

fn cases(dir: &Path) -> BTreeMap<String, String> {
    let text = fs::read_to_string(dir.join("cases.toml")).unwrap();
    let json = parse_toml(&text).unwrap().to_json();
    json.as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
        .collect()
}

#[test]
fn valid_styles_load() {
    let found = files(&corpus().join("valid"));
    assert_eq!(found.len(), 5, "the valid corpus has five files");
    for path in found {
        let text = fs::read_to_string(&path).unwrap();
        if let Err(e) = Style::parse("sample", &text) {
            panic!("{} should load: {e}", path.display());
        }
    }
}

#[test]
fn invalid_styles_fail_at_the_listed_pointer() {
    let dir = corpus().join("invalid");
    let expected = cases(&dir);
    let found = files(&dir);
    assert_eq!(
        found.len(),
        expected.len(),
        "cases.toml lists every invalid file"
    );
    for path in found {
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let text = fs::read_to_string(&path).unwrap();
        let Err(StyleError::Invalid { problems, .. }) = Style::parse("sample", &text) else {
            panic!("{name} should be rejected as invalid");
        };
        let pointer = &expected[&name];
        assert!(
            problems
                .iter()
                .any(|(_, p)| p.starts_with(pointer.as_str())),
            "{name}: expected a problem at `{pointer}`, got {problems:?}"
        );
    }
}

#[test]
fn the_full_valid_style_keeps_every_field() {
    let text = fs::read_to_string(corpus().join("valid/full.toml")).unwrap();
    let style = Style::parse("sample", &text).unwrap();
    assert_eq!(style.extends, ["core:plain"]);
    assert_eq!(style.conflicts, ["marketing"]);
    assert_eq!(style.terms.allow, Some(vec!["slopvac".to_owned()]));
    assert_eq!(
        style.terms.blocklists,
        Some(vec!["blocklist.toml".to_owned()])
    );
    assert_eq!(style.rules.len(), 1);
    assert_eq!(style.categories.len(), 2);
}
