//! Registry, declarative tagger and gate behaviour through the public API.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(
    clippy::allow_attributes_without_reason,
    reason = "TEMPORARY: burn-down in slopvac-8l4.3.50"
)]
#![allow(clippy::assert_is_empty)]

use slopvac_context::{
    BlockKind, ContextGate, Error, OwnedBlock, PlainDocument, TagIndex, TagRegistry, Tagger,
    TaggerSpec,
};

fn tagger(yaml: &str) -> Box<dyn Tagger> {
    Box::new(TaggerSpec::from_yaml(yaml).unwrap().compile().unwrap())
}

fn run(yaml: &str, path: &str, src: &str) -> TagIndex {
    let mut reg = TagRegistry::new();
    reg.register(tagger(yaml)).unwrap();
    reg.tag_document(&PlainDocument::from_markdown(path, src))
        .unwrap()
}

const DOC: &str = "# Guide\n\nIntro text here.\n\n## Breaking changes\n\nThe flag is gone. Use the new one.\n\n## Usage\n\nRun it.\n";

#[test]
fn section_scope_covers_the_heading_section_only() {
    let idx = run(
        "schema_version: 3\nid: t\ntag: change-note\nscope: section\nwhen:\n  heading_path: breaking\n",
        "a.md",
        DOC,
    );
    let start = DOC.find("Breaking changes").unwrap();
    let end = DOC.find("Use the new one.").unwrap() + "Use the new one.".len();
    assert_eq!(idx.tags().len(), 1);
    assert_eq!(idx.tags()[0].span, start..end);
    assert_eq!(idx.tags()[0].kind, "change-note");
}

#[test]
fn section_scope_stops_at_the_next_peer_and_keeps_subsections() {
    let src = "# T\n\n## Notes\n\nA text.\n\n### Deep\n\nD text.\n\n## Other\n\nB text.\n\n## Notes\n\nC text.\n";
    let idx = run(
        "schema_version: 3\nid: t\ntag: k\nscope: section\nwhen:\n  heading_path: '^notes$'\n",
        "a.md",
        src,
    );
    let at = |s: &str| src.find(s).unwrap();
    assert_eq!(idx.kinds_at(&(at("A text.")..at("A text.") + 7)), vec!["k"]);
    assert_eq!(idx.kinds_at(&(at("D text.")..at("D text.") + 7)), vec!["k"]);
    assert!(idx.kinds_at(&(at("B text.")..at("B text.") + 7)).is_empty());
    assert_eq!(idx.kinds_at(&(at("C text.")..at("C text.") + 7)), vec!["k"]);
    let first_end = at("## Other") - 2;
    assert!(idx.tags().iter().any(|t| t.span.end == first_end));
}

#[test]
fn identically_named_sibling_headings_are_tagged_independently() {
    let src = "# T\n\n## Notes\n\nThe flag is gone.\n\n## Notes\n\nNothing else here.\n";
    let idx = run(
        "schema_version: 3\nid: t\ntag: k\nscope: section\nwhen:\n  heading_path: '^notes$'\n  sentence: flag\n",
        "a.md",
        src,
    );
    let first = src.find("The flag is gone.").unwrap();
    let second = src.find("Nothing else here.").unwrap();
    assert_eq!(idx.tags().len(), 1);
    assert_eq!(idx.kinds_at(&(first..first + 17)), vec!["k"]);
    assert!(idx.kinds_at(&(second..second + 18)).is_empty());
    let second_heading = src.rfind("## Notes").unwrap();
    assert!(idx.tags()[0].span.end <= second_heading);
}

#[test]
fn sentence_scope_tags_the_matching_sentence() {
    let idx = run(
        "schema_version: 3\nid: t\ntag: requirement\nconfidence: 0.8\nwhen:\n  sentence: '^use '\n",
        "a.md",
        DOC,
    );
    let s = DOC.find("Use the new one.").unwrap();
    assert_eq!(idx.tags().len(), 1);
    assert_eq!(idx.tags()[0].span, s..s + "Use the new one.".len());
    assert!((idx.tags()[0].confidence - 0.8).abs() < f32::EPSILON);
}

#[test]
fn document_scope_covers_the_whole_source_from_a_path_condition() {
    let y = "schema_version: 3\nid: t\ntag: change-note\nscope: document\nwhen:\n  path: '(^|/)CHANGELOG\\.md$'\n";
    let hit = run(y, "docs/CHANGELOG.md", DOC);
    let miss = run(y, "docs/guide.md", DOC);
    assert_eq!(hit.tags()[0].span, 0..DOC.len());
    assert!(miss.tags().is_empty());
}

