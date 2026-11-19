//! Behaviour of the built-in taggers through the public API.
#![allow(clippy::assert_is_empty)]

use rstest::rstest;
use slopvac_context::{BlockKind, OwnedBlock, PlainDocument, builtin_registry};

fn kinds_for(path: &str, src: &str, needle: &str) -> Vec<String> {
    let doc = PlainDocument::from_markdown(path, src);
    let idx = builtin_registry().unwrap().tag_document(&doc).unwrap();
    let start = src
        .find(needle)
        .unwrap_or_else(|| panic!("`{needle}` not in source"));
    idx.kinds_at(&(start..start + needle.len()))
        .into_iter()
        .map(String::from)
        .collect()
}

fn has(kinds: &[String], kind: &str) -> bool {
    kinds.iter().any(|k| k == kind)
}

#[test]
fn requirement_sentences() {
    let cases = [
        ("You must set the token.", true),
        ("Never commit secrets.", true),
        ("Do not touch the config.", true),
        ("Pass --force only on CI.", true),
        ("Do not use `--force` here.", true),
        ("Skip the cache, but not `--force`.", true),
        ("The tool is fast.", false),
        ("It supports model-only mode.", false),
        ("The loader never stops.", false),
    ];
    for (sentence, want) in cases {
        let src = format!("# T\n\n{sentence}\n");
        let kinds = kinds_for("a.md", &src, sentence);
        assert_eq!(has(&kinds, "requirement"), want, "{sentence}: {kinds:?}");
    }
}

#[test]
fn enforced_invariant_sentences() {
    let cases = [
        ("The server rejects unsigned requests.", true),
        ("Writes are atomic because the file is renamed.", true),
        ("The server never rejects requests.", false),
        ("The server does not block requests.", false),
        ("The server is fast.", false),
    ];
    for (sentence, want) in cases {
        let src = format!("# T\n\n{sentence}\n");
        let kinds = kinds_for("a.md", &src, sentence);
        assert_eq!(
            has(&kinds, "enforced-invariant"),
            want,
            "{sentence}: {kinds:?}"
        );
    }
}

#[test]
fn change_note_by_heading_or_version() {
    let cases = [
        (
            "# Guide\n\n## Breaking changes\n\nThe flag is gone.\n",
            "The flag is gone.",
            true,
        ),
        (
            "# Log\n\n## 2.19.0 (2018-06-12)\n\nThe flag is gone.\n",
            "The flag is gone.",
            true,
        ),
        (
            "# Log\n\n## Version 2.2.0\n\nThe flag is gone.\n",
            "The flag is gone.",
            true,
        ),
        (
            "# Log\n\n### Fixed\n\nThe flag is gone.\n",
            "The flag is gone.",
            true,
        ),
        (
            "# Guide\n\nThe flag was removed in v2.0.\n",
            "The flag was removed in v2.0.",
            true,
        ),
        (
            "# Guide\n\n## Usage\n\nThe flag is gone.\n",
            "The flag is gone.",
            false,
        ),
        (
            "# Guide\n\n## Fixed-point maths\n\nThe flag is gone.\n",
            "The flag is gone.",
            false,
        ),
        (
            "# Guide\n\nIt waits 1.5 seconds.\n",
            "It waits 1.5 seconds.",
            false,
        ),
    ];
    for (src, needle, want) in cases {
        let kinds = kinds_for("a.md", src, needle);
        assert_eq!(has(&kinds, "change-note"), want, "{src:?}: {kinds:?}");
    }
}

#[test]
fn change_note_by_file_name() {
    let src = "# Project\n\nThe flag is gone.\n";
    assert!(has(
        &kinds_for("docs/CHANGELOG.md", src, "The flag"),
        "change-note"
    ));
    assert!(!has(
        &kinds_for("docs/guide.md", src, "The flag"),
        "change-note"
    ));
}

#[test]
fn change_note_covers_list_under_a_version_lead_in() {
    let src = "# T\n\nIn 2.0 these changed:\n\n- The flag is gone.\n- The mode is gone.\n\nOther text here.\n";
    assert!(has(
        &kinds_for("a.md", src, "The flag is gone."),
        "change-note"
    ));
    assert!(has(
        &kinds_for("a.md", src, "The mode is gone."),
        "change-note"
    ));
    assert!(!has(
        &kinds_for("a.md", src, "Other text here."),
        "change-note"
    ));
}

#[test]
fn structural_sections() {
    let cases = [
        ("# T\n\n## Install\n\nRun it.\n", "Run it.", "procedure"),
        ("# T\n\n## Examples\n\nRun it.\n", "Run it.", "example"),
        ("# T\n\n## Warnings\n\nRun it.\n", "Run it.", "safety"),
        (
            "# T\n\n## Generated reference\n\nRun it.\n",
            "Run it.",
            "generated",
        ),
        (
            "---\ngenerated: true\n---\n# T\n\nRun it.\n",
            "Run it.",
            "generated",
        ),
    ];
    for (src, needle, kind) in cases {
        let kinds = kinds_for("a.md", src, needle);
        assert!(has(&kinds, kind), "{src:?}: {kinds:?}");
    }
    assert!(kinds_for("a.md", "# T\n\nRun it.\n", "Run it.").is_empty());
}

