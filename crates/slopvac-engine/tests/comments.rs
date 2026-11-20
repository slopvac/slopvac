//! Code-comment mode: rules read the prose of comments and findings point at source bytes.

#![allow(missing_docs, clippy::assert_is_empty)]

mod common;

use common::{category, engine, id, rule, ruleset, settings, tokens};
use rstest::rstest;
use serde_json::{Value, json};
use slopvac_comments::Language;
use slopvac_engine::{
    DiagramChecker, DiagramHit, DiagramInput, DiagramRule, Engine, Input, SkipReason, fix_document,
};
use slopvac_model::{Document, NoticeLevel};

/// A directive that allows the `dull` rule. `@` stands for it in the sources below.
const ALLOW: &str = "<!-- slopvac-allow: rule=core:cat.dull reason=dead-metaphor -->";

fn dull() -> Engine {
    engine(&[tokens(
        "dull",
        "leverage",
        &json!({"exceptions": [{"name": "dead-metaphor"}]}),
    )])
}

fn lint_with(engine: &Engine, language: Language, source: &str) -> Document {
    engine
        .lint(&Input::comments("f", &source.replace('@', ALLOW), language))
        .unwrap()
}

fn lint(language: Language, source: &str) -> Document {
    lint_with(&dull(), language, source)
}

/// The source text each finding's span covers.
fn covered<'a>(doc: &Document, source: &'a str) -> Vec<&'a str> {
    doc.findings
        .iter()
        .map(|f| {
            let span = f.extent.unwrap().span;
            &source[span.start()..span.end()]
        })
        .collect()
}

/// The line of each finding.
fn lines(doc: &Document) -> Vec<usize> {
    doc.findings.iter().map(|f| f.start.line()).collect()
}

#[rstest]
#[case::lf("# a\n# leverage it\n")]
#[case::crlf("# a\r\n# leverage it\r\n")]
#[case::cr("# a\r# leverage it\r")]
#[case::bom("\u{feff}# a\n# leverage it\n")]
fn a_finding_covers_its_exact_source_bytes_and_line(#[case] source: &str) {
    let doc = lint(Language::Shell, source);
    assert_eq!(covered(&doc, source), ["leverage"]);
    assert_eq!(lines(&doc), [2]);
}

#[test]
fn the_comment_marker_is_never_matched() {
    let source = "// we\n// leverage\n// it\n";
    let marker = engine(&[tokens("slashes", "//", &json!({}))]);
    let doc = marker
        .lint(&Input::comments("f", source, Language::Rust))
        .unwrap();
    assert!(doc.findings.is_empty());
}

#[rstest]
#[case::string("x = \"leverage\"\n", Language::Python)]
#[case::code("leverage()\n", Language::JavaScript)]
#[case::string_with_marker("x = \"// leverage\"\n", Language::Rust)]
fn code_and_strings_are_not_read(#[case] source: &str, #[case] language: Language) {
    assert!(lint(language, source).findings.is_empty());
}

#[test]
fn a_trailing_comment_is_read_where_it_stands() {
    let source = "x = 1  # leverage\n";
    let doc = lint(Language::Python, source);
    assert_eq!(covered(&doc, source), ["leverage"]);
    assert_eq!(doc.findings[0].start.column(), 10);
}

/// Where a directive reaches, from the 2.x rule: it covers the comment block that starts on
/// its line, else the next block that holds comment text. A line with only directives, a
/// blank comment line and a line of code are not comment text.
#[rstest]
#[case::covers_the_next_block("# @\n# leverage\n\n# leverage\n", &[4])]
#[case::covers_the_block_it_starts("# @ leverage\n# leverage\n\n# leverage\n", &[4])]
#[case::a_line_of_directives_ends_a_block("# leverage\n# @\n# leverage\n", &[1])]
#[case::reaches_past_code("# @\nx = 1\n# leverage\n", &[])]
#[case::reaches_past_a_blank_line("# @\n\n# leverage\n", &[])]
#[case::covers_nothing_when_no_comment_follows("# leverage\n\n# @\n", &[1])]
fn a_directive_covers_a_comment_block(#[case] source: &str, #[case] lines_found: &[usize]) {
    let doc = lint(Language::Shell, source);
    assert_eq!(lines(&doc), lines_found);
}

