//! The vocabulary rules: the word blocklist, keyed by part of speech, on the Rust tagger.
//!
//! The expected tags are linguistic facts worked out apart from the code (a noun follows
//! "the", a verb follows "we"), and the scope cases state what Vale lints: prose inside
//! inline HTML, no inline code, hyphenated words whole.

#![allow(
    missing_docs,
    clippy::assert_is_empty,
    clippy::cloned_ref_to_slice_refs
)]

mod common;

use std::fmt::Write as _;

use common::{category, matched, places, rule, ruleset, settings};
use serde_json::{Value, json};
use slopvac_engine::{Engine, Input};
use slopvac_model::{Document, NoticeLevel};
use slopvac_terms::Blocklist;

const OWNER: &str = "word-outside-controlled-vocabulary";

fn vocabulary_rule(id: &str, extra: &Value) -> Value {
    rule(id, &json!({"kind": "vocabulary", "check": id}), extra)
}

fn owner() -> Value {
    vocabulary_rule(
        OWNER,
        &json!({"message": "\"{match}\" is on the blocklist."}),
    )
}

fn list(entries: &[(&str, &str)]) -> Blocklist {
    let mut text = String::new();
    for (word, pos) in entries {
        let _ = write!(
            text,
            "[[entries]]\nword = \"{word}\"\npos = \"{pos}\"\nreason = \"r\"\n"
        );
    }
    Blocklist::parse("b.toml", "toml", &text).unwrap()
}

fn engine_with(rules: &[Value], blocklist: &Blocklist) -> Engine {
    Engine::new(
        vec![ruleset(vec![category("ste-words", rules)])],
        settings(),
    )
    .unwrap()
    .with_blocklist(blocklist)
    .unwrap()
}

fn lint(entries: &[(&str, &str)], text: &str) -> Document {
    engine_with(&[owner()], &list(entries))
        .lint(&Input::markdown("a.md", text))
        .unwrap()
}

#[test]
fn a_word_is_flagged_only_as_the_part_of_speech_the_blocklist_names() {
    // (blocklist word, part of speech, text, whether the word is flagged)
    let cases = [
        ("deploy", "noun", "The deploy failed.\n", true),
        ("deploy", "noun", "We deploy the app.\n", false),
        ("request", "verb", "We request the data.\n", true),
        ("request", "verb", "The request failed.\n", false),
        ("easy", "adjective", "The task is easy.\n", true),
        ("easy", "adjective", "We solved it with ease.\n", false),
        ("quickly", "adverb", "It runs quickly.\n", true),
        ("quickly", "adverb", "It is a quick run.\n", false),
    ];
    for (word, pos, text, flagged) in cases {
        let doc = lint(&[(word, pos)], text);
        let want: Vec<&str> = if flagged { vec![word] } else { Vec::new() };
        assert_eq!(matched(&doc), want, "{word} as {pos} in {text:?}");
    }
}

#[test]
fn a_flagged_word_reports_the_owner_rule_with_exact_position_and_message() {
    let doc = lint(
        &[("deploy", "noun")],
        "Caf\u{e9} notes.\n\nThe Deploy failed.\n",
    );
    assert_eq!(
        places(&doc),
        [(
            "core:ste-words.word-outside-controlled-vocabulary".to_owned(),
            3,
            5
        )]
    );
    let finding = &doc.findings[0];
    assert_eq!(
        finding.matched_text, "Deploy",
        "the word is matched in any case"
    );
    assert_eq!(finding.message.as_str(), "\"Deploy\" is on the blocklist.");
    assert_eq!(finding.extent.unwrap().span.start(), 18);
}

#[test]
fn inline_code_is_not_prose() {
    let doc = lint(
        &[("deploy", "noun")],
        "Run `deploy` then the deploy failed.\n",
    );
    assert_eq!(places(&doc).len(), 1);
    // Column 23 is the second `deploy`; the first sits inside the code span.
    assert_eq!(doc.findings[0].start.column(), 23);
}

