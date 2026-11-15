//! Safe fixes: parity with the 2.x `--fix`, and the refusals 3.0 adds.

#![allow(
    missing_docs,
    clippy::assert_is_empty,
    clippy::cloned_ref_to_slice_refs
)]

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use common::parity_engine;
use common::{engine, rule};
use serde_json::Value;
use serde_json::json;
use slopvac_engine::{
    Engine, FixReport, Input, Kind, Refusal, SkipReason, fix_document, fix_file, unified_diff,
};

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[derive(serde::Deserialize)]
struct Divergence {
    #[allow(dead_code)]
    reason: String,
}

#[test]
fn fixed_text_matches_the_2x_reference() {
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string(manifest().join("tests/fixtures/fix-parity.json")).unwrap(),
    )
    .unwrap();
    let divergences: BTreeMap<String, Divergence> = serde_json::from_str(
        &std::fs::read_to_string(manifest().join("tests/fixtures/fix-divergences.json"))
            .unwrap_or_else(|_| "{}".to_owned()),
    )
    .unwrap();
    let engine = parity_engine();
    let mut problems = Vec::new();
    for entry in fixture["documents"].as_array().unwrap() {
        let name = entry["name"].as_str().unwrap();
        let text = entry["text"].as_str().unwrap();
        let input = Input::markdown(name, text);
        let doc = engine.lint(&input).unwrap();
        let report = fix_document(&input, &doc, None).unwrap();
        let want = entry["fixed"].as_str().unwrap();
        let same = report.text == want;
        match (same, divergences.contains_key(name)) {
            (false, false) => problems.push(format!(
                "{name}: differs\n{}",
                unified_diff(name, want, &report.text)
            )),
            (true, true) => problems.push(format!("{name}: agrees; remove it from the list")),
            _ => {}
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

fn swaps() -> Engine {
    engine(&[rule(
        "swap",
        &json!({"kind": "substitution", "substitutions": {
                "alpha beta": "x", "beta gamma": "y", "delta": "d", "via": "by, or through"}}),
        &json!({}),
    )])
}

fn fix(
    engine: &Engine,
    text: &str,
    lines: Option<&[std::ops::RangeInclusive<usize>]>,
) -> FixReport {
    let input = Input::markdown("doc.md", text);
    let doc = engine.lint(&input).unwrap();
    fix_document(&input, &doc, lines).unwrap()
}

#[test]
fn the_first_of_two_overlapping_fixes_wins_and_the_other_is_reported() {
    let overlapping = engine(&[
        rule(
            "first",
            &json!({"kind": "substitution", "substitutions": {"alpha beta": "x"}}),
            &json!({}),
        ),
        rule(
            "second",
            &json!({"kind": "substitution", "substitutions": {"beta gamma": "y"}}),
            &json!({}),
        ),
    ]);
    let report = fix(&overlapping, "Say alpha beta gamma now.\n", None);
    assert_eq!(report.text, "Say x gamma now.\n");
    assert_eq!(report.applied, 1);
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].reason, SkipReason::Overlap);
}

#[test]
fn a_replacement_with_alternatives_is_not_fixed_or_reported_as_skipped() {
    let report = fix(&swaps(), "Send it via mail.\n", None);
    assert_eq!(report.text, "Send it via mail.\n");
    assert!(report.skipped.is_empty());
}

#[test]
#[allow(clippy::single_range_in_vec_init)]
fn only_lines_in_the_changed_ranges_are_fixed() {
    let text = "delta one.\ndelta two.\ndelta three.\n";
    let report = fix(&swaps(), text, Some(&[2..=2]));
    assert_eq!(report.text, "delta one.\nd two.\ndelta three.\n");
    let reasons: Vec<_> = report.skipped.iter().map(|s| s.reason).collect();
    assert_eq!(reasons, [SkipReason::OutsideChangedLines; 2]);
}

#[test]
fn a_document_that_changed_since_the_lint_is_not_fixed() {
    let engine = swaps();
    let linted = "Use delta here.\n";
    let doc = engine.lint(&Input::markdown("doc.md", linted)).unwrap();
    let current = "Use gamma here.\n";
    let report = fix_document(&Input::markdown("doc.md", current), &doc, None).unwrap();
    assert_eq!(report.text, current);
    assert_eq!(report.skipped[0].reason, SkipReason::Stale);
}

