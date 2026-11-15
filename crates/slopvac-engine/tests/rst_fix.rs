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