#[test]
fn document_scope_with_only_document_conditions_needs_no_prose_block() {
    let y = "schema_version: 3\nid: t\ntag: generated\nscope: document\nwhen:\n  front_matter:\n    key: generated\n";
    for src in [
        "---\ngenerated: yes\n---\n",
        "---\ngenerated: yes\n---\n```\ncode\n```\n",
    ] {
        let idx = run(y, "a.md", src);
        assert_eq!(idx.tags().len(), 1, "{src:?}");
        assert_eq!(idx.tags()[0].span, 0..src.len());
    }
}

#[test]
fn frontmatter_condition_checks_the_value() {
    let y = "schema_version: 3\nid: t\ntag: change-note\nscope: document\nwhen:\n  front_matter:\n    key: type\n    matches: release\n";
    let src = "---\ntype: release-notes\n---\n# T\n\nBody.\n";
    assert_eq!(run(y, "a.md", src).tags().len(), 1);
    assert!(
        run(y, "a.md", &src.replace("release-notes", "guide"))
            .tags()
            .is_empty()
    );
}

#[test]
fn conditions_combine_with_and_across_document_block_and_sentence_levels() {
    let y = "schema_version: 3\nid: t\ntag: k\nwhen:\n  path: 'docs/'\n  heading_path: breaking\n  sentence: 'flag'\n";
    let sentence = "The flag is gone.";
    let at = DOC.find(sentence).unwrap();
    let span = at..at + sentence.len();
    // All three hold.
    assert_eq!(run(y, "docs/a.md", DOC).kinds_at(&span), vec!["k"]);
    // Each alone fails.
    assert!(run(y, "src/a.md", DOC).tags().is_empty(), "path fails");
    let other_heading = DOC.replace("Breaking changes", "Usage notes");
    assert!(
        run(y, "docs/a.md", &other_heading).tags().is_empty(),
        "heading fails"
    );
    let other_sentence = DOC.replace("The flag is gone.", "The mode is gone.");
    assert!(
        run(y, "docs/a.md", &other_sentence).tags().is_empty(),
        "sentence fails"
    );
    // Only the matching sentence of the matching block is tagged.
    assert_eq!(run(y, "docs/a.md", DOC).tags().len(), 1);
}

#[test]
fn admonition_title_condition() {
    let mut doc = PlainDocument::from_markdown("a.md", "# T\n\nBody text.\n");
    doc.blocks.push(OwnedBlock {
        kind: BlockKind::Paragraph,
        span: 100..110,
        text: "Gone now.".into(),
        heading_path: vec!["T".into()],
        heading_level: None,
        admonition_title: Some("Changed in 2.0".into()),
    });
    let mut reg = TagRegistry::new();
    reg.register(tagger(
        "schema_version: 3\nid: t\ntag: change-note\nscope: paragraph\nwhen:\n  admonition_title: changed in\n",
    ))
    .unwrap();
    let idx = reg.tag_document(&doc).unwrap();
    assert_eq!(idx.tags().len(), 1);
    assert_eq!(idx.tags()[0].span, 100..110);
}

#[test]
fn kinds_at_needs_the_tag_to_contain_the_span() {
    let idx = run(
        "schema_version: 3\nid: t\ntag: change-note\nscope: section\nwhen:\n  heading_path: breaking\n",
        "a.md",
        DOC,
    );
    let inside = DOC.find("gone").unwrap();
    assert_eq!(idx.kinds_at(&(inside..inside + 4)), vec!["change-note"]);
    let outside = DOC.find("Run it").unwrap();
    assert!(idx.kinds_at(&(outside..outside + 3)).is_empty());
    assert_eq!(idx.kinds_seen(), vec!["change-note"]);
}

#[test]
fn headings_with_extra_spaces_and_non_ascii_text_keep_exact_spans() {
    let src = "#   Überschrift ##\n\nText.\n";
    let doc = PlainDocument::from_markdown("a.md", src);
    let heading = &slopvac_context::DocumentView::blocks(&doc)[0];
    assert_eq!(heading.text, "Überschrift");
    assert_eq!(&src[heading.span.clone()], "Überschrift");
    assert_eq!(heading.heading_level, Some(1));
}

