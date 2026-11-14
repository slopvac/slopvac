//! Genre registry behaviour through the public API.
#![allow(clippy::assert_is_empty)]

use slopvac_context::{
    Error, GenreAssignment, GenreRegistry, GenreSource, PlainDocument, ResolveOptions,
    builtin_genres, parse_genres,
};

fn resolve(path: &str, src: &str) -> slopvac_context::GenreResult {
    builtin_genres()
        .unwrap()
        .resolve(
            &PlainDocument::from_markdown(path, src),
            &ResolveOptions::default(),
        )
        .unwrap()
}

#[test]
fn path_signal_detects_change_comms() {
    let r = resolve("docs/CHANGELOG.md", "# Project\n\nText here.\n");
    assert_eq!(r.id, "change-comms");
    assert_eq!(r.source, GenreSource::Detected);
    assert_eq!(r.reasons.len(), 1);
    assert_eq!(r.reasons[0].signal, "path");
    assert_eq!(r.reasons[0].detail, "path `docs/CHANGELOG.md` (+3)");
}

#[test]
fn heading_signal_reaches_the_threshold_exactly() {
    let r = resolve("x.md", "# Notes\n\n## Release notes\n\nText here.\n");
    assert_eq!(
        (r.id.as_str(), r.source),
        ("change-comms", GenreSource::Detected)
    );
    assert_eq!(r.reasons[0].detail, "heading `Release notes` (+3)");
}

#[test]
fn a_signal_counts_once_however_many_headings_match() {
    let r = resolve("x.md", "# T\n\n## Context\n\nA.\n\n## Decision\n\nB.\n");
    assert_eq!(
        (r.id.as_str(), r.source),
        ("consumer", GenreSource::Default)
    );
    let internal = r.scores.iter().find(|(id, _)| id == "internal").unwrap();
    assert!((internal.1 - 1.5).abs() < f32::EPSILON);
}

#[test]
fn signals_add_up_across_definitions() {
    let r = resolve("CONTRIBUTING.md", "# T\n\n## Context\n\nA.\n");
    assert_eq!(
        (r.id.as_str(), r.source),
        ("internal", GenreSource::Detected)
    );
    assert_eq!(r.reasons.len(), 2);
}

#[test]
fn below_threshold_falls_back_to_the_default() {
    let r = resolve("README.md", "# T\n\nBody.\n");
    assert_eq!(
        (r.id.as_str(), r.source),
        ("consumer", GenreSource::Default)
    );
    assert_eq!(r.reasons[0].signal, "default");
    let consumer = r.scores.iter().find(|(id, _)| id == "consumer").unwrap();
    assert!((consumer.1 - 2.0).abs() < f32::EPSILON);
}

#[test]
fn adr_path_detects_internal() {
    assert_eq!(
        resolve("docs/adr/0001-choice.md", "# T\n\nBody.\n").id,
        "internal"
    );
}

#[test]
fn front_matter_beats_detection() {
    let r = resolve("CHANGELOG.md", "---\ngenre: reference\n---\n# T\n\nBody.\n");
    assert_eq!(
        (r.id.as_str(), r.source),
        ("reference", GenreSource::Explicit)
    );
    assert_eq!(r.reasons[0].signal, "front-matter");
}

#[test]
fn assignments_apply_in_file_order_and_the_last_match_wins() {
    let reg = builtin_genres().unwrap();
    let doc = PlainDocument::from_markdown("docs/api/a.md", "# T\n\nBody.\n");
    let assignments = [
        GenreAssignment {
            files: vec!["docs/**".into()],
            genre: "consumer".into(),
        },
        GenreAssignment {
            files: vec!["docs/api/*.md".into()],
            genre: "reference".into(),
        },
        GenreAssignment {
            files: vec!["other/**".into()],
            genre: "informal".into(),
        },
    ];
    let r = reg
        .resolve(
            &doc,
            &ResolveOptions {
                assignments: &assignments,
                relative_path: None,
            },
        )
        .unwrap();
    assert_eq!(
        (r.id.as_str(), r.source),
        ("reference", GenreSource::Explicit)
    );
    assert_eq!(r.reasons[0].signal, "path-glob");
    assert_eq!(
        r.reasons[0].detail,
        "`[[genres]]` block 1 matches `docs/api/a.md`"
    );
}