#[test]
fn tags_around_inline_code_match_the_source() {
    // Vale tags "return" here as a verb, and it must not read as the refused noun. The
    // tagger sees a run of `*` as wide as each code span, so the tags around it hold.
    let source = "`recheck()` and `cancel()` each return a new lazy promise.\n";
    let doc = lint(&[("return", "noun")], source);
    assert_eq!(matched(&doc), Vec::<&str>::new());
}

#[test]
fn a_word_after_inline_code_and_multibyte_text_has_an_exact_span() {
    let source = "\u{e9}t\u{e9} `a` `b` and the deploy.\n";
    let doc = lint(&[("deploy", "noun")], source);
    let span = doc.findings[0].extent.unwrap().span;
    assert_eq!(&source[span.start()..span.end()], "deploy");
    assert_eq!(doc.findings[0].start.column(), 21);
}

#[test]
fn a_hyphenated_word_is_one_token() {
    // "deploy-time" is one word, so the listed noun "deploy" is not in it.
    let doc = lint(&[("deploy", "noun")], "The deploy-time flag is set.\n");
    assert_eq!(matched(&doc), Vec::<&str>::new());
    let doc = lint(&[("deploy-time", "noun")], "The deploy-time flag is set.\n");
    assert_eq!(
        matched(&doc),
        Vec::<&str>::new(),
        "an entry with a hyphen never matches"
    );
}

#[test]
fn prose_inside_inline_html_is_linted() {
    let source = "This is <em>easy</em> to do.\n";
    let doc = lint(&[("easy", "adjective")], source);
    assert_eq!(matched(&doc), ["easy"]);
    assert_eq!(doc.findings[0].start.column(), 13);
}

#[test]
fn prose_inside_inline_html_in_a_table_cell_is_linted() {
    let source = "| Step | Action |\n| --- | --- |\n| 1 | We <b>request</b> the data |\n";
    let doc = lint(&[("request", "verb")], source);
    assert_eq!(matched(&doc), ["request"]);
    assert_eq!(
        (doc.findings[0].start.line(), doc.findings[0].start.column()),
        (3, 13)
    );
}

#[test]
fn a_heading_is_tagged_as_its_own_block() {
    // The heading has no terminal punctuation. It must not join the next paragraph into
    // one sentence, where "Deploy" would read as a noun mid-sentence.
    let doc = lint(&[("deploy", "verb")], "# Deploy the app\n\nThe app runs.\n");
    assert_eq!(matched(&doc), ["Deploy"]);
}

#[test]
fn a_directive_suppresses_a_vocabulary_finding() {
    let source = "<!-- slopvac-disable-next-line -->\nThe deploy failed.\n\nThe deploy again.\n";
    let doc = lint(&[("deploy", "noun")], source);
    assert_eq!(doc.findings.len(), 1);
    assert_eq!(doc.findings[0].start.line(), 4);
}

#[test]
fn a_quotation_exception_keeps_a_quoted_word_out() {
    let rules = [vocabulary_rule(
        OWNER,
        &json!({"exceptions": [{"name": "quotation", "detector": "quotation"}]}),
    )];
    let engine = engine_with(&rules, &list(&[("deploy", "noun")]));
    let doc = engine
        .lint(&Input::markdown(
            "a.md",
            "Write \"the deploy\" or the deploy.\n",
        ))
        .unwrap();
    assert_eq!(doc.findings.len(), 1);
    assert_eq!(doc.findings[0].start.column(), 27);
}

#[test]
fn a_blocklist_entry_of_two_letters_or_with_digits_never_matches() {
    let doc = lint(&[("go", "verb"), ("k8s", "noun")], "We go to k8s now.\n");
    assert!(doc.findings.is_empty());
}

#[test]
fn a_part_of_speech_without_a_rule_never_matches() {
    let doc = lint(&[("the", "article")], "The app runs.\n");
    assert!(doc.findings.is_empty());
}

