//! The `schemas/v3/conformance/plugin-manifest` corpus, one test per file. Every `valid/` case
//! is accepted and every `invalid/` case is refused at the JSON Pointer that `cases.toml`
//! names. Each file goes through `Manifest::from_reader`, the entry point the pack loader uses.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use rstest::rstest;
use slopvac_packs::Manifest;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3/conformance/plugin-manifest")
}

fn expected() -> toml::Table {
    toml::from_str(&fs::read_to_string(corpus().join("invalid/cases.toml")).expect("cases.toml"))
        .expect("cases.toml parses")
}

fn load(path: &Path) -> Result<Manifest, slopvac_packs::ManifestError> {
    Manifest::from_reader(File::open(path).expect("open case"))
}

#[rstest]
fn valid_case_is_accepted(
    #[files("../../schemas/v3/conformance/plugin-manifest/valid/*.toml")] path: PathBuf,
) {
    if let Err(e) = load(&path) {
        panic!("{} should load: {e}", path.display());
    }
}

#[rstest]
fn invalid_case_is_refused_at_its_pointer(
    #[files("../../schemas/v3/conformance/plugin-manifest/invalid/*.toml")]
    #[exclude("cases\\.toml$")]
    path: PathBuf,
) {
    let name = path
        .file_name()
        .expect("file name")
        .to_string_lossy()
        .into_owned();
    let want = expected()
        .get(&name)
        .and_then(toml::Value::as_str)
        .map_or_else(
            || panic!("{name} is missing from cases.toml"),
            str::to_owned,
        );
    let err = load(&path).expect_err("an invalid case must be refused");
    assert_eq!(err.pointer(), Some(want.as_str()), "{name}: {err}");
}

#[test]
fn cases_toml_names_exactly_the_invalid_files() {
    let on_disk: BTreeSet<String> = fs::read_dir(corpus().join("invalid"))
        .expect("invalid dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .filter(|n| {
            Path::new(n)
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("toml"))
                && n != "cases.toml"
        })
        .collect();
    let listed: BTreeSet<String> = expected().keys().cloned().collect();
    assert_eq!(on_disk, listed);
}
