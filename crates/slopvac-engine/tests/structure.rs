//! Structure rules: `heading-hierarchy` runs as the 2.x script ran it, and the five checks
//! that 2.x never implemented stay silent.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![allow(missing_docs, reason = "test crate: nothing public to document")]

mod common;

use common::{category, engine, id, places, rule, ruleset, settings};
use serde_json::{Value, json};
use slopvac_engine::cases::run_cases;
use slopvac_engine::{Detector, Detectors, Engine, Input, Level, MatchSite, Mode};
use slopvac_model::{Document, NoticeLevel};

fn hierarchy(extra: &Value) -> Value {
    rule(
        "heading-hierarchy",
        &json!({"kind": "structure", "check": "heading-hierarchy"}),
        &{
            let mut merged = json!({
                "scope": "document",
                "message": "heading hierarchy: {match} -- fix the level",
            });
            for (k, v) in extra.as_object().into_iter().flatten() {
                merged[k] = v.clone();
            }
            merged
        },
    )
}

fn lint(engine: &Engine, text: &str) -> Document {
    engine.lint(&Input::markdown("a.md", text)).unwrap()
}

fn lines(text: &str) -> Vec<usize> {
    let doc = lint(&engine(&[hierarchy(&json!({}))]), text);
    places(&doc).into_iter().map(|(_, line, _)| line).collect()
}

#[test]
fn a_skipped_heading_level_is_reported_on_the_heading_line() {
    let doc = lint(
        &engine(&[hierarchy(&json!({}))]),
        "# One\n\nBody text.\n\n#### Four\n\nMore body.\n",
    );
    assert_eq!(doc.findings.len(), 1);
    let f = &doc.findings[0];
    assert_eq!(f.rule_id.as_str(), "core:cat.heading-hierarchy");
    assert_eq!((f.start.line(), f.start.column()), (5, 1));
    assert_eq!(f.matched_text, "#### Four");
    // {match} is empty and the whitespace around it folds.
    assert_eq!(f.message.as_str(), "heading hierarchy: -- fix the level");
    let extent = f.extent.as_ref().unwrap();
    assert_eq!((extent.span.start(), extent.span.end()), (19, 28));
    assert_eq!((extent.end.line(), extent.end.column()), (5, 10));
    assert!(doc.unchecked.is_empty(), "{:?}", doc.unchecked);
}

#[test]
fn the_cases_of_the_rule_run_through_the_case_runner() {
    let r = hierarchy(&json!({"cases": [
        {"input": "# One\n\nBody.\n\n#### Four\n", "want": 1, "contains": ["#### Four"]},
        {"input": "# One\n\nBody.\n\n## Two\n\n### Three\n", "want": 0},
        {"input": "## Start\n\nBody.\n\n#### Four\n\n##### Five\n", "want": [1, 1]},
    ]}));
    let report = run_cases(&engine(&[r]), None).unwrap();
    assert_eq!(report.passed(), 3, "{report}");
    assert!(report.is_clean(), "{report}");
}

#[test]
fn headings_in_front_matter_set_the_level_but_are_not_reported() {
    // The front matter holds `## yaml`, so `#### Four` skips a level.
    assert_eq!(lines("---\n## yaml\n---\n#### Four\n\nBody.\n"), [4]);
    // A skip inside the front matter is dropped.
    assert_eq!(
        lines("---\n# c\n### d\n---\n# One\n\nBody.\n"),
        Vec::<usize>::new()
    );
}

#[test]
fn headings_inside_a_comment_set_the_level_but_are_not_reported() {
    // `#### x` skips a level but sits in a comment; it still sets the level to 4.
    assert_eq!(
        lines("# One\n\n<!--\n#### x\n-->\n#### Four\n"),
        Vec::<usize>::new()
    );
    // A one-line comment is not a heading line, so `#### Four` skips from `# One`.
    assert_eq!(lines("# One\n\n<!-- #### x -->\n#### Four\n"), [4]);
    // An unclosed opener hides nothing.
    assert_eq!(lines("# One\n\n<!--\n#### x\n"), [4]);
}