#[test]
fn without_a_blocklist_the_rules_say_why_they_did_not_run() {
    let engine = Engine::new(
        vec![ruleset(vec![category("ste-words", &[owner()])])],
        settings(),
    )
    .unwrap();
    let doc = engine
        .lint(&Input::markdown("a.md", "The deploy failed.\n"))
        .unwrap();
    assert!(doc.findings.is_empty());
    let [notice] = doc.unchecked.as_slice() else {
        panic!("one notice, got {:?}", doc.unchecked);
    };
    assert_eq!(notice.code.as_str(), "vocabulary-no-blocklist");
    assert_eq!(notice.level, NoticeLevel::Info);
    assert!(doc.gate.is_passed());
}

#[test]
fn an_empty_blocklist_is_the_same_as_none() {
    let engine = engine_with(&[owner()], &Blocklist::default());
    let doc = engine
        .lint(&Input::markdown("a.md", "The deploy failed.\n"))
        .unwrap();
    assert_eq!(doc.unchecked[0].code.as_str(), "vocabulary-no-blocklist");
}

#[test]
fn a_blocklist_with_nothing_to_tag_says_so() {
    let doc = lint(&[("the", "article")], "The app runs.\n");
    let [notice] = doc.unchecked.as_slice() else {
        panic!("one notice, got {:?}", doc.unchecked);
    };
    assert_eq!(notice.code.as_str(), "vocabulary-no-taggable-entries");
}

#[test]
fn a_run_sweep_has_no_notice_when_one_rule_owns_it() {
    let doc = lint(&[("deploy", "noun")], "The deploy failed.\n");
    assert!(doc.unchecked.is_empty());
}

#[test]
fn one_vocabulary_rule_owns_the_sweep_and_the_others_report_nothing() {
    let wrong_pos = vocabulary_rule("word-used-in-wrong-part-of-speech", &json!({}));
    let forms = vocabulary_rule("verb-or-adjective-form-not-permitted", &json!({}));
    // The preferred owner is last, so election cannot be "the first rule".
    let engine = engine_with(&[forms, wrong_pos, owner()], &list(&[("deploy", "noun")]));
    let doc = engine
        .lint(&Input::markdown("a.md", "The deploy failed.\n"))
        .unwrap();
    assert_eq!(
        places(&doc),
        [(
            "core:ste-words.word-outside-controlled-vocabulary".to_owned(),
            1,
            5
        )]
    );
    let [notice] = doc.unchecked.as_slice() else {
        panic!("one notice, got {:?}", doc.unchecked);
    };
    assert_eq!(notice.code.as_str(), "vocabulary-sweep-shared");
    assert!(notice.message.as_str().contains(OWNER));
}

#[test]
fn another_vocabulary_rule_owns_the_sweep_when_the_preferred_one_is_absent() {
    let wrong_pos = vocabulary_rule("word-used-in-wrong-part-of-speech", &json!({}));
    let forms = vocabulary_rule("verb-or-adjective-form-not-permitted", &json!({}));
    let engine = engine_with(&[forms, wrong_pos], &list(&[("deploy", "noun")]));
    let doc = engine
        .lint(&Input::markdown("a.md", "The deploy failed.\n"))
        .unwrap();
    assert_eq!(
        places(&doc)[0].0,
        "core:ste-words.word-used-in-wrong-part-of-speech"
    );
}
#[test]
fn an_adverb_entry_matches_a_wh_adverb_because_the_tag_pattern_is_unanchored() {
    // Vale's `RB|RBR|RBS` pattern is unanchored, so it also matches `WRB`, the tag of
    // "how" in this sentence. 2.x flagged it, and so must we.
    let doc = lint(&[("how", "adverb")], "Explain how the job fails.\n");
    assert_eq!(matched(&doc), ["how"]);
}

fn lint_with(extra: &Value, entries: &[(&str, &str)], text: &str) -> Document {
    engine_with(&[vocabulary_rule(OWNER, extra)], &list(entries))
        .lint(&Input::markdown("a.md", text))
        .unwrap()
}

