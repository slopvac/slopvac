//! Behaviour of the built-in taggers through the public API.
#![allow(clippy::assert_is_empty)]

use slopvac_context::{BlockKind, OwnedBlock, PlainDocument, builtin_registry};

fn kinds_for(path: &str, src: &str, needle: &str) -> Vec<String> {
    let doc = PlainDocument::from_markdown(path, src);
    let idx = builtin_registry().unwrap().tag_document(&doc);
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
        admonition_title: title.map(String::from),
    };
    doc.blocks
        .push(mk(BlockKind::Paragraph, 100..110, Some("Warning")));
    doc.blocks
        .push(mk(BlockKind::Paragraph, 200..210, Some("Note")));
    doc.blocks.push(mk(BlockKind::Quote, 300..310, None));
    doc.blocks
        .push(mk(BlockKind::Paragraph, 400..410, Some("Changed in 2.0")));
    let idx = builtin_registry().unwrap().tag_document(&doc);
    assert_eq!(idx.kinds_at(&(100..110)), vec!["safety"]);
    assert!(idx.kinds_at(&(200..210)).is_empty());
    assert_eq!(idx.kinds_at(&(300..310)), vec!["quote"]);
    assert_eq!(idx.kinds_at(&(400..410)), vec!["change-note"]);
}