#[test]
fn duplicate_tagger_ids_are_rejected() {
    let y = "schema_version: 3\nid: t\ntag: a\nwhen:\n  sentence: x\n";
    let mut reg = TagRegistry::new();
    reg.register(tagger(y)).unwrap();
    assert!(matches!(reg.register(tagger(y)), Err(Error::DuplicateTagger(id)) if id == "t"));
}

#[test]
fn invalid_specs_are_errors() {
    let compile = |y: &str| TaggerSpec::from_yaml(y).and_then(TaggerSpec::compile);
    let cases = [
        "schema_version: 3\nid: t\ntag: a\nwhen:\n  sentence: x\nbogus: 1\n",
        "schema_version: 3\nid: t\ntag: a\nwhen: {}\n",
        "schema_version: 3\nid: t\ntag: a\nconfidence: 2\nwhen:\n  sentence: x\n",
        "schema_version: 3\nid: t\ntag: Change Note\nwhen:\n  sentence: x\n",
        "schema_version: 3\nid: t\ntag: '1a'\nwhen:\n  sentence: x\n",
        "schema_version: 3\nid: t\ntag: a_b\nwhen:\n  sentence: x\n",
        "schema_version: 3\nid: t\ntag: ''\nwhen:\n  sentence: x\n",
    ];
    for y in cases {
        assert!(compile(y).is_err(), "{y}");
    }
    assert!(matches!(
        compile("schema_version: 3\nid: t\ntag: a\nwhen:\n  sentence: '('\n"),
        Err(Error::Pattern { .. })
    ));
}

#[test]
fn pcre2_execution_errors_surface_instead_of_missing_tags() {
    let y = "schema_version: 3\nid: t\ntag: k\nwhen:\n  sentence: '^(a+)+$'\n";
    let src = format!("# T\n\n{}b.\n", "a".repeat(64));
    let mut reg = TagRegistry::new();
    reg.register(tagger(y)).unwrap();
    let err = reg
        .tag_document(&PlainDocument::from_markdown("a.md", &src))
        .unwrap_err();
    assert!(matches!(err, Error::Match { .. }), "{err:?}");
}

#[test]
fn context_gate_semantics() {
    let gate = |only: &[&str], skip: &[&str]| ContextGate {
        only: only.iter().map(|s| (*s).into()).collect(),
        skip: skip.iter().map(|s| (*s).into()).collect(),
    };
    let cases: [(ContextGate, &[&str], bool); 6] = [
        (gate(&[], &[]), &[], true),
        (gate(&[], &["change-note"]), &["change-note"], false),
        (gate(&[], &["change-note"]), &["requirement"], true),
        (gate(&["procedure"], &[]), &[], false),
        (gate(&["procedure"], &[]), &["procedure"], true),
        (
            gate(&["procedure"], &["example"]),
            &["procedure", "example"],
            false,
        ),
    ];
    for (g, kinds, want) in cases {
        assert_eq!(g.allows(kinds), want, "{g:?} on {kinds:?}");
    }
}

fn piece(kind: &str, span: std::ops::Range<usize>, group: usize) -> (slopvac_context::Tag, usize) {
    (
        slopvac_context::Tag {
            kind: kind.to_owned(),
            span,
            confidence: 1.0,
            evidence: String::new(),
        },
        group,
    )
}

#[test]
#[allow(clippy::single_range_in_vec_init)]
fn ranges_on_different_pieces_of_one_tag_are_covered_by_it() {
    // One tag in two pieces (two lines of a cell), and an unrelated tag of the same kind.
    let idx = TagIndex::from_pieces(vec![
        piece("requirement", 2..14, 0),
        piece("requirement", 40..57, 0),
        piece("requirement", 80..90, 1),
    ]);
    assert_eq!(idx.kinds_covering(&[10..14, 40..46]), ["requirement"]);
    // A range inside the other tag's piece is not covered by the first tag's pieces.
    assert!(idx.kinds_covering(&[10..14, 80..85]).is_empty());
    // A range between the pieces is covered by none.
    assert!(idx.kinds_covering(&[10..14, 20..25]).is_empty());
    // One range is the plain containment check.
    assert_eq!(idx.kinds_covering(&[3..5]), idx.kinds_at(&(3..5)));
    assert!(idx.kinds_covering(&[]).is_empty());
}

fn block(
    kind: BlockKind,
    span: std::ops::Range<usize>,
    text: &str,
    level: Option<u8>,
) -> OwnedBlock {
    OwnedBlock {
        kind,
        span,
        text: text.to_owned(),
        heading_path: vec!["Procedure".to_owned()],
        heading_level: level,
        admonition_title: None,
    }
}

