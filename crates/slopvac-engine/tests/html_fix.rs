//! A standalone HTML file gets no fix: its findings carry none, and a fix run edits nothing.
mod common;

use common::{engine, rule};
use rstest::rstest;
use serde_json::json;
use slopvac_engine::{Engine, Input, SkipReason, fix_document};
use slopvac_model::{Edit, Extent, Fix, Span};

/// An engine whose one rule replaces `delta` with `d`.
fn swap() -> Engine {
    engine(&[rule(
        "swap",
        &json!({"kind": "substitution", "substitutions": {"delta": "d"}}),
        &json!({}),
    )])
}

#[rstest]
#[case::markdown(Input::markdown("page.md", "We use delta now.\n"), true)]
#[case::html(Input::html("page.html", "We use delta now.\n"), false)]
fn a_finding_carries_a_fix_only_outside_html(#[case] input: Input<'_>, #[case] fix: bool) {
    let doc = swap().lint(&input).expect("lint");
    assert_eq!(doc.findings.len(), 1, "{:?}", doc.findings);
    assert_eq!(doc.findings[0].fix.is_some(), fix);
}

#[test]
fn a_fix_run_on_an_html_file_edits_nothing_and_reports_each_fix_as_protected() {
    let html = "<p>We use delta now.</p>\n";
    let engine = swap();
    let input = Input::html("page.html", html);
    let mut doc = engine.lint(&input).expect("lint");
    // A fix that a caller built itself: `delta` is at 10..15.
    let finding = &mut doc.findings[0];
    let span = Span::new(10, 15).expect("span");
    let end = finding.extent.expect("extent").end;
    finding.extent = Some(Extent { span, end });
    finding.fix = Some(
        Fix::new(vec![Edit {
            span,
            text: "d".to_owned(),
        }])
        .expect("fix"),
    );
    let report = fix_document(&engine, &input, &doc, None).expect("fix");
    assert_eq!(report.text, html);
    assert_eq!(report.applied, 0);
    let reasons: Vec<SkipReason> = report.skipped.iter().map(|s| s.reason).collect();
    assert_eq!(reasons, [SkipReason::Protected]);
}
