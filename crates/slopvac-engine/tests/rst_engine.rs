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

use common::{engine, places, rule, tokens};
use serde_json::json;
use slopvac_engine::rst::RstOptions;
use slopvac_engine::{Engine, Input, Kind};
use slopvac_model::NoticeLevel;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/rst");

/// The fake interpreter, written once before any test starts a process.
fn interpreter() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let dir = common::private_dir("rst-engine-fakes");
        let path = dir.join("fake-python");
        let write = |path: &Path, body: String| {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        };
        let json =
            |name: &str| format!("#!/bin/sh\ncat >/dev/null\ncat '{FIXTURES}/{name}.json'\n");
        write(&path, json("sections"));
        write(&dir.join("tables-python"), json("table-repeat"));
        write(&dir.join("wrap-python"), json("table-wrap"));
        write(
            &dir.join("planted/python3"),
            "#!/bin/sh\ntouch \"$(dirname \"$0\")/ran\"\nexit 0\n".to_owned(),
        );
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

fn tables_engine(rules: &[serde_json::Value]) -> Engine {
    let mut options = RstOptions::default();
    options.interpreter = Some(interpreter().with_file_name("tables-python"));
    engine(rules).with_rst(options)
}

fn tables() -> String {
    fs::read_to_string(format!("{FIXTURES}/table-repeat.rst")).unwrap()
}

#[test]
fn a_rule_that_excludes_tables_is_silent_in_an_rst_table() {
    let open = tables_engine(&[tokens("dull", "Alpha", &json!({}))]);
    let doc = open.lint(&Input::rst("t.rst", &tables())).unwrap();
    assert_eq!(doc.findings.len(), 1);
    let closed = tables_engine(&[tokens(
        "dull",
        "Alpha",
        &json!({"exclude_blocks": ["table"]}),
    )]);
    let doc = closed.lint(&Input::rst("t.rst", &tables())).unwrap();
    assert_eq!(doc.findings.len(), 0);
}

#[test]
fn an_engine_without_project_boundaries_runs_nothing_from_the_path() {
    let planted = interpreter().with_file_name("planted");
    let marker = planted.join("ran");
    let _ = fs::remove_file(&marker);
    let mut options = RstOptions::default();
    options.search_path = Some(planted.clone().into_os_string());
    let e = engine(&[tokens("dull", "installer", &json!({}))]).with_rst(options);
    let doc = e.lint(&Input::rst("a.rst", &sections())).unwrap();
    assert!(
        !marker.exists(),
        "the interpreter ran without project boundaries"
    );
    assert!(
        doc.unchecked
            .iter()
            .any(|n| n.code.as_str() == "rst-converter-missing")
    );
}

#[test]
fn project_roots_given_to_the_engine_reach_the_search() {
    let planted = interpreter().with_file_name("planted");
    let mut options = RstOptions::default();
    options.search_path = Some(planted.clone().into_os_string());
    let e = engine(&[tokens("dull", "installer", &json!({}))])
        .with_rst(options)
        .with_project_roots(vec![planted.join("..")]);
    let doc = e.lint(&Input::rst("a.rst", &sections())).unwrap();
    // The planted directory is inside the project root, so it is passed over.
    assert!(
        doc.unchecked
            .iter()
            .any(|n| n.code.as_str() == "rst-converter-missing")
    );
    assert!(!planted.join("ran").exists());
}

#[test]
fn a_wrapped_left_cell_does_not_hide_the_right_cell_from_document_rules() {
    let mut options = RstOptions::default();
    options.interpreter = Some(interpreter().with_file_name("wrap-python"));
    let e = engine(&[tokens("dull", "Rightonly", &json!({"scope": "document"}))]).with_rst(options);
    let source = fs::read_to_string(format!("{FIXTURES}/table-wrap.rst")).unwrap();
    let doc = e.lint(&Input::rst("t.rst", &source)).unwrap();
    // "Rightonly" is on line 2 of the table, after "| Left wrap | ".
    assert_eq!(places(&doc), [("core:cat.dull".to_owned(), 2, 15)]);
}

fn wrap_engine(pattern: &str) -> Engine {
    let mut options = RstOptions::default();
    options.interpreter = Some(interpreter().with_file_name("wrap-python"));
    engine(&[rule(
        "pat",
        &json!({"kind": "pattern", "pattern": pattern}),
        &json!({"scope": "document"}),
    )])
    .with_rst(options)
}

fn wrap_source() -> String {
    fs::read_to_string(format!("{FIXTURES}/table-wrap.rst")).unwrap()
}

#[test]
fn a_document_start_anchor_matches_only_the_first_cell_of_an_interleaved_table() {
    let source = wrap_source();
    let first = wrap_engine("^Left")
        .lint(&Input::rst("t.rst", &source))
        .unwrap();
    assert_eq!(first.findings.len(), 1);
    // "Rightonly" is not at the start of the document, whatever its position in the file.
    let right = wrap_engine("^Rightonly")
        .lint(&Input::rst("t.rst", &source))
        .unwrap();
    assert_eq!(right.findings.len(), 0);
}

#[test]
fn a_match_across_interleaved_cells_maps_to_the_hull_of_its_tokens() {
    let source = wrap_source();
    let doc = wrap_engine(r"here Rightonly")
        .lint(&Input::rst("t.rst", &source))
        .unwrap();
    assert_eq!(doc.findings.len(), 1);
    let span = doc.findings[0].extent.unwrap().span;
    // "Rightonly" comes first in the file, "here" ends the match in reading order but sits on
    // the last line: the span runs from the smallest start to the largest end.
    let start = source.find("Rightonly").unwrap();
    let end = source.find("here").unwrap() + "here".len();
    assert_eq!((span.start(), span.end()), (start, end));
}

#[test]
fn the_rst_kind_is_public() {
    assert_eq!(Input::rst("a.rst", "x").kind, Kind::Rst);
}