#[test]
fn a_section_runs_to_the_furthest_end_of_its_blocks() {
    // Reading order is not source order: the second cell ends before the first one does.
    let doc = PlainDocument {
        path: "a.rst".to_owned(),
        source_len: 200,
        blocks: vec![
            block(BlockKind::Heading, 0..9, "Procedure", Some(1)),
            block(BlockKind::TableRow, 50..120, "left cell", None),
            block(BlockKind::TableRow, 70..90, "right cell", None),
        ],
        ..PlainDocument::default()
    };
    let mut reg = TagRegistry::new();
    reg.register(tagger(
        "schema_version: 3\nid: t\ntag: procedure\nscope: section\nwhen:\n  heading_path: procedure\n",
    ))
    .unwrap();
    let idx = reg.tag_document(&doc).unwrap();
    assert_eq!(idx.tags().len(), 1);
    assert_eq!(idx.tags()[0].span, 0..120);
}

#[test]
#[allow(clippy::single_range_in_vec_init)]
fn overlapping_groups_keep_every_piece_they_share() {
    // Group 0 is one piece. Group 1 has the same first piece and a second one.
    let idx = TagIndex::from_pieces(vec![
        piece("requirement", 2..14, 0),
        piece("requirement", 2..14, 1),
        piece("requirement", 40..57, 1),
    ]);
    assert_eq!(idx.kinds_covering(&[10..14, 40..46]), ["requirement"]);
    // Group 0 alone does not reach the second range.
    assert!(idx.kinds_covering(&[10..14, 60..65]).is_empty());
    // Equal pieces of one group are still one piece.
    let same = TagIndex::from_pieces(vec![
        piece("requirement", 2..14, 0),
        piece("requirement", 2..14, 0),
    ]);
    assert_eq!(same.tags().len(), 1);
}

#[test]
fn windows_style_paths_match_path_conditions_like_unix_paths() {
    let y = "schema_version: 3\nid: t\ntag: change-note\nscope: document\nwhen:\n  path: '^docs/CHANGELOG\\.md$'\n";
    let idx = run(y, "C:\\docs\\CHANGELOG.md", DOC);
    assert_eq!(idx.tags().len(), 1);
    assert_eq!(idx.tags()[0].kind, "change-note");
    assert_eq!(idx.tags()[0].span, 0..DOC.len());
}

#[test]
fn dot_components_are_dropped_before_an_anchored_path_condition() {
    let y = "schema_version: 3\nid: t\ntag: change-note\nscope: document\nwhen:\n  path: '^docs/CHANGELOG\\.md$'\n";
    let idx = run(y, "./docs/./CHANGELOG.md", DOC);
    assert_eq!(idx.tags().len(), 1);
    assert_eq!(idx.tags()[0].kind, "change-note");
    assert_eq!(idx.tags()[0].span, 0..DOC.len());
}

#[test]
fn a_structural_tagger_reports_the_evidence_it_was_given() {
    let idx = run(
        "schema_version: 3\nid: t\ntag: k\nevidence: Says flag.\nwhen:\n  sentence: flag\n",
        "a.md",
        DOC,
    );
    assert_eq!(idx.tags().len(), 1);
    assert_eq!(idx.tags()[0].evidence, "Says flag.");
}

#[test]
fn a_structural_tagger_without_evidence_names_itself_and_its_scope() {
    let idx = run(
        "schema_version: 3\nid: t\ntag: k\nwhen:\n  sentence: flag\n",
        "a.md",
        DOC,
    );
    assert_eq!(idx.tags()[0].evidence, "t: sentence");
}

#[rstest::rstest]
#[case::block_condition_only("heading_path: '^notes$'", &["Notes", "First one.", "Second one."])]
#[case::sentence_condition("heading_path: '^notes$'\n  sentence: first", &["First one."])]
fn sentence_scope_tags_each_sentence_not_the_whole_block(
    #[case] when: &str,
    #[case] expected: &[&str],
) {
    let src = "# T\n\n## Notes\n\nFirst one. Second one.\n";
    let yaml = format!("schema_version: 3\nid: t\ntag: k\nscope: sentence\nwhen:\n  {when}\n");
    let idx = run(&yaml, "a.md", src);
    let spans: Vec<&str> = idx.tags().iter().map(|t| &src[t.span.clone()]).collect();
    assert_eq!(spans, expected);
}
