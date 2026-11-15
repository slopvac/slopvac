//! `Engine::lint` on an RST file goes through the docutils adapter.
//!
//! The interpreter is a shell script that prints the driver output that real docutils wrote
//! for `tests/fixtures/rst/sections.rst`, so the test runs without Python.
#![cfg(unix)]
#![allow(
    missing_docs,
    clippy::assert_is_empty,
    clippy::cloned_ref_to_slice_refs
)]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use common::{engine, places, tokens};
use serde_json::json;
use slopvac_engine::rst::RstOptions;
use slopvac_engine::{Engine, Input, Kind};
use slopvac_model::NoticeLevel;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/rst");

/// The fake interpreter, written once before any test starts a process.
fn interpreter() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("rst-engine-fakes");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fake-python");
        let body = format!("#!/bin/sh\ncat >/dev/null\ncat '{FIXTURES}/sections.json'\n");
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    })
}

fn with_rst(engine: Engine, options: RstOptions) -> Engine {
    engine.with_rst(options)
}

fn sections() -> String {
    fs::read_to_string(format!("{FIXTURES}/sections.rst")).unwrap()
}

#[test]
fn an_rst_file_is_linted_in_its_own_coordinates() {
    let mut options = RstOptions::default();
    options.interpreter = Some(interpreter().to_owned());
    let e = with_rst(engine(&[tokens("dull", "installer", &json!({}))]), options);
    let doc = e
        .lint(&Input::rst("notes.rst", &sections()))
        .expect("the RST file lints");
    // "Run the installer, then restart the service." is line 10 of the file.
    assert_eq!(places(&doc), [("core:cat.dull".to_owned(), 10, 9)]);
    assert!(
        doc.unchecked
            .iter()
            .all(|n| n.level != NoticeLevel::Warning)
    );
}

#[test]
fn a_missing_interpreter_makes_the_document_incomplete() {
    let empty = tempfile::tempdir().unwrap();
    let mut options = RstOptions::default();
    options.search_path = Some(empty.path().as_os_str().to_owned());
    let e = with_rst(engine(&[tokens("dull", "installer", &json!({}))]), options);
    let doc = e.lint(&Input::rst("docs/notes.rst", &sections())).unwrap();
    assert!(doc.findings.is_empty());
    let warnings: Vec<_> = doc
        .unchecked
        .iter()
        .filter(|n| n.level == NoticeLevel::Warning)
        .collect();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].code.as_str(), "rst-converter-missing");
    assert_eq!(warnings[0].path.as_deref(), Some("docs/notes.rst"));
    assert!(
        warnings[0].message.as_str().starts_with(
            "RST target(s) skipped: rst2html or rst2html.py not found on PATH (docs/notes.rst); \
             install with `pip install docutils`."
        ),
        "{}",
        warnings[0].message
    );
    assert!(!doc.gate.is_passed());
}

#[test]
fn the_rst_kind_is_public() {
    assert_eq!(Input::rst("a.rst", "x").kind, Kind::Rst);
}
