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
use slopvac_engine::{Engine, FixReport, Input, SkipReason, fix_document, unified_diff};

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
        let report = fix_document(&engine, &input, &doc, None).unwrap();
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
    fix_document(engine, &input, &doc, lines).unwrap()
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
    let report = fix_document(&engine, &Input::markdown("doc.md", current), &doc, None).unwrap();
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
    let report = fix_document(&engine, &Input::markdown("doc.md", current), &doc, None).unwrap();
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

#[test]
fn a_raw_scope_rule_cannot_rewrite_a_destination_or_an_attribute() {
    let raw = single("delta", "d", &json!({"scope": "raw"}));
    for (text, want) in [
        ("[x](docs/delta.md) delta\n", "[x](docs/delta.md) d\n"),
        (
            "![x](img/delta.png \"delta\") delta\n",
            "![x](img/delta.png \"delta\") d\n",
        ),
        (
            "<a href=\"docs/delta.md\">delta</a>\n",
            "<a href=\"docs/delta.md\">d</a>\n",
        ),
        (
            "<img src='delta.png' alt=delta />\n",
            "<img src='delta.png' alt=delta />\n",
        ),
        (
            "[x][delta]\n\n[delta]: docs/delta.md\n",
            "[x][delta]\n\n[delta]: docs/delta.md\n",
        ),
    ] {
        assert_eq!(fix(&raw, text, None).text, want, "{text:?}");
    }
}

#[test]
fn a_definition_inside_a_quote_keeps_its_references_resolving() {
    let e = single("delta", "d", &json!({}));
    for text in [
        "see [delta] now\n\n> [delta]: /x\n",
        "see [delta][] now\n\n- item\n\n  [delta]: /x\n",
    ] {
        assert!(!fix(&e, text, None).text.contains("[d]"), "{text:?}");
    }
}

fn fix_mdx(engine: &Engine, text: &str) -> String {
    let input = Input::mdx("doc.mdx", text);
    let doc = engine.lint(&input).unwrap();
    fix_document(engine, &input, &doc, None).unwrap().text
}

#[test]
fn mdx_syntax_is_never_rewritten_but_its_prose_is() {
    let raw = single("delta", "d", &json!({"scope": "raw"}));
    for (text, want) in [
        (
            "<Comp title=\"delta\" {...delta} />\n\ndelta\n",
            "<Comp title=\"delta\" {...delta} />\n\nd\n",
        ),
        ("{delta}\n\ndelta\n", "{delta}\n\nd\n"),
        (
            "export const delta = 1\n\ndelta\n",
            "export const delta = 1\n\nd\n",
        ),
        (
            "<A b={delta > 1} c='delta'>delta</A>\n",
            "<A b={delta > 1} c='delta'>d</A>\n",
        ),
        ("text {delta} and delta\n", "text {delta} and d\n"),
    ] {
        assert_eq!(fix_mdx(&raw, text), want, "{text:?}");
    }
}

#[test]
fn an_autolink_inside_an_image_does_not_unprotect_the_image_destination() {
    let raw = single("delta", "d", &json!({"scope": "raw"}));
    let text = "![see <https://example.com> now](img/delta.png) delta\n";
    assert_eq!(
        fix(&raw, text, None).text,
        "![see <https://example.com> now](img/delta.png) d\n"
    );
}

#[test]
fn a_regex_in_an_attribute_expression_never_exposes_the_attributes_after_it() {
    let raw = single("delta", "d", &json!({"scope": "raw"}));
    // Each regex holds a quoted `{`, an escaped `}` and a `>`, which a scanner that does not
    // read regular expressions takes for the end of the expression and of the tag. The MDX
    // parser accepts them, so both must parse. The attribute after the regex keeps `delta`,
    // and the child prose on the same line is fixed.
    for (text, want) in [
        (
            "<A b={/['{']\\}>/} c=\"delta\">delta</A>\n",
            "<A b={/['{']\\}>/} c=\"delta\">d</A>\n",
        ),
        (
            "<A b={/['{']\\}>/.test(x)} c=\"delta\">delta</A>\n",
            "<A b={/['{']\\}>/.test(x)} c=\"delta\">d</A>\n",
        ),
    ] {
        assert_eq!(fix_mdx(&raw, text), want, "{text:?}");
    }
}

#[test]
fn every_js_line_terminator_ends_a_comment_in_a_tag() {
    let raw = single("delta", "d", &json!({"scope": "raw"}));
    for terminator in ["\n", "\r", "\u{2028}", "\u{2029}"] {
        let text = format!("<A b={{1 // note{terminator}}} c=\"delta\">delta</A>\n");
        let fixed = fix_mdx(&raw, &text);
        let want = format!("<A b={{1 // note{terminator}}} c=\"delta\">d</A>\n");
        assert_eq!(fixed, want, "{terminator:?}");
    }
}

#[test]
fn raw_html_prose_is_rewritten_but_its_tags_and_attributes_are_not() {
    let e = single("delta", "d", &json!({}));
    let text = "<p title=\"delta\">A delta <a href=\"delta\">delta</a> end.</p>\n";
    let report = fix(&e, text, None);
    assert_eq!(
        report.text,
        "<p title=\"delta\">A d <a href=\"delta\">d</a> end.</p>\n"
    );
}

#[test]
fn a_fence_in_a_raw_html_block_is_never_rewritten() {
    let e = single("delta", "d", &json!({}));
    let text = "<div>\n```\ndelta code\n```\nSee delta now.\n</div>\n";
    assert_eq!(
        fix(&e, text, None).text,
        "<div>\n```\ndelta code\n```\nSee d now.\n</div>\n"
    );
}

#[test]
fn html_code_and_pre_contents_stay_while_neighbouring_prose_is_fixed() {
    let raw = single("delta", "d", &json!({"scope": "raw"}));
    let text = "<p>delta <code>delta</code> delta <kbd>delta</kbd></p>\n\n<pre>\ndelta\n</pre>\n";
    assert_eq!(
        fix(&raw, text, None).text,
        "<p>d <code>delta</code> d <kbd>delta</kbd></p>\n\n<pre>\ndelta\n</pre>\n"
    );
}
