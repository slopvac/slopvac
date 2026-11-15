//! `Engine::lint` on an RST file goes through the docutils adapter.
//!
//! The interpreter is a shell script that prints the driver output that real docutils wrote
//! for `tests/fixtures/rst/sections.rst.in`, so the test runs without Python.
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
        write(&dir.join("normative-python"), json("table-normative"));
        write(&dir.join("list-python"), json("table-list"));
        write(&dir.join("section-python"), json("table-section"));
        write(&dir.join("quote-python"), json("table-quote"));
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
    fs::read_to_string(format!("{FIXTURES}/sections.rst.in")).unwrap()
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
    fs::read_to_string(format!("{FIXTURES}/table-repeat.rst.in")).unwrap()
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
    let source = fs::read_to_string(format!("{FIXTURES}/table-wrap.rst.in")).unwrap();
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
    fs::read_to_string(format!("{FIXTURES}/table-wrap.rst.in")).unwrap()
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
fn a_requirement_in_a_wrapped_cell_does_not_reach_the_cell_beside_it() {
    let source = fs::read_to_string(format!("{FIXTURES}/table-normative.rst.in")).unwrap();
    let skip = json!({"contexts": {"skip": ["requirement"]}});
    let run = |token: &str| {
        let mut options = RstOptions::default();
        options.interpreter = Some(interpreter().with_file_name("normative-python"));
        engine(&[tokens("dull", token, &skip)])
            .with_rst(options)
            .lint(&Input::rst("t.rst", &source))
            .unwrap()
    };
    // The left cell states a requirement, so a rule that skips requirements is silent there.
    assert_eq!(run("continue").findings.len(), 0);
    // The right cell states none. Its text sits inside the extent of the left cell, but the
    // tag belongs to the left cell's own text, so the rule still fires.
    assert_eq!(run("Rightonly").findings.len(), 1);
}

/// Findings of `token` in the fixture `name` when the rule excludes `block`.
fn excluded_findings(name: &str, python: &str, block: &str, token: &str) -> usize {
    let source = fs::read_to_string(format!("{FIXTURES}/{name}.rst.in")).unwrap();
    let mut options = RstOptions::default();
    options.interpreter = Some(interpreter().with_file_name(python));
    engine(&[tokens("dull", token, &json!({"exclude_blocks": [block]}))])
        .with_rst(options)
        .lint(&Input::rst("t.rst", &source))
        .unwrap()
        .findings
        .len()
}

#[test]
fn a_list_item_in_a_wrapped_cell_does_not_hide_the_cell_beside_it() {
    // The item's lines enclose the right cell's text, but the item is not the right cell.
    assert_eq!(
        excluded_findings("table-list", "list-python", "list-item", "Item"),
        0
    );
    assert_eq!(
        excluded_findings("table-list", "list-python", "list-item", "Rightonly"),
        1
    );
}

#[test]
fn a_quote_in_a_wrapped_cell_does_not_hide_the_cell_beside_it() {
    assert_eq!(
        excluded_findings("table-quote", "quote-python", "quote", "Quoted"),
        0
    );
    assert_eq!(
        excluded_findings("table-quote", "quote-python", "quote", "Rightonly"),
        1
    );
}

#[test]
fn a_match_across_the_line_break_of_a_wrapped_requirement_keeps_its_context() {
    let source = fs::read_to_string(format!("{FIXTURES}/table-normative.rst.in")).unwrap();
    let mut options = RstOptions::default();
    options.interpreter = Some(interpreter().with_file_name("normative-python"));
    let e = engine(&[rule(
        "pat",
        &json!({"kind": "pattern", "pattern": "must continue"}),
        &json!({"contexts": {"only": ["requirement"]}}),
    )])
    .with_rst(options);
    let doc = e.lint(&Input::rst("t.rst", &source)).unwrap();
    // "must" ends line 2 of the left cell and "continue" starts line 3. Each word sits in
    // a different line piece of the same requirement sentence.
    assert_eq!(doc.findings.len(), 1);
    assert!(
        doc.findings[0]
            .contexts
            .iter()
            .any(|c| c.as_str() == "requirement"),
        "{:?}",
        doc.findings[0].contexts
    );
}

#[test]
fn a_document_scope_match_across_a_wrapped_cell_break_keeps_its_requirement_context() {
    let source = fs::read_to_string(format!("{FIXTURES}/table-normative.rst.in")).unwrap();
    let mut options = RstOptions::default();
    options.interpreter = Some(interpreter().with_file_name("normative-python"));
    let e = engine(&[rule(
        "pat",
        &json!({"kind": "pattern", "pattern": "must continue"}),
        &json!({"scope": "document", "contexts": {"only": ["requirement"]}}),
    )])
    .with_rst(options);
    let doc = e.lint(&Input::rst("t.rst", &source)).unwrap();
    // The document text joins the cells in reading order. The match still has one range
    // for each line of the left cell, because the line break is a bridge in that text too.
    assert_eq!(doc.findings.len(), 1);
    assert!(
        doc.findings[0]
            .contexts
            .iter()
            .any(|c| c.as_str() == "requirement"),
        "{:?}",
        doc.findings[0].contexts
    );
}

#[test]
fn a_section_context_reaches_the_continuation_lines_of_a_wrapped_cell() {
    let source = fs::read_to_string(format!("{FIXTURES}/table-section.rst.in")).unwrap();
    let run = |token: &str| {
        let mut options = RstOptions::default();
        options.interpreter = Some(interpreter().with_file_name("section-python"));
        engine(&[tokens(
            "dull",
            token,
            &json!({"contexts": {"only": ["procedure"]}}),
        )])
        .with_rst(options)
        .lint(&Input::rst("t.rst", &source))
        .unwrap()
        .findings
        .len()
    };
    // "continues" is on the last line of the table, after the right cell ends.
    assert_eq!(run("continues"), 1);
    assert_eq!(run("Rightonly"), 1);
}

#[test]
fn an_rst_fix_is_refused_as_protected_until_literals_are_known() {
    let mut options = RstOptions::default();
    options.interpreter = Some(interpreter().with_file_name("fake-python"));
    let e = engine(&[rule(
        "swap",
        &json!({"kind": "substitution", "substitutions": {"installer": "tool"}}),
        &json!({}),
    )])
    .with_rst(options);
    let source = sections();
    let input = Input::rst("notes.rst", &source);
    let doc = e.lint(&input).unwrap();
    assert_eq!(doc.findings.len(), 1);
    assert!(doc.findings[0].fix.is_some());
    let report = slopvac_engine::fix_document(&input, &doc, None).unwrap();
    assert_eq!(report.applied, 0);
    assert_eq!(report.text, source);
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(
        report.skipped[0].reason,
        slopvac_engine::SkipReason::Protected
    );
}

#[test]
fn the_rst_kind_is_public() {
    assert_eq!(Input::rst("a.rst", "x").kind, Kind::Rst);
}