#[test]
fn excluded_blocks_are_not_swept() {
    let text = "# The deploy\n\nThe deploy failed.\n\n| a |\n| - |\n| The deploy |\n";
    let all = lint_with(&json!({}), &[("deploy", "noun")], text);
    assert_eq!(
        all.findings.len(),
        3,
        "without exclusions every block is swept"
    );
    for (kind, kept_lines) in [("heading", vec![3, 7]), ("table", vec![1, 3])] {
        let doc = lint_with(
            &json!({"exclude_blocks": [kind]}),
            &[("deploy", "noun")],
            text,
        );
        let lines: Vec<usize> = doc.findings.iter().map(|f| f.start.line()).collect();
        assert_eq!(lines, kept_lines, "exclude_blocks {kind}");
    }
}

#[test]
fn context_gates_apply_to_the_sweep() {
    let text = "> The deploy failed.\n\nThe deploy failed.\n";
    let skip = lint_with(
        &json!({"contexts": {"skip": ["quote"]}}),
        &[("deploy", "noun")],
        text,
    );
    assert_eq!(places(&skip).iter().map(|p| p.1).collect::<Vec<_>>(), [3]);
    let only = lint_with(
        &json!({"contexts": {"only": ["quote"]}}),
        &[("deploy", "noun")],
        text,
    );
    assert_eq!(places(&only).iter().map(|p| p.1).collect::<Vec<_>>(), [1]);
}

#[test]
fn an_escaped_backtick_does_not_pair_with_a_code_span() {
    // The parser decodes \` to a literal backtick. It must not open a code span in the
    // tagger text that swallows "the deploy" up to the real code span.
    let doc = lint(
        &[("deploy", "noun")],
        "A \\` mark, the deploy and `x` failed.\n",
    );
    assert_eq!(matched(&doc), ["deploy"]);
}

#[test]
fn raw_html_blocks_in_markdown_are_swept() {
    let source = "Intro.\n\n<table>\n<tr>\n<td>We request data</td>\n</tr>\n</table>\n\n\
                  <p align=\"center\">\n    <em>It is easy to learn</em>\n</p>\n";
    let doc = lint(&[("request", "verb"), ("easy", "adjective")], source);
    let at: Vec<(usize, usize, &str)> = doc
        .findings
        .iter()
        .map(|f| (f.start.line(), f.start.column(), f.matched_text.as_str()))
        .collect();
    assert_eq!(at, [(5, 8, "request"), (10, 15, "easy")]);
}

#[test]
fn a_raw_html_block_in_a_block_quote_keeps_exact_positions() {
    let source = "> <td>We request data</td>\n";
    let doc = lint(&[("request", "verb")], source);
    assert_eq!(doc.findings.len(), 1);
    let span = doc.findings[0].extent.unwrap().span;
    assert_eq!(&source[span.start()..span.end()], "request");
}

#[test]
fn fallback_owners_report_a_blocklist_message_without_open_placeholders() {
    // The shipped vocabulary rules, with the preferred owner and then the next ones off.
    let core = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../slopvac-rules/tests/fixtures/core-v3");
    let owner = "ste-words.word-outside-controlled-vocabulary";
    let wrong = "ste-words.word-used-in-wrong-part-of-speech";
    let forms = "ste-words.verb-or-adjective-form-not-permitted";
    let off_sets: [&[&str]; 3] = [&[owner], &[owner, wrong], &[owner, wrong, forms]];
    for off in off_sets {
        let set = slopvac_rules::RuleSet::load_dir(
            &slopvac_model::PackId::new("core").unwrap(),
            &core,
            &slopvac_rules::CompileOptions::default(),
        )
        .unwrap();
        let mut s = settings();
        for name in off {
            s.rule_levels.insert(
                common::id(&format!("core:{name}")),
                slopvac_engine::Level::Off,
            );
        }
        let engine = Engine::new(vec![set], s)
            .unwrap()
            .with_blocklist(&list(&[("deploy", "noun")]))
            .unwrap();
        let doc = engine
            .lint(&Input::markdown("a.md", "The deploy failed.\n"))
            .unwrap();
        let sweep: Vec<_> = doc
            .findings
            .iter()
            .filter(|f| f.matched_text == "deploy" && f.message.as_str().contains("blocklist"))
            .collect();
        assert_eq!(sweep.len(), 1, "off: {off:?}");
        assert_eq!(
            sweep[0].message.as_str(),
            "\"deploy\" is on this project's word blocklist as a noun.",
            "off: {off:?}"
        );
    }
}
#[test]
fn a_multi_line_pre_in_a_block_quote_hides_its_lines() {
    // Each quoted line is its own source range. Read one by one, the middle line would
    // look like prose.
    let source = "> <pre>\n> We request data\n> </pre>\n";
    let doc = lint(&[("request", "verb")], source);
    assert!(doc.findings.is_empty(), "{:?}", matched(&doc));
}

