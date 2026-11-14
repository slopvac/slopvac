//! Registry, declarative tagger and gate behaviour through the public API.
#![allow(clippy::assert_is_empty)]

use slopvac_context::{
    BlockKind, ContextGate, Error, OwnedBlock, PlainDocument, TagRegistry, TaggerSpec,
};

fn tagger(yaml: &str) -> Box<dyn slopvac_context::Tagger> {
    Box::new(TaggerSpec::from_yaml(yaml).unwrap().compile().unwrap())
}

const DOC: &str = "# Guide\n\nIntro text here.\n\n## Breaking changes\n\nThe flag is gone. Use the new one.\n\n## Usage\n\nRun it.\n";

#[test]
fn section_scope_covers_the_heading_section_only() {
    let mut reg = TagRegistry::new();
    reg.register(tagger(
        "id: t\nkind: change-note\nscope: section\nwhen:\n  heading_path: breaking\n",
    ))
    .unwrap();
    let doc = PlainDocument::from_markdown("a.md", DOC);
    let idx = reg.tag_document(&doc);
    let start = DOC.find("Breaking changes").unwrap();
    let end = DOC.find("Use the new one.").unwrap() + "Use the new one.".len();
    assert_eq!(idx.tags().len(), 1);
    assert_eq!(idx.tags()[0].span, start..end);
    assert_eq!(idx.tags()[0].kind, "change-note");
}

#[test]
fn sentence_scope_tags_the_matching_sentence() {
    let mut reg = TagRegistry::new();
    reg.register(tagger(
        "id: t\nkind: requirement\nconfidence: 0.8\nwhen:\n  sentence: '^use '\n",
    ))
    .unwrap();
    let doc = PlainDocument::from_markdown("a.md", DOC);
    let idx = reg.tag_document(&doc);
    let s = DOC.find("Use the new one.").unwrap();
    assert_eq!(idx.tags().len(), 1);
    assert_eq!(idx.tags()[0].span, s..s + "Use the new one.".len());
    assert!((idx.tags()[0].confidence - 0.8).abs() < f32::EPSILON);
}

#[test]
fn document_scope_from_path_condition() {
    let mut reg = TagRegistry::new();
    reg.register(tagger(
        "id: t\nkind: change-note\nscope: document\nwhen:\n  path: '(^|/)CHANGELOG\\.md$'\n",
    ))
    .unwrap();
    let hit = reg.tag_document(&PlainDocument::from_markdown("docs/CHANGELOG.md", DOC));
    let miss = reg.tag_document(&PlainDocument::from_markdown("docs/guide.md", DOC));
    assert_eq!(hit.tags()[0].span, 0..DOC.trim_end().len());
    assert!(miss.tags().is_empty());
}

#[test]
fn frontmatter_condition_checks_the_value() {
    let src = "---\ntype: release-notes\n---\n# T\n\nBody.\n";
    let mut reg = TagRegistry::new();
    reg.register(tagger(
        "id: t\nkind: change-note\nscope: document\nwhen:\n  frontmatter:\n    key: type\n    matches: release\n",
    ))
    .unwrap();
    assert_eq!(
        reg.tag_document(&PlainDocument::from_markdown("a.md", src))
            .tags()
            .len(),
        1
    );
    let other = src.replace("release-notes", "guide");
    assert!(
        reg.tag_document(&PlainDocument::from_markdown("a.md", &other))
            .tags()
            .is_empty()
    );
}

#[test]
fn admonition_title_condition() {
    let mut doc = PlainDocument::from_markdown("a.md", "# T\n\nBody text.\n");
    doc.blocks.push(OwnedBlock {
        kind: BlockKind::Paragraph,
        span: 100..110,
        text: "Gone now.".into(),
        heading_path: vec!["T".into()],
        admonition_title: Some("Changed in 2.0".into()),
    });
    let mut reg = TagRegistry::new();
    reg.register(tagger(
        "id: t\nkind: change-note\nscope: paragraph\nwhen:\n  admonition_title: changed in\n",
    ))
    .unwrap();
    let idx = reg.tag_document(&doc);
    assert_eq!(idx.tags().len(), 1);
    assert_eq!(idx.tags()[0].span, 100..110);
}

#[test]
fn kinds_at_needs_the_tag_to_contain_the_span() {
    let mut reg = TagRegistry::new();
    reg.register(tagger(
        "id: t\nkind: change-note\nscope: section\nwhen:\n  heading_path: breaking\n",
    ))
    .unwrap();
    let idx = reg.tag_document(&PlainDocument::from_markdown("a.md", DOC));
    let inside = DOC.find("gone").unwrap();
    assert_eq!(idx.kinds_at(&(inside..inside + 4)), vec!["change-note"]);
    let outside = DOC.find("Run it").unwrap();
    assert!(idx.kinds_at(&(outside..outside + 3)).is_empty());
    assert_eq!(idx.kinds_seen(), vec!["change-note"]);
}

#[test]
fn duplicate_tagger_ids_are_rejected() {
    let y = "id: t\nkind: a\nwhen:\n  sentence: x\n";
    let mut reg = TagRegistry::new();
    reg.register(tagger(y)).unwrap();
    assert!(matches!(reg.register(tagger(y)), Err(Error::DuplicateTagger(id)) if id == "t"));
}

#[test]
fn invalid_specs_are_errors() {
    assert!(TaggerSpec::from_yaml("id: t\nkind: a\nwhen:\n  sentence: x\nbogus: 1\n").is_err());
    assert!(
        TaggerSpec::from_yaml("id: t\nkind: a\nwhen: {}\n")
            .unwrap()
            .compile()
            .is_err()
    );
    assert!(
        TaggerSpec::from_yaml("id: t\nkind: a\nconfidence: 2\nwhen:\n  sentence: x\n")
            .unwrap()
            .compile()
            .is_err()
    );
    assert!(matches!(
        TaggerSpec::from_yaml("id: t\nkind: a\nwhen:\n  sentence: '('\n")
            .unwrap()
            .compile(),
        Err(Error::Pattern { .. })
    ));
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