#[test]
fn a_match_that_spans_inline_code_is_not_fixed() {
    let spanning = engine(&[rule(
        "across",
        &json!({"kind": "substitution", "substitutions": {"alpha[^.]*beta": "x"}}),
        &json!({}),
    )]);
    let text = "Use alpha `code` beta.\n";
    let doc = spanning.lint(&Input::markdown("doc.md", text)).unwrap();
    assert!(doc.findings[0].fix.is_some(), "the finding offers a fix");
    let report = fix(&spanning, text, None);
    assert_eq!(report.text, text);
    assert_eq!(report.skipped[0].reason, SkipReason::Protected);
}

#[test]
fn unified_diff_has_headers_hunks_and_context() {
    let diff = unified_diff("doc.md", "a\nb\nc\n", "a\nB\nc\n");
    assert_eq!(
        diff,
        "--- a/doc.md\n+++ b/doc.md\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n"
    );
}

#[test]
fn unified_diff_of_equal_texts_is_empty() {
    assert_eq!(unified_diff("doc.md", "a\n", "a\n"), "");
}

fn dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn a_dry_run_reports_the_diff_and_leaves_the_file_alone() {
    let d = dir();
    let path = d.path().join("doc.md");
    std::fs::write(&path, "Use delta here.\n").unwrap();
    let out = fix_file(&swaps(), &path, Kind::Markdown, true, None).unwrap();
    assert!(!out.written);
    assert_eq!(out.report.text, "Use d here.\n");
    assert!(
        out.diff.contains("-Use delta here.\n+Use d here.\n"),
        "{}",
        out.diff
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "Use delta here.\n");
}

#[test]
fn a_write_fixes_the_file_and_lints_the_fixed_text_again() {
    let d = dir();
    let path = d.path().join("doc.md");
    std::fs::write(&path, "Use delta here, delta there.\n").unwrap();
    let out = fix_file(&swaps(), &path, Kind::Markdown, false, None).unwrap();
    assert!(out.written);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "Use d here, d there.\n"
    );
    assert!(out.document.findings.is_empty());
}

#[test]
fn a_file_with_nothing_to_fix_is_not_written() {
    let d = dir();
    let path = d.path().join("doc.md");
    std::fs::write(&path, "Clean text.\n").unwrap();
    let out = fix_file(&swaps(), &path, Kind::Markdown, false, None).unwrap();
    assert!(!out.written);
    assert_eq!(out.diff, "");
}

#[test]
fn a_file_that_is_not_utf8_is_an_error_and_stays_as_it_was() {
    let d = dir();
    let path = d.path().join("doc.md");
    std::fs::write(&path, b"delta \xff\n").unwrap();
    let err = fix_file(&swaps(), &path, Kind::Markdown, false, None).unwrap_err();
    assert!(matches!(err, slopvac_engine::FixError::NotUtf8(_)), "{err}");
    assert_eq!(std::fs::read(&path).unwrap(), b"delta \xff\n");
}

#[cfg(unix)]
#[test]
fn a_symlink_is_refused_and_its_target_is_unchanged() {
    let d = dir();
    let target = d.path().join("real.md");
    let link = d.path().join("link.md");
    std::fs::write(&target, "Use delta here.\n").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let out = fix_file(&swaps(), &link, Kind::Markdown, false, None).unwrap();
    assert_eq!(out.refused, Some(Refusal::Symlink));
    assert!(!out.written);
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "Use delta here.\n"
    );
}

#[cfg(unix)]
#[test]
fn a_hard_linked_file_is_refused_and_both_names_are_unchanged() {
    let d = dir();
    let first = d.path().join("a.md");
    let second = d.path().join("b.md");
    std::fs::write(&first, "Use delta here.\n").unwrap();
    std::fs::hard_link(&first, &second).unwrap();
    let out = fix_file(&swaps(), &first, Kind::Markdown, false, None).unwrap();
    assert_eq!(out.refused, Some(Refusal::Hardlinked));
    assert_eq!(
        std::fs::read_to_string(&second).unwrap(),
        "Use delta here.\n"
    );
}