#[test]
fn a_multi_line_comment_in_a_block_quote_hides_its_lines() {
    let source = "> <!--\n> We request data\n> -->\n";
    let doc = lint(&[("request", "verb")], source);
    assert!(doc.findings.is_empty(), "{:?}", matched(&doc));
}

#[test]
fn quoted_html_prose_keeps_its_sentence_across_lines_and_exact_positions() {
    let source = "> <p>Please\n> request\n> data</p>\n";
    let doc = lint(&[("request", "verb")], source);
    assert_eq!(doc.findings.len(), 1);
    let finding = &doc.findings[0];
    assert_eq!((finding.start.line(), finding.start.column()), (2, 3));
    let span = finding.extent.unwrap().span;
    assert_eq!(&source[span.start()..span.end()], "request");
}

#[test]
fn raw_html_in_a_list_item_is_swept_with_exact_positions() {
    let source = "- item\n\n  <p>Please\n  request data</p>\n";
    let doc = lint(&[("request", "verb")], source);
    assert_eq!(doc.findings.len(), 1);
    let span = doc.findings[0].extent.unwrap().span;
    assert_eq!(&source[span.start()..span.end()], "request");
}

#[test]
fn a_table_exclusion_does_not_reach_the_cells_of_an_html_table() {
    let source = "<table>\n<tr><td>Please request data</td></tr>\n</table>\n";
    let none = lint_with(
        &json!({"exclude_blocks": ["table"]}),
        &[("request", "verb")],
        source,
    );
    // 2.x reads the prose of a raw HTML block as a paragraph, not as table cells.
    assert_eq!(none.findings.len(), 1);
}

#[test]
fn an_escaped_backtick_stays_attached_to_its_word_as_in_vale() {
    // Vale tags nothing for `deploy`` here: the backtick keeps the word whole, so the
    // blocked noun does not match.
    let doc = lint(&[("deploy", "noun")], "The deploy\\` failed.\n");
    assert_eq!(matched(&doc), Vec::<&str>::new());
}
const QUOTED_HTML: &str = "> <p>Please request data</p>\n\n<p>Please request data</p>\n";

fn lines(doc: &Document) -> Vec<usize> {
    doc.findings.iter().map(|f| f.start.line()).collect()
}

#[test]
fn a_quote_exclusion_does_not_reach_raw_html_in_a_block_quote() {
    let doc = lint_with(
        &json!({"exclude_blocks": ["quote"]}),
        &[("request", "verb")],
        QUOTED_HTML,
    );
    assert_eq!(lines(&doc), [1, 3]);
}

#[test]
fn a_list_item_exclusion_does_not_reach_raw_html_in_a_list_item() {
    let source = "- item\n\n  <p>Please request data</p>\n\n<p>Please request data</p>\n";
    let doc = lint_with(
        &json!({"exclude_blocks": ["list-item"]}),
        &[("request", "verb")],
        source,
    );
    assert_eq!(lines(&doc), [3, 5]);
}

#[test]
fn a_quote_exclusion_does_not_reach_an_html_block_quote_inside_raw_html() {
    let source =
        "<blockquote><p>Please request data</p></blockquote>\n\n<p>Please request data</p>\n";
    let doc = lint_with(
        &json!({"exclude_blocks": ["quote"]}),
        &[("request", "verb")],
        source,
    );
    assert_eq!(lines(&doc), [1, 3]);
}

