//! Declared contexts: security comes from a genre, never from vocabulary.
#![allow(clippy::assert_is_empty)]

use slopvac_context::{
    DeclaredContextTagger, GenreAssignment, PlainDocument, ResolveOptions, TagRegistry,
    builtin_genres, builtin_registry, context_hint,
};

const SECURITY_TEXT: &str = "# Policy\n\nTokens grant credentials. The sandbox denies secrets and approval escalates privileges.\n";

fn tags_for(doc: &PlainDocument, assignments: &[GenreAssignment]) -> (String, Vec<String>) {
    let genres = builtin_genres().unwrap();
    let result = genres
        .resolve(
            doc,
            &ResolveOptions {
                assignments,
                relative_path: None,
            },
        )
        .unwrap();
    let mut reg: TagRegistry = builtin_registry().unwrap();
    if let Some(t) = DeclaredContextTagger::for_genre(&genres, &result).unwrap() {
        reg.register(Box::new(t)).unwrap();
    }
    let idx = reg.tag_document(doc).unwrap();
    let kinds = idx.kinds_seen().into_iter().map(String::from).collect();
    (result.id, kinds)
}

#[test]
fn front_matter_genre_declares_security() {
    let src = format!("---\ngenre: security\n---\n{SECURITY_TEXT}");
    let (genre, kinds) = tags_for(&PlainDocument::from_markdown("a.md", &src), &[]);
    assert_eq!(genre, "security");
    assert!(kinds.contains(&"security".to_string()), "{kinds:?}");
}

#[test]
fn per_path_assignment_declares_security() {
    let a = [GenreAssignment {
        files: vec!["policies/**".into()],
        genre: "security".into(),
    }];
    let doc = PlainDocument::from_markdown("policies/access.md", SECURITY_TEXT);
    let (genre, kinds) = tags_for(&doc, &a);
    assert_eq!(genre, "security");
    assert!(kinds.contains(&"security".to_string()));
    let elsewhere = PlainDocument::from_markdown("docs/access.md", SECURITY_TEXT);
    assert!(!tags_for(&elsewhere, &a).1.contains(&"security".to_string()));
}

#[test]
fn security_vocabulary_alone_never_declares_security() {
    let doc = PlainDocument::from_markdown("docs/security/trust.md", SECURITY_TEXT);
    let (genre, kinds) = tags_for(&doc, &[]);
    assert_eq!(genre, "consumer");
    assert!(!kinds.contains(&"security".to_string()), "{kinds:?}");
}

#[test]
fn a_genre_without_contexts_adds_no_tagger() {
    let genres = builtin_genres().unwrap();
    let doc = PlainDocument::from_markdown("a.md", "# T\n\nBody.\n");
    let result = genres.resolve(&doc, &ResolveOptions::default()).unwrap();
    assert!(
        DeclaredContextTagger::for_genre(&genres, &result)
            .unwrap()
            .is_none()
    );
}

#[test]
fn security_context_carries_the_name_the_enforcement_hint() {
    let hint = context_hint("security").unwrap();
    assert!(hint.contains("Name the enforcement"));
    assert_eq!(context_hint("requirement"), None);
}

#[test]
fn security_span_covers_every_original_byte_including_front_matter() {
    let src = "---\ngenre: security\n---\n# Überblick\n\nDer Zugriff ist nur über Token erlaubt.\n";
    let doc = PlainDocument::from_markdown("a.md", src);
    let genres = builtin_genres().unwrap();
    let result = genres.resolve(&doc, &ResolveOptions::default()).unwrap();
    let mut reg = TagRegistry::new();
    reg.register(Box::new(
        DeclaredContextTagger::for_genre(&genres, &result)
            .unwrap()
            .unwrap(),
    ))
    .unwrap();
    let idx = reg.tag_document(&doc).unwrap();
    assert_eq!(idx.tags().len(), 1);
    assert_eq!(idx.tags()[0].kind, "security");
    assert_eq!(idx.tags()[0].span, 0..src.len());
    assert_eq!(idx.tags()[0].evidence, "declared by genre `security`");
    let last_char = src.len() - 1..src.len();
    assert_eq!(idx.kinds_at(&(0..1)), vec!["security"]);
    assert_eq!(idx.kinds_at(&last_char), vec!["security"]);
    assert_eq!(idx.kinds_at(&(src.len()..src.len())), vec!["security"]);
    assert!(idx.kinds_at(&(0..src.len() + 1)).is_empty());
}
