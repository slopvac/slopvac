//! Structure rules: `heading-hierarchy` runs as the 2.x script ran it, and the five checks
//! that 2.x never implemented stay silent.

#![allow(missing_docs)]

mod common;

use common::{category, engine, id, places, rule, ruleset, settings};
use serde_json::{Value, json};
use slopvac_engine::cases::{Outcome, run_cases};
use slopvac_engine::{Engine, Input, Level, Mode};
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
    // The 2.x script left {match} empty.
    assert_eq!(f.message.as_str(), "heading hierarchy:  -- fix the level");
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

// The five checks 2.x never implemented: rule id, and the `bad` example from the rule data.
const SILENT: [(&str, &str); 5] = [
    (
        "vertical-list-lead-in-missing-colon",
        "The manifest sets these values.\n- The image tag\n- The replica count\n",
    ),
    (
        "vertical-list-item-punctuation",
        "The manifest sets these values:\n- the image tag,\n- the replica count;\n- the resource limits\n",
    ),
    (
        "safety-block-missing-consequence",
        "WARNING: Do not use the force flag on a production database.\n",
    ),
    (
        "inconsistent-term-for-same-thing",
        "The worker reads the task queue. Later, the job runner drains the work list.\n",
    ),
    (
        "inconsistent-wording-for-same-step",
        "Step 3. Apply the migration with the CLI.\n\nStep 9. Use the CLI to run the migration.\n",
    ),
];

#[test]
fn the_checks_that_2x_never_implemented_report_nothing_and_add_no_notice() {
    for (check, bad) in SILENT {
        let r = rule(
            check,
            &json!({"kind": "structure", "check": check}),
            &json!({"scope": "paragraph"}),
        );
        let doc = lint(&engine(&[r]), bad);
        assert!(doc.findings.is_empty(), "{check}: {:?}", doc.findings);
        assert!(doc.unchecked.is_empty(), "{check}: {:?}", doc.unchecked);
        assert!(doc.gate.is_passed(), "{check}");
    }
}

#[test]
fn the_case_runner_never_passes_a_check_that_does_not_run() {
    for (check, bad) in SILENT {
        let r = rule(
            check,
            &json!({"kind": "structure", "check": check}),
            &json!({"cases": [{"input": bad, "want": 0}]}),
        );
        let report = run_cases(&engine(&[r]), None).unwrap();
        assert_eq!(report.passed(), 0, "{check}");
        let incomplete: Vec<_> = report.incomplete().map(|r| &r.outcome).collect();
        assert_eq!(
            incomplete,
            [&Outcome::Incomplete(
                "the engine reports nothing for this structure check, as 2.x did: it has no \
                 implementation"
                    .to_owned()
            )],
            "{check}"
        );
    }
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
    assert_eq!(n.level, NoticeLevel::Info);
    assert!(
        n.message
            .as_str()
            .starts_with("1 active structure rule(s) did not run")
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