/// An example of a directive is not a directive: text indented as a Markdown code block, or
/// inside a fence, is quoted. Two columns of the comment text count, as in 2.x: the delimiter
/// takes one blank and the rest is the author's indentation.
#[rstest]
#[case::hash_indented("#     @\n# leverage\n", Language::Shell, &[2])]
#[case::slashes_indented("//     @\n// leverage\n", Language::Rust, &[2])]
#[case::block_comment_indented("/*     @\n   leverage */\n", Language::JavaScript, &[2])]
#[case::fenced("# ```\n# @\n# ```\n# leverage\n", Language::Shell, &[4])]
#[case::three_columns_is_not_code("#    @\n# leverage\n", Language::Shell, &[])]
#[case::live("# @\n# leverage\n", Language::Shell, &[])]
fn an_indented_or_fenced_directive_is_an_example(
    #[case] source: &str,
    #[case] language: Language,
    #[case] lines_found: &[usize],
) {
    assert_eq!(lines(&lint(language, source)), lines_found);
}

#[rstest]
#[case::heading(json!({"scope": "heading"}))]
#[case::sentence(json!({"scope": "sentence"}))]
#[case::body_sentence(json!({"scope": "body-sentence"}))]
#[case::paragraph(json!({"scope": "paragraph"}))]
#[case::document(json!({"scope": "document"}))]
#[case::raw(json!({"scope": "raw"}))]
fn a_comment_is_the_unit_whatever_the_scope_of_the_rule(#[case] extra: Value) {
    let source = "// leverage it\n";
    let e = engine(&[tokens("dull", "leverage", &extra)]);
    let doc = e
        .lint(&Input::comments("f", source, Language::Rust))
        .unwrap();
    assert_eq!(covered(&doc, source), ["leverage"]);
}

#[rstest]
#[case::sentence("sentence")]
#[case::paragraph("paragraph")]
fn a_match_may_cross_the_lines_of_one_comment(#[case] scope: &str) {
    let source = "// One thing.\n// This can help.\n";
    let e = engine(&[rule(
        "demonstrative",
        &json!({"kind": "pattern", "pattern": r"thing\.\s+This"}),
        &json!({"scope": scope}),
    )]);
    let doc = e
        .lint(&Input::comments("f", source, Language::Rust))
        .unwrap();
    assert_eq!(covered(&doc, source), ["thing.\n// This"]);
    assert_eq!(lines(&doc), [1]);
}

#[test]
fn comments_apart_are_separate_units() {
    let source = "// One thing.\n\n// This can help.\n";
    let e = engine(&[rule(
        "demonstrative",
        &json!({"kind": "pattern", "pattern": r"thing\.\s+This"}),
        &json!({}),
    )]);
    let doc = e
        .lint(&Input::comments("f", source, Language::Rust))
        .unwrap();
    assert!(doc.findings.is_empty());
}

fn has_context(tags: &[slopvac_model::ContextTag], name: &str) -> bool {
    tags.iter().any(|t| t.as_str() == name)
}

#[rstest]
#[case::only_code_comment(json!({"contexts": {"only": ["code-comment"]}}), 1)]
#[case::skip_code_comment(json!({"contexts": {"skip": ["code-comment"]}}), 0)]
#[case::only_other_context(json!({"contexts": {"only": ["requirement"]}}), 0)]
#[case::skip_other_context(json!({"contexts": {"skip": ["requirement"]}}), 1)]
fn the_context_gate_sees_code_comment(#[case] extra: Value, #[case] expected: usize) {
    let e = engine(&[tokens("dull", "leverage", &extra)]);
    let doc = e
        .lint(&Input::comments("f", "# leverage it\n", Language::Shell))
        .unwrap();
    assert_eq!(doc.findings.len(), expected);
}

#[test]
fn a_rule_limited_to_code_comments_stays_silent_in_prose() {
    let e = engine(&[tokens(
        "dull",
        "leverage",
        &json!({"contexts": {"only": ["code-comment"]}}),
    )]);
    let doc = e.lint(&Input::markdown("f.md", "leverage it\n")).unwrap();
    assert!(doc.findings.is_empty());
}

#[test]
fn a_finding_and_its_document_report_the_code_comment_context() {
    let doc = lint(Language::Shell, "# leverage it\n");
    assert!(has_context(&doc.findings[0].contexts, "code-comment"));
    assert!(has_context(&doc.contexts_seen, "code-comment"));
}

#[test]
fn a_source_without_comments_has_no_code_comment_context() {
    let doc = lint(Language::Shell, "x=1\n");
    assert!(!has_context(&doc.contexts_seen, "code-comment"));
}

fn long_rule() -> Value {
    rule(
        "long",
        &json!({"kind": "metric", "metric": "sentence_words", "comparison": "gt", "threshold": 3}),
        &json!({}),
    )
}

/// A checker with one rule that no comment can reach.
struct Acme;

