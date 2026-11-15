//! Declarative tagger behaviour (`tagger.schema.json`) through the public API.
#![allow(clippy::assert_is_empty)]

use slopvac_context::{PlainDocument, TagRegistry, Tagger, TaggerSpec};

fn spec(yaml: &str) -> Result<Box<dyn Tagger>, slopvac_context::Error> {
    Ok(Box::new(TaggerSpec::from_yaml(yaml)?.compile()?))
}

/// The text each tag covers, in document order.
fn tagged(yaml: &str, src: &str) -> Vec<String> {
    let mut reg = TagRegistry::new();
    reg.register(spec(yaml).unwrap()).unwrap();
    let idx = reg
        .tag_document(&PlainDocument::from_markdown("a.md", src))
        .unwrap();
    idx.tags()
        .iter()
        .map(|t| src[t.span.clone()].to_string())
        .collect()
}

const HEAD: &str = "schema_version: 3\nid: t\ntag: k\n";

#[test]
fn a_sentence_unit_tags_each_matching_sentence() {
    let y = format!("{HEAD}match:\n  - pattern: '\\bmust\\b'\n");
    let got = tagged(&y, "# T\n\nIt must run. It may stop. You must wait.\n");
    assert_eq!(got, ["It must run.", "You must wait."]);
}

#[test]
fn a_paragraph_unit_tags_the_whole_paragraph() {
    let y = format!("{HEAD}unit: paragraph\nmatch:\n  - pattern: 'stop'\n");
    let got = tagged(&y, "# T\n\nIt must run. It may stop.\n\nOther text.\n");
    assert_eq!(got, ["It must run. It may stop."]);
}

#[test]
fn a_heading_unit_reads_headings_only() {
    let y = format!("{HEAD}unit: heading\nmatch:\n  - pattern: 'notes'\n");
    let got = tagged(&y, "# Release notes\n\nSee the notes.\n");
    assert_eq!(got, ["Release notes"]);
}

#[test]
fn an_unless_entry_silences_the_unit() {
    let y = format!("{HEAD}match:\n  - pattern: 'must'\nunless:\n  - pattern: '^note:'\n");
    let got = tagged(&y, "# T\n\nNote: it must run. It must stop.\n");
    assert_eq!(got, ["It must stop."]);
}

#[test]
fn matching_ignores_case_unless_the_matcher_is_case_sensitive() {
    let loose = format!("{HEAD}match:\n  - pattern: 'warning'\n");
    let strict = format!("{HEAD}match:\n  - pattern: 'warning'\n    case_sensitive: true\n");
    let src = "# T\n\nWarning here. warning there.\n";
    assert_eq!(tagged(&loose, src).len(), 2);
    assert_eq!(tagged(&strict, src), ["warning there."]);
}

#[test]
fn words_match_whole_words_and_phrases_only() {
    let y = format!("{HEAD}match:\n  - words: ['must', 'in order to', 'c++']\n");
    let src = "# T\n\nIt must run. Mustard is food. Act in order to win. Learn C++ now. Act in order too.\n";
    assert_eq!(
        tagged(&y, src),
        ["It must run.", "Act in order to win.", "Learn C++ now."]
    );
}

#[test]
fn the_tag_carries_the_confidence_and_the_evidence() {
    let y = format!("{HEAD}confidence: 0.4\nevidence: Says must.\nmatch:\n  - pattern: 'must'\n");
    let mut reg = TagRegistry::new();
    reg.register(spec(&y).unwrap()).unwrap();
    let doc = PlainDocument::from_markdown("a.md", "# T\n\nIt must run.\n");
    let idx = reg.tag_document(&doc).unwrap();
    assert_eq!(idx.tags()[0].kind, "k");
    assert!((idx.tags()[0].confidence - 0.4).abs() < f32::EPSILON);
    assert_eq!(idx.tags()[0].evidence, "Says must.");
}

#[test]
fn x_keys_are_ignored_and_other_unknown_keys_are_errors() {
    let ok = format!("{HEAD}x-owner: docs\nmatch:\n  - pattern: a\n    x-why: b\n");
    assert!(spec(&ok).is_ok());
    let top = format!("{HEAD}colour: red\nmatch:\n  - pattern: a\n");
    let inner = format!("{HEAD}match:\n  - pattern: a\n    colour: red\n");
    for y in [top, inner] {
        let err = spec(&y).err().expect("unknown key").to_string();
        assert!(err.contains("unknown field `colour`"), "{err}");
    }
}

#[test]
fn a_wrong_schema_version_is_an_error() {
    let y = "schema_version: 2\nid: t\ntag: k\nmatch:\n  - pattern: a\n";
    assert!(spec(y).is_err());
}