#[test]
fn admonition_and_quote_blocks() {
    let mut doc = PlainDocument::from_markdown("a.md", "# T\n\nBody text.\n");
    let mk = |kind, span, title: Option<&str>| OwnedBlock {
        kind,
        span,
        text: "x".repeat(10),
        heading_path: vec!["T".into()],
        heading_level: None,
        admonition_title: title.map(String::from),
    };
    doc.blocks
        .push(mk(BlockKind::Paragraph, 100..110, Some("Warning")));
    doc.blocks
        .push(mk(BlockKind::Paragraph, 200..210, Some("Note")));
    doc.blocks.push(mk(BlockKind::Quote, 300..310, None));
    doc.blocks
        .push(mk(BlockKind::Paragraph, 400..410, Some("Changed in 2.0")));
    let idx = builtin_registry().unwrap().tag_document(&doc).unwrap();
    assert_eq!(idx.kinds_at(&(100..110)), vec!["safety"]);
    assert!(idx.kinds_at(&(200..210)).is_empty());
    assert_eq!(idx.kinds_at(&(300..310)), vec!["quote"]);
    assert_eq!(idx.kinds_at(&(400..410)), vec!["change-note"]);
}

#[test]
fn code_spans_of_any_backtick_length_hide_their_words_from_the_taggers() {
    let cases = [
        ("Set ``must`` to true.", false),
        ("Set ``a `must` b`` to true.", false),
        ("Set `must` to true.", false),
        ("You must set ``x`` first.", true),
        ("The server ``rejects`` nothing.", false),
        ("Do not use `` `--force` `` here.", true),
    ];
    for (sentence, want) in cases {
        let src = format!("# T\n\n{sentence}\n");
        let kinds = kinds_for("a.md", &src, sentence);
        let got = has(&kinds, "requirement") || has(&kinds, "enforced-invariant");
        assert_eq!(got, want, "{sentence}: {kinds:?}");
    }
}

#[test]
fn example_admonition_tags_only_example_titles() {
    let mut doc = PlainDocument::from_markdown("a.md", "# T\n\nBody text.\n");
    for (span, title) in [
        (100..110, "Example: usage"),
        (200..210, "Note"),
        (300..310, "Counterexample"),
    ] {
        doc.blocks.push(OwnedBlock {
            kind: BlockKind::Paragraph,
            span,
            text: "x".repeat(10),
            heading_path: vec!["T".into()],
            heading_level: None,
            admonition_title: Some(title.into()),
        });
    }
    let idx = builtin_registry().unwrap().tag_document(&doc).unwrap();
    assert_eq!(idx.kinds_at(&(100..110)), vec!["example"]);
    assert!(idx.kinds_at(&(200..210)).is_empty());
    assert!(idx.kinds_at(&(300..310)).is_empty());
}

#[test]
fn generated_notice_tags_its_paragraph_only() {
    let src = "# T\n\nThis file is generated by the build. Do not edit.\n\nYou can generate files with the tool.\n";
    let notice = kinds_for("a.md", src, "This file is generated by the build.");
    assert!(has(&notice, "generated"), "{notice:?}");
    let other = kinds_for("a.md", src, "You can generate files with the tool.");
    assert!(!has(&other, "generated"), "{other:?}");
}

#[test]
fn generated_notice_also_covers_list_items_and_headings() {
    let src = "# This file is generated\n\n- Do not edit this by hand.\n- Plain item.\n";
    let heading = kinds_for("a.md", src, "This file is generated");
    assert!(has(&heading, "generated"), "{heading:?}");
    let item = kinds_for("a.md", src, "Do not edit this by hand.");
    assert!(has(&item, "generated"), "{item:?}");
    let plain = kinds_for("a.md", src, "Plain item.");
    assert!(!has(&plain, "generated"), "{plain:?}");
}

#[rstest]
#[case::emphasis_and_link_are_reduced_to_words(
    "**Never** pass `--force` to [the tool](http://x).",
    true
)]
#[case::a_marker_in_a_code_span_is_dropped("Use `must` here.", false)]
#[case::a_marker_in_a_link_target_is_dropped("See [docs](http://x/must) now.", false)]
#[case::an_unmatched_link_opener_keeps_the_text_after_it("Read a](b and you must stop.", true)]
#[case::an_unclosed_backtick_stays_literal("Use ` and you must stop.", true)]
#[case::a_shorter_run_does_not_close_a_longer_span("Use `` a ` must `` now.", false)]
#[case::an_unclosed_run_leaves_a_later_span_to_close("A ``` b `x` you must stop.", true)]
fn requirement_reads_the_cleaned_sentence(#[case] sentence: &str, #[case] want: bool) {
    let src = format!("# T\n\n{sentence}\n");
    let kinds = kinds_for("a.md", &src, sentence);
    assert_eq!(has(&kinds, "requirement"), want, "{kinds:?}");
}

#[rstest]
#[case::link_text_is_kept("The [rejects](http://x) page loads.", true)]
#[case::a_verb_in_a_code_span_is_dropped("The server `rejects` unsigned requests.", false)]
#[case::an_unmatched_link_opener_keeps_the_text_after_it("The server a](b rejects requests.", true)]
#[case::an_unclosed_backtick_stays_literal("The server ` rejects requests.", true)]
fn enforced_invariant_reads_the_cleaned_sentence(#[case] sentence: &str, #[case] want: bool) {
    let src = format!("# T\n\n{sentence}\n");
    let kinds = kinds_for("a.md", &src, sentence);
    assert_eq!(has(&kinds, "enforced-invariant"), want, "{kinds:?}");
}