#[test]
fn a_heading_after_a_comment_that_ends_on_the_line_before_is_reported() {
    assert_eq!(lines("# One\n\n<!--\n-->\n#### Four\n"), [5]);
}

#[test]
fn fenced_headings_are_not_headings() {
    assert_eq!(
        lines("# One\n\n```\n# not\n#### x\n```\n\n~~~\n#### y\n~~~\n\n#### z\n"),
        [12]
    );
}

#[test]
fn headings_in_raw_html_blocks_count_as_in_2x() {
    assert_eq!(lines("# One\n\n<div>\n#### inhtml\n</div>\n"), [4]);
}

#[test]
fn mdx_and_crlf_files_are_read_the_same_way() {
    let e = engine(&[hierarchy(&json!({}))]);
    let doc = e
        .lint(&Input::mdx(
            "a.mdx",
            "# One\r\n\r\nBody.\r\n\r\n### Three\r\n",
        ))
        .unwrap();
    assert_eq!(
        places(&doc),
        [("core:cat.heading-hierarchy".to_owned(), 5, 1)]
    );
    assert_eq!(doc.findings[0].matched_text, "### Three");
}

#[test]
fn the_level_gates_of_other_kinds_apply() {
    let text = "# One\n\nBody.\n\n#### Four\n";
    let mut off = settings();
    off.rule_levels
        .insert(id("core:cat.heading-hierarchy"), Level::Off);
    let with = |s, r| Engine::new(vec![ruleset(vec![category("cat", &[r])])], s).unwrap();
    assert_eq!(
        lint(&with(off, hierarchy(&json!({}))), text).findings.len(),
        0
    );

    let other_genre = hierarchy(&json!({"applies_to": {"genres": ["tutorial"]}}));
    assert_eq!(lint(&with(settings(), other_genre), text).findings.len(), 0);

    let code_only = hierarchy(&json!({"applies_to": {"modes": ["code-comments"]}}));
    assert_eq!(
        lint(&with(settings(), code_only.clone()), text)
            .findings
            .len(),
        0
    );
    let mut comments = settings();
    comments.mode = Mode::CodeComments;
    assert_eq!(lint(&with(comments, code_only), text).findings.len(), 1);
}

#[test]
fn exclude_blocks_and_context_gates_apply() {
    let text = "# One\n\nBody.\n\n#### Four\n";
    let excluded = hierarchy(&json!({"exclude_blocks": ["heading"]}));
    assert_eq!(lint(&engine(&[excluded]), text).findings.len(), 0);
    let only_quotes = hierarchy(&json!({"contexts": {"only": ["quote"]}}));
    assert_eq!(lint(&engine(&[only_quotes]), text).findings.len(), 0);
    let skip_quotes = hierarchy(&json!({"contexts": {"skip": ["quote"]}}));
    assert_eq!(lint(&engine(&[skip_quotes]), text).findings.len(), 1);
}

#[test]
fn a_directive_suppresses_the_finding_on_its_line() {
    let e = engine(&[hierarchy(&json!({}))]);
    let text = "# One\n\n<!-- slopvac-disable-next-line -->\n#### Four\n\n###### Six\n";
    let doc = lint(&e, text);
    assert_eq!(
        places(&doc),
        [("core:cat.heading-hierarchy".to_owned(), 6, 1)]
    );
}

#[test]
fn the_severity_follows_the_rule() {
    let doc = lint(
        &engine(&[hierarchy(&json!({"severity": "error"}))]),
        "# One\n\n### Three\n",
    );
    assert_eq!((doc.errors, doc.warnings), (1, 0));
}

#[test]
fn a_check_the_engine_does_not_know_is_still_reported_as_not_run() {
    let r = rule(
        "mystery",
        &json!({"kind": "structure", "check": "mystery-check"}),
        &json!({}),
    );
    let doc = lint(&engine(&[r]), "Some text.\n");
    assert_eq!(doc.unchecked.len(), 1);
    let n = &doc.unchecked[0];
    assert_eq!(n.code.as_str(), "rule-kind-not-implemented");
    assert_eq!(n.level, NoticeLevel::Warning);
    assert!(
        n.message
            .as_str()
            .starts_with("1 active structure rule(s) did not run")
    );
    assert!(
        !doc.gate.is_passed(),
        "a warning notice makes the run incomplete"
    );
}