#[test]
fn a_list_item_exclusion_does_not_reach_an_html_list_inside_raw_html() {
    let source = "<ul><li>Please request data</li></ul>\n\n<p>Please request data</p>\n";
    let doc = lint_with(
        &json!({"exclude_blocks": ["list-item"]}),
        &[("request", "verb")],
        source,
    );
    assert_eq!(lines(&doc), [1, 3]);
}

#[test]
fn an_html_list_in_a_markdown_quote_is_a_plain_paragraph() {
    // 2.x holds the prose of a raw HTML block in no quote and no list item, whatever it is in.
    let source = "> <ul><li>Please request data</li></ul>\n\n<p>Please request data</p>\n";
    for kind in ["quote", "list-item"] {
        let doc = lint_with(
            &json!({"exclude_blocks": [kind]}),
            &[("request", "verb")],
            source,
        );
        assert_eq!(lines(&doc), [1, 3], "exclude_blocks {kind}");
    }
}

#[test]
fn context_gates_do_not_see_an_html_block_quote_inside_raw_html() {
    let source =
        "<blockquote><p>Please request data</p></blockquote>\n\n<p>Please request data</p>\n";
    let only = lint_with(
        &json!({"contexts": {"only": ["quote"]}}),
        &[("request", "verb")],
        source,
    );
    assert_eq!(lines(&only), Vec::<usize>::new());
}

#[test]
fn context_gates_do_not_see_raw_html_in_a_block_quote() {
    let skip = lint_with(
        &json!({"contexts": {"skip": ["quote"]}}),
        &[("request", "verb")],
        QUOTED_HTML,
    );
    assert_eq!(lines(&skip), [1, 3]);
    let only = lint_with(
        &json!({"contexts": {"only": ["quote"]}}),
        &[("request", "verb")],
        QUOTED_HTML,
    );
    assert_eq!(lines(&only), Vec::<usize>::new());
}

#[test]
fn a_tag_exception_detector_does_not_see_raw_html_in_a_block_quote() {
    // A detector with no registered implementation matches a context tag of that name.
    let doc = lint_with(
        &json!({"exceptions": [{"name": "quoted", "detector": "quote"}]}),
        &[("request", "verb")],
        QUOTED_HTML,
    );
    assert_eq!(lines(&doc), [1, 3]);
}
const SECTIONS: &str =
    "## Examples\n\n<p>Please request data</p>\n\n## Other\n\n<p>Please request data</p>\n";

#[test]
fn a_section_tag_covers_raw_html_that_ends_its_section() {
    // The Markdown parser has no block for the HTML, so the section "Examples" ends at its
    // heading. The tag still applies to the HTML under it.
    let skip = lint_with(
        &json!({"contexts": {"skip": ["example"]}}),
        &[("request", "verb")],
        SECTIONS,
    );
    assert_eq!(lines(&skip), [7]);
    let only = lint_with(
        &json!({"contexts": {"only": ["example"]}}),
        &[("request", "verb")],
        SECTIONS,
    );
    assert_eq!(lines(&only), [3]);
}

#[test]
fn a_tag_detector_sees_the_section_tag_of_raw_html() {
    let doc = lint_with(
        &json!({"exceptions": [{"name": "example", "detector": "example"}]}),
        &[("request", "verb")],
        SECTIONS,
    );
    assert_eq!(lines(&doc), [7]);
}

#[test]
fn a_document_tag_from_front_matter_covers_raw_html() {
    let generated = "---\ngenerated: \"true\"\n---\n\n<p>Please request data</p>\n";
    let plain = "---\ntitle: x\n---\n\n<p>Please request data</p>\n";
    let gate = json!({"contexts": {"skip": ["generated"]}});
    let none = lint_with(&gate, &[("request", "verb")], generated);
    assert!(none.findings.is_empty(), "{:?}", lines(&none));
    let some = lint_with(&gate, &[("request", "verb")], plain);
    assert_eq!(lines(&some), [5]);
}