#[test]
fn assignments_use_the_relative_path() {
    let reg = builtin_genres().unwrap();
    let doc = PlainDocument::from_markdown("/abs/proj/docs/a.md", "# T\n\nBody.\n");
    let assignments = [GenreAssignment {
        files: vec!["docs/*.md".into()],
        genre: "internal".into(),
    }];
    let r = reg
        .resolve(
            &doc,
            &ResolveOptions {
                assignments: &assignments,
                relative_path: Some("docs/a.md"),
            },
        )
        .unwrap();
    assert_eq!(r.id, "internal");
}

#[test]
fn unknown_genre_is_an_error() {
    let reg = builtin_genres().unwrap();
    let doc = PlainDocument::from_markdown("a.md", "---\ngenre: nope\n---\n# T\n\nBody.\n");
    assert!(matches!(
        reg.resolve(&doc, &ResolveOptions::default()),
        Err(Error::UnknownGenre(id)) if id == "nope"
    ));
}

const CHAIN: &str = "
- id: base
  abstract: true
  profile: strict
  tolerances: { prose-craft.future-tense: warning, prose-craft.hedging: error }
  contexts: [security]
  detect:
    - { path: 'secret', weight: 9 }
- id: child
  extends: base
  tolerances: { prose-craft.future-tense: off }
  contexts: [security, example]
";

#[test]
fn extends_merges_tolerances_contexts_and_profile() {
    let reg = GenreRegistry::new(parse_genres(CHAIN).unwrap(), "child").unwrap();
    let tol = reg.tolerances("child").unwrap();
    assert_eq!(tol["prose-craft.future-tense"], "off");
    assert_eq!(tol["prose-craft.hedging"], "error");
    assert_eq!(reg.contexts("child").unwrap(), vec!["security", "example"]);
    assert_eq!(reg.profile("child").unwrap().as_deref(), Some("strict"));
}

#[test]
fn abstract_genres_are_not_detected() {
    let reg = GenreRegistry::new(parse_genres(CHAIN).unwrap(), "child").unwrap();
    let doc = PlainDocument::from_markdown("secret.md", "# T\n\nBody.\n");
    let r = reg.resolve(&doc, &ResolveOptions::default()).unwrap();
    assert_eq!((r.id.as_str(), r.source), ("child", GenreSource::Default));
}

#[test]
fn invalid_definitions_are_errors() {
    let bad = [
        "- {id: a}\n- {id: a}\n",
        "- {id: a, extends: ghost}\n",
        "- {id: a, extends: b}\n- {id: b, extends: a}\n",
        "- {id: a, tolerances: {r.x: loud}}\n",
        "- {id: a, detect: [{weight: 1}]}\n",
        "- {id: a, detect: [{path: x, heading: y, weight: 1}]}\n",
        "- {id: a, detect: [{path: x, weight: 0}]}\n",
        "- {id: a, detect: [{path: '(', weight: 1}]}\n",
    ];
    for y in bad {
        let r = parse_genres(y).and_then(|s| GenreRegistry::new(s, "a"));
        assert!(r.is_err(), "{y}");
    }
}

#[test]
fn render_lists_reasons_and_scores() {
    let r = resolve("docs/CHANGELOG.md", "# Project\n\nText here.\n");
    let text = slopvac_context::render("docs/CHANGELOG.md", &r);
    assert!(text.starts_with("docs/CHANGELOG.md: change-comms (detected)\n  path: path `docs/CHANGELOG.md` (+3)\n  scores: change-comms 3,"), "{text}");
}

#[test]
fn genre_command_parses_files_and_assignments() {
    let m = slopvac_context::command()
        .try_get_matches_from(["genre", "a.md", "b.md", "--assign", "docs/**=reference"])
        .unwrap();
    assert_eq!(m.get_many::<String>("files").unwrap().count(), 2);
    assert!(
        slopvac_context::command()
            .try_get_matches_from(["genre"])
            .is_err()
    );
}