#[test]
fn the_reported_lines_match_the_2x_reference_on_probe_documents() {
    // Each expectation is the line list that `slopvac lint --profile normal` 2.x printed
    // for the same file.
    let probes: [(&str, &[usize]); 8] = [
        (
            "# One\n\nText here.\n\n#### Four\n\nBody.\n\n## Two\n\n#### Skip2\n\n# Back\n\n### Three\n",
            &[5, 11, 15],
        ),
        (
            "# One\n\n```\n# not heading\n#### x\n```\n\n~~~\n#### y\n~~~\n\nBody.\n\n  ## indented\n\n#nospace\n\n####### seven\n\n> #### quoted\n\nSetext\n======\n\n### H3\n",
            &[18],
        ),
        (
            "# One\n\n    ```\n#### a\n    ```\n\nBody.\n\n```\n#### unclosed\n",
            &[4],
        ),
        (
            "# One\n\n- item\n\n   ```\n#### infence\n   ```\n\nBody.\n\n  ```\n#### two\n  ```\n\n#### Four\n",
            &[15],
        ),
        (
            "- # One\n\n  Body.\n\n#\tTab\n\n##\tTab2\n\n# One\n\n## Two\n\n### \n\n#### Empty3\n",
            &[],
        ),
        (
            "# One\n\n<!-- a\n--> #### notline\n<!-- x --> \n#### Four\n\n<!-- c -->#### Five\n\n<!--\n-->\n###### Six\n",
            &[6, 12],
        ),
        (
            "# One\n\nBody.\n\n#### Four <!-- c -->\n\n<!--\n-->\n#### Five\n",
            &[5],
        ),
        ("---\n---\n# One\n\n#### Four\n", &[5]),
    ];
    for (text, want) in probes {
        assert_eq!(lines(text), want, "{text:?}");
    }
}

#[test]
fn headings_are_found_after_lf_crlf_and_lone_cr_line_ends() {
    // An h1 then an h4 in a CR-only document: the skip is on line 3.
    assert_eq!(lines("# One\rBody.\r#### Four\r"), [3]);
    assert_eq!(lines("# One\r\nBody.\r\n#### Four\r\n"), [3]);
    assert_eq!(lines("# One\nBody.\r#### Four\r\nBody\n"), [3]);
}

#[test]
fn a_lone_cr_heading_finding_covers_the_heading_text_only() {
    let doc = lint(
        &engine(&[hierarchy(&json!({}))]),
        "# One\rBody.\r#### Four\rMore.\r",
    );
    assert_eq!(doc.findings[0].matched_text, "#### Four");
}

#[test]
fn fences_work_with_lone_cr_line_ends() {
    assert_eq!(lines("# One\r```\r#### x\r```\r#### y\r"), [5]);
}

#[test]
fn a_fence_indented_four_spaces_is_not_a_fence() {
    assert_eq!(lines("# One\n\n    ```\n#### x\n"), [4]);
}

#[test]
fn an_unclosed_fence_hides_the_rest() {
    assert_eq!(lines("# One\n\n```\n#### x\n"), Vec::<usize>::new());
}

#[test]
fn a_fence_hides_its_lines_and_any_fence_line_closes_it() {
    assert_eq!(lines("# a\n\n```\n#### x\n~~~\n\n#### y\n"), [7]);
}

#[test]
fn a_skipped_level_is_flagged_and_a_step_down_is_not() {
    assert_eq!(lines("# a\n## b\n#### c\n"), [3]);
    assert_eq!(lines("# a\n## b\n### c\n"), Vec::<usize>::new());
}

#[test]
fn the_first_heading_may_start_at_any_level() {
    assert_eq!(lines("### a\n"), Vec::<usize>::new());
}

#[test]
fn a_heading_after_a_skip_compares_with_the_skipped_one() {
    assert_eq!(lines("# a\n#### b\n##### c\n###### d\n"), [2]);
}

#[test]
fn going_back_up_and_then_down_by_one_is_fine() {
    assert_eq!(lines("# a\n## b\n### c\n# d\n## e\n"), Vec::<usize>::new());
}

