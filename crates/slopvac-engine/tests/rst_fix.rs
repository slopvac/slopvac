//! `--fix` on RST files: only prose words change.
//!
//! The same document runs twice: through a fake interpreter that prints the driver output
//! real docutils wrote for it (always, on Unix), and through the docutils on `PATH` (when
//! there is one; CI sets `SLOPVAC_RST_LIVE`). Both must give the same fixed text.
#![allow(
    missing_docs,
    clippy::assert_is_empty,
    clippy::cloned_ref_to_slice_refs
)]

mod common;

use std::path::PathBuf;

use common::{engine, rule};
use serde_json::json;
use slopvac_engine::rst::RstOptions;
use slopvac_engine::{Engine, Input, SkipReason, fix_document};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/rst");

fn source() -> String {
    std::fs::read_to_string(format!("{FIXTURES}/fix.rst.in")).unwrap()
}

/// What the fixed document must be, worked out by hand from the source: each `find`
/// occurs once, and every other `delta` stays.
const CHANGES: &[(&str, &str)] = &[
    ("Fix delta\n", "Fix d\n"),
    (":delta: delta field", ":delta: d field"),
    ("Plain delta prose", "Plain d prose"),
    ("`delta text <https", "`d text <https"),
    ("Note delta text.", "Note d text."),
    ("Literal delta::", "Literal d::"),
    ("for delta.", "for d."),
    ("*delta star*", "*d star*"),
    ("**delta strong**", "**d strong**"),
];

fn expected() -> String {
    let mut text = source();
    for (find, replace) in CHANGES {
        assert_eq!(text.matches(find).count(), 1, "{find:?}");
        text = text.replacen(find, replace, 1);
    }
    text
}

fn swap(options: RstOptions) -> Engine {
    engine(&[rule(
        "swap",
        &json!({"kind": "substitution", "substitutions": {"delta": "d"}}),
        &json!({}),
    )])
    .with_rst(options)
}

fn fixed(engine: &Engine, text: &str) -> slopvac_engine::FixReport {
    let input = Input::rst("doc.rst", text);
    let doc = engine.lint(&input).unwrap();
    fix_document(engine, &input, &doc, None).unwrap()
}

fn check(engine: &Engine) {
    let report = fixed(engine, &source());
    assert_eq!(report.text, expected());
    assert_eq!(report.applied, CHANGES.len());
    // The others are findings the fix refused.
    assert!(
        report
            .skipped
            .iter()
            .all(|s| s.reason == SkipReason::Protected)
    );
    assert!(!report.skipped.is_empty());
    // A second run has nothing left to change in the prose.
    let again = fixed(engine, &report.text);
    assert_eq!(again.text, report.text);
    assert_eq!(again.applied, 0);
}

#[cfg(unix)]
#[test]
fn rst_prose_is_fixed_and_everything_else_stays_with_the_fake_driver() {
    use std::os::unix::fs::PermissionsExt;
    let dir = common::private_dir("rst-fix-fake");
    let path = dir.join("fake-python");
    std::fs::write(
        &path,
        format!("#!/bin/sh\ncat >/dev/null\ncat '{FIXTURES}/fix.json'\n"),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut options = RstOptions::default();
    options.interpreter = Some(path);
    check(&swap(options));
}

#[test]
fn rst_prose_is_fixed_and_everything_else_stays_with_docutils() {
    let mut options = RstOptions::default();
    options.project_roots = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR"))];
    let e = swap(options);
    if let Err(error) = slopvac_engine::rst::RstAdapter::new({
        let mut o = RstOptions::default();
        o.project_roots = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR"))];
        o
    })
    .interpreter()
    {
        assert!(
            std::env::var_os("SLOPVAC_RST_LIVE").is_none(),
            "SLOPVAC_RST_LIVE is set but there is no usable docutils: {error}"
        );
        return;
    }
    check(&e);
}

/// A fake interpreter that prints the driver output saved in `fixture`.
#[cfg(unix)]
fn fake(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
fn engine_for(path: PathBuf, words: &serde_json::Value) -> Engine {
    let mut options = RstOptions::default();
    options.interpreter = Some(path);
    engine(&[rule(
        "swap",
        &json!({"kind": "substitution", "substitutions": words}),
        &json!({}),
    )])
    .with_rst(options)
}

#[cfg(unix)]
#[test]
fn words_in_two_cells_of_a_wrapped_row_are_both_fixed_but_a_phrase_across_the_wrap_is_not() {
    let dir = common::private_dir("rst-fix-table");
    let python = fake(
        &dir,
        "python",
        &format!("cat >/dev/null\ncat '{FIXTURES}/table-wrap.json'"),
    );
    let source = std::fs::read_to_string(format!("{FIXTURES}/table-wrap.rst.in")).unwrap();
    let e = engine_for(
        python.clone(),
        &json!({"continues": "goes", "Rightonly": "solo"}),
    );
    let report = fixed(&e, &source);
    assert_eq!(
        report.text,
        source
            .replacen("continues", "goes", 1)
            .replacen("Rightonly", "Solo", 1)
    );
    assert_eq!(report.applied, 2);
    // "wrap continues" runs over the line break, which also crosses the cell beside it, so
    // the text must not change whether the finding is dropped or refused.
    let e = engine_for(python, &json!({"wrap continues": "x"}));
    let report = fixed(&e, &source);
    assert_eq!(report.text, source);
    assert_eq!(report.applied, 0);
}

#[cfg(unix)]
#[test]
fn a_converter_that_disappears_after_the_lint_fails_the_fix_and_writes_nothing() {
    use slopvac_engine::{FixError, Kind, fix_file};
    let dir = common::private_dir("rst-fix-gone");
    let marker = dir.join("ran");
    let python = fake(
        &dir,
        "python",
        &format!(
            "cat >/dev/null\nif [ -e '{m}' ]; then exit 3; fi\ntouch '{m}'\ncat '{FIXTURES}/fix.json'",
            m = marker.display()
        ),
    );
    let e = engine_for(python.clone(), &json!({"delta": "d"}));
    let source = source();
    let input = Input::rst("doc.rst", &source);
    let doc = e.lint(&input).unwrap();
    // The first run found the interpreter; the second does not.
    let error = fix_document(&e, &input, &doc, None).unwrap_err();
    assert!(matches!(error, FixError::Rst(_)), "{error}");

    let root = common::private_dir("rst-fix-gone-root");
    std::fs::write(root.join("doc.rst"), &source).unwrap();
    std::fs::remove_file(&marker).unwrap();
    let result = fix_file(
        &e,
        &root,
        std::path::Path::new("doc.rst"),
        Kind::Rst,
        false,
        None,
    );
    assert!(matches!(result, Err(FixError::Rst(_))), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(root.join("doc.rst")).unwrap(),
        source
    );
}