impl DiagramChecker for Acme {
    fn rules(&self) -> Vec<DiagramRule> {
        vec![DiagramRule {
            id: id("acme:diagram.every"),
            severity: slopvac_model::Severity::Suggestion,
        }]
    }

    fn check(&self, _: &DiagramInput<'_>) -> Vec<DiagramHit> {
        Vec::new()
    }
}

fn not_run(doc: &Document) -> Vec<String> {
    let [notice] = doc.unchecked.as_slice() else {
        panic!("{:?}", doc.unchecked)
    };
    assert_eq!(notice.code.as_str(), "comment-rules-not-run");
    assert_eq!(notice.level, NoticeLevel::Info);
    let mut rules: Vec<String> = notice.data["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_owned())
        .collect();
    rules.sort();
    rules
}

#[test]
fn rules_that_cannot_read_comments_are_listed_and_the_run_stays_complete() {
    let e = engine(&[tokens("dull", "leverage", &json!({})), long_rule()]);
    let doc = e
        .lint(&Input::comments("f", "# a b c d e\n", Language::Shell))
        .unwrap();
    assert!(doc.findings.is_empty());
    assert_eq!(not_run(&doc), ["core:cat.long"]);
    assert!(doc.gate.is_passed());
}

#[test]
fn diagram_rules_are_listed_with_the_rules_that_cannot_read_comments() {
    let label = rule(
        "label-run-on",
        &json!({"kind": "structure", "check": "diagram-label-run-on"}),
        &json!({"scope": "prose"}),
    );
    let e = Engine::new(
        vec![ruleset(vec![category("cat", &[long_rule(), label])])],
        settings(),
    )
    .unwrap()
    .with_diagram_checker(Box::new(Acme));
    let doc = e
        .lint(&Input::comments("f", "# fine\n", Language::Shell))
        .unwrap();
    assert_eq!(
        not_run(&doc),
        [
            "acme:diagram.every",
            "core:cat.label-run-on",
            "core:cat.long"
        ]
    );
}

#[test]
fn a_judgement_rule_is_not_listed() {
    let judgement = rule(
        "vague",
        &json!({"kind": "judgement", "question": "Is it vague?"}),
        &json!({"severity": "suggestion"}),
    );
    let e = engine(&[tokens("dull", "leverage", &json!({})), judgement]);
    let doc = e
        .lint(&Input::comments("f", "# fine\n", Language::Shell))
        .unwrap();
    assert!(doc.unchecked.is_empty());
}

#[test]
fn a_clean_source_has_no_notice() {
    assert!(lint(Language::Shell, "# fine\n").unchecked.is_empty());
}

fn swaps() -> Engine {
    engine(&[rule(
        "swap",
        &json!({"kind": "substitution", "substitutions": {
            "leverage": "use", "make use of": "use"}}),
        &json!({}),
    )])
}

fn fixed(language: Language, source: &str) -> slopvac_engine::FixReport {
    let e = swaps();
    let input = Input::comments("f", source, language);
    let doc = e.lint(&input).unwrap();
    fix_document(&e, &input, &doc, None).unwrap()
}

/// Comments are linted and not fixed: the file stays as it is, and each fix is refused.
#[rstest]
#[case::line_comment(Language::Rust, "// leverage it\n")]
#[case::trailing_comment(Language::Python, "x = 1  # leverage it\n")]
#[case::block_comment(Language::JavaScript, "/*\n * leverage it\n */\n")]
#[case::crlf_and_bom(Language::Shell, "\u{feff}# leverage\r\n# it\r\n")]
fn a_fix_in_a_comment_is_refused_and_the_file_is_unchanged(
    #[case] language: Language,
    #[case] source: &str,
) {
    let report = fixed(language, source);
    assert_eq!(report.text, source);
    assert_eq!(report.applied, 0);
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].reason, SkipReason::Protected);
}

/// The id of a rule inside its own directive is not prose.
#[rstest]
#[case::tokens(json!({}), "# @\n# fine\n", 0)]
#[case::raw(json!({"scope": "raw"}), "# @\n# fine\n", 0)]
#[case::prose_beside_an_html_comment_is_read(json!({}), "# dull <!-- note -->\n", 1)]
fn directive_text_is_not_matched(#[case] extra: Value, #[case] source: &str, #[case] n: usize) {
    let mut allow = tokens("dull", "dull", &extra);
    allow["exceptions"] = json!([{"name": "dead-metaphor"}]);
    let doc = lint_with(&engine(&[allow]), Language::Shell, source);
    assert_eq!(doc.findings.len(), n);
}