#[test]
fn lines_that_are_not_atx_headings_do_not_count() {
    for text in [
        "  #### indented",
        "#nospace",
        "####",
        "#\ttab",
        "> #### quoted",
        "- #### item",
    ] {
        assert_eq!(
            lines(&format!("# a\n\n{text}\n")),
            Vec::<usize>::new(),
            "{text:?} is not a heading"
        );
    }
}

#[test]
fn deep_and_empty_headings_count() {
    assert_eq!(lines("# a\n\n####### b\n"), [3]);
    assert_eq!(lines("# a\n\n#### \n"), [3]);
}

#[test]
fn an_opener_does_not_close_on_its_own_dashes() {
    // `<!-->` is not a closed comment, so the next line is reported.
    assert_eq!(lines("# One\n<!-->\n#### Four\n"), [3]);
    // A comment closed later hides the heading that starts inside it.
    assert_eq!(lines("# One\n<!-->\n#### Four\n-->\n"), Vec::<usize>::new());
}

struct Marked;

impl Detector for Marked {
    fn name(&self) -> &'static str {
        "marked"
    }

    fn detects(&self, site: &MatchSite<'_>) -> bool {
        site.text.contains("legacy")
    }
}

fn marked_engine(extra: &Value) -> Engine {
    let mut detectors = Detectors::builtin();
    detectors.register(Box::new(Marked)).unwrap();
    engine(&[hierarchy(extra)]).with_detectors(detectors)
}

#[test]
fn an_exception_detector_suppresses_the_finding_it_detects() {
    let extra = json!({"exceptions": [{"name": "marked", "detector": "marked"}]});
    let doc = lint(
        &marked_engine(&extra),
        "# One\n\n#### legacy Four\n\n###### Six\n",
    );
    assert_eq!(
        places(&doc),
        [("core:cat.heading-hierarchy".to_owned(), 5, 1)]
    );
    // Control: without the exception both skips are reported.
    let control = lint(
        &marked_engine(&json!({})),
        "# One\n\n#### legacy Four\n\n###### Six\n",
    );
    assert_eq!(control.findings.len(), 2);
}

#[test]
fn a_named_exemption_suppresses_the_finding_it_covers() {
    let extra = json!({"exceptions": [{"name": "legacy-layout"}]});
    let e = engine(&[hierarchy(&extra)]);
    let allowed = "# One\n\n<!-- slopvac-allow: rule=core:cat.heading-hierarchy reason=legacy-layout -->\n#### Four\n";
    assert_eq!(lint(&e, allowed).findings.len(), 0);
    // Control: the same heading without the directive is reported.
    assert_eq!(lint(&e, "# One\n\n#### Four\n").findings.len(), 1);
}

#[test]
fn raw_scoped_rules_keep_findings_in_front_matter_and_comments() {
    let raw = |extra: Value| {
        let mut merged = json!({"scope": "raw"});
        for (k, v) in extra.as_object().into_iter().flatten() {
            merged[k] = v.clone();
        }
        engine(&[hierarchy(&merged)])
    };
    let count = |e: &Engine, text: &str| lint(e, text).findings.len();
    // A skip in front matter and a skip in a comment.
    let in_front = "---\n# c\n### d\n---\n# One\n";
    let in_comment = "# One\n\n<!--\n#### x\n-->\n";
    let document = engine(&[hierarchy(&json!({}))]);
    assert_eq!(count(&document, in_front), 0);
    assert_eq!(count(&document, in_comment), 0);
    let raw_engine = raw(json!({}));
    assert_eq!(count(&raw_engine, in_front), 1);
    assert_eq!(count(&raw_engine, in_comment), 1);
}

#[test]
fn explicit_exclusions_still_hide_raw_scoped_findings() {
    let e = engine(&[hierarchy(
        &json!({"scope": "raw", "exclude_blocks": ["front-matter"]}),
    )]);
    assert_eq!(lint(&e, "---\n# c\n### d\n---\n# One\n").findings.len(), 0);
    assert_eq!(lint(&e, "# One\n\n<!--\n#### x\n-->\n").findings.len(), 1);
}
