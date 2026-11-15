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
    Engine, FixReport, Input, Kind, Refusal, SkipReason, fix_document, fix_file, fix_file_hooked,
    unified_diff,
};

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// A document where Rust differs from the reference on purpose. `expected` lists edits,
/// worked out by hand, that turn the reference text into the text Rust must write. Each
/// `find` occurs once in the reference text.
#[derive(serde::Deserialize)]
struct Divergence {
    #[allow(dead_code)]
    reason: String,
    expected: Vec<(String, String)>,
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
        let mut want = entry["fixed"].as_str().unwrap().to_owned();
        if let Some(divergence) = divergences.get(name) {
            assert_ne!(report.text, want, "{name}: agrees; remove it from the list");
            for (find, replace) in &divergence.expected {
                assert_eq!(want.matches(find.as_str()).count(), 1, "{name}: {find:?}");
                want = want.replacen(find.as_str(), replace, 1);
            }
        }
        if report.text != want {
            problems.push(format!(
                "{name}: differs\n{}",
                unified_diff(name, &want, &report.text)
            ));
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

fn single(key: &str, to: &str, extra: &serde_json::Value) -> Engine {
    engine(&[rule(
        "one",
        &json!({"kind": "substitution", "substitutions": {key: to}}),
        extra,
    )])
}

#[test]
fn a_destination_is_never_rewritten_but_its_label_and_prose_are() {
    let e = single("delta", "d", &json!({}));
    for (text, want) in [
        (
            "<https://example.com/delta> and delta.\n",
            "<https://example.com/delta> and d.\n",
        ),
        (
            "<mailto:delta@example.com> delta\n",
            "<mailto:delta@example.com> d\n",
        ),
        (
            "see https://example.com/delta now delta\n",
            "see https://example.com/delta now d\n",
        ),
        (
            "see www.delta.example now delta\n",
            "see www.delta.example now d\n",
        ),
        (
            "mail delta@example.com or delta\n",
            "mail delta@example.com or d\n",
        ),
        (
            "[delta](https://example.com/delta) delta\n",
            "[d](https://example.com/delta) d\n",
        ),
        (
            "[delta][r] delta\n\n[r]: https://example.com/delta\n",
            "[d][r] d\n\n[r]: https://example.com/delta\n",
        ),
        ("[delta]\n\n[delta]: /x\n", "[delta]\n\n[delta]: /x\n"),
        (
            "[a][delta] delta\n\n[Delta]: /x\n",
            "[a][delta] d\n\n[Delta]: /x\n",
        ),
    ] {
        assert_eq!(fix(&e, text, None).text, want, "{text:?}");
    }
}

#[test]
fn stale_unicode_input_is_reported_not_a_panic() {
    let engine = swaps();
    let doc = engine.lint(&Input::markdown("doc.md", "delta\n")).unwrap();
    let current = "abcd\u{e9}\n";
    let report = fix_document(&Input::markdown("doc.md", current), &doc, None).unwrap();
    assert_eq!(report.text, current);
    assert_eq!(report.skipped[0].reason, SkipReason::Stale);
}

#[test]
fn the_replacement_takes_the_case_of_the_match() {
    let all_caps = engine(&[rule(
        "one",
        &json!({"kind": "substitution", "substitutions": {"utilize": "use"}, "ignore_case": true, "match_all_caps": true}),
        &json!({}),
    )]);
    for (text, want) in [
        ("utilize it\n", "use it\n"),
        ("Utilize it\n", "Use it\n"),
        ("UTILIZE it\n", "USE it\n"),
    ] {
        assert_eq!(fix(&all_caps, text, None).text, want, "{text:?}");
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Run `fix_file` on a file holding `Use delta here.`, with `tamper` run just before the commit.
fn raced(tamper: impl FnOnce(&Path)) -> (tempfile::TempDir, slopvac_engine::FileFix) {
    let d = dir();
    let path = d.path().join("doc.md");
    std::fs::write(&path, "Use delta here.\n").unwrap();
    let mut tamper = Some(tamper);
    let out = fix_file_hooked(&swaps(), &path, Kind::Markdown, false, None, &mut || {
        tamper.take().unwrap()(&path);
    })
    .unwrap();
    (d, out)
}

#[test]
fn a_file_edited_before_the_commit_is_left_as_the_editor_saved_it() {
    // Same length, so only the bytes tell.
    let (d, out) = raced(|p| std::fs::write(p, "Use gamma here.\n").unwrap());
    assert_eq!(out.refused, Some(Refusal::Changed));
    assert!(!out.written);
    assert_eq!(
        std::fs::read_to_string(d.path().join("doc.md")).unwrap(),
        "Use gamma here.\n"
    );
    assert_eq!(names(d.path()), ["doc.md"], "no temporary file is left");
}

#[cfg(unix)]
#[test]
fn a_path_swapped_for_a_symlink_before_the_commit_is_not_written_through() {
    let outside = dir();
    let secret = outside.path().join("secret.md");
    std::fs::write(&secret, "Use delta here.\n").unwrap();
    let (d, out) = raced(|p| {
        std::fs::remove_file(p).unwrap();
        std::os::unix::fs::symlink(&secret, p).unwrap();
    });
    assert_eq!(out.refused, Some(Refusal::Changed));
    assert_eq!(
        std::fs::read_to_string(&secret).unwrap(),
        "Use delta here.\n"
    );
    assert!(
        std::fs::symlink_metadata(d.path().join("doc.md"))
            .unwrap()
            .is_symlink()
    );
    assert_eq!(names(d.path()), ["doc.md"]);
}

#[cfg(unix)]
#[test]
fn a_path_swapped_for_another_file_before_the_commit_is_left_alone() {
    let (d, out) = raced(|p| {
        let other = p.with_file_name("other.md");
        std::fs::write(&other, "Use delta here.\n").unwrap();
        std::fs::rename(&other, p).unwrap();
    });
    assert_eq!(out.refused, Some(Refusal::Changed));
    assert_eq!(names(d.path()), ["doc.md"]);
}

#[cfg(unix)]
#[test]
fn a_hard_link_added_before_the_commit_stops_the_write() {
    let (d, out) = raced(|p| std::fs::hard_link(p, p.with_file_name("alias.md")).unwrap());
    assert_eq!(out.refused, Some(Refusal::Changed));
    assert_eq!(
        std::fs::read_to_string(d.path().join("alias.md")).unwrap(),
        "Use delta here.\n"
    );
    assert_eq!(names(d.path()), ["alias.md", "doc.md"]);
}

#[cfg(unix)]
#[test]
fn a_write_keeps_the_mode_and_leaves_no_temporary_file() {
    use std::os::unix::fs::PermissionsExt;
    let d = dir();
    let path = d.path().join("doc.md");
    std::fs::write(&path, "Use delta here.\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    let out = fix_file(&swaps(), &path, Kind::Markdown, false, None).unwrap();
    assert!(out.written);
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o640);
    assert_eq!(names(d.path()), ["doc.md"]);
}
