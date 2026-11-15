//! Content-type genres: ui-copy, agent-instructions and the Diataxis types.
#![allow(clippy::assert_is_empty)]

use slopvac_context::{GenreSource, PlainDocument, ResolveOptions, builtin_genres};

fn genre_of(path: &str, src: &str) -> (String, GenreSource) {
    let r = builtin_genres()
        .unwrap()
        .resolve(
            &PlainDocument::from_markdown(path, src),
            &ResolveOptions::default(),
        )
        .unwrap();
    (r.id, r.source)
}

#[test]
fn genres_detect_on_fixtures() {
    let cases = [
        (
            "AGENTS.md",
            "# Rules\n\nYou MUST run tests.\n",
            "agent-instructions",
        ),
        (
            "CLAUDE.md",
            "# Rules\n\nNEVER push to main.\n",
            "agent-instructions",
        ),
        (
            ".cursor/rules/style.md",
            "# R\n\nBody.\n",
            "agent-instructions",
        ),
        (
            "strings/errors.md",
            "# Errors\n\nFile not found.\n",
            "ui-copy",
        ),
        ("---\ntype: ui-copy\n---\n", "", "ui-copy"),
        ("docs/tutorials/first.md", "# First\n\nBody.\n", "tutorial"),
        ("docs/how-to/deploy.md", "# Deploy\n\nBody.\n", "how-to"),
        ("docs/guide.md", "# How to deploy\n\nBody.\n", "how-to"),
        ("docs/cli/run.md", "# Run\n\nBody.\n", "reference-page"),
        (
            "docs/concepts/cache.md",
            "# Cache\n\nBody.\n",
            "explanation",
        ),
    ];
    for (path, src, want) in cases {
        let (path, src) = if path.starts_with("---") {
            ("notes.md", format!("{path}{src}# T\n\nBody.\n"))
        } else {
            (path, src.to_string())
        };
        assert_eq!(
            genre_of(path, &src),
            (want.to_string(), GenreSource::Detected),
            "{path}"
        );
    }
}

#[test]
fn content_types_extend_consumer_and_agent_instructions_extend_reference() {
    let reg = builtin_genres().unwrap();
    for id in ["tutorial", "how-to", "reference-page", "explanation"] {
        assert_eq!(reg.profile(id).unwrap().as_deref(), Some("normal"), "{id}");
        assert_eq!(reg.chain(id).unwrap()[0].id, "consumer");
    }
    assert_eq!(
        reg.profile("agent-instructions").unwrap().as_deref(),
        Some("strict")
    );
    assert_eq!(
        reg.tolerances("agent-instructions").unwrap()["prose-craft.negative-requirement"],
        "off"
    );
}

#[test]
fn tolerances_follow_the_content_type() {
    let reg = builtin_genres().unwrap();
    assert_eq!(
        reg.tolerances("tutorial").unwrap()["prose-craft.future-tense"],
        "off"
    );
    assert_eq!(
        reg.tolerances("explanation").unwrap()["prose-scope.contrastive-inversion-frames"],
        "off"
    );
    assert!(reg.tolerances("how-to").unwrap().is_empty());
}

#[test]
fn mixed_types_lists_every_content_type_the_page_reaches() {
    let reg = builtin_genres().unwrap();
    let mixed = PlainDocument::from_markdown(
        "docs/tutorials/deploy.md",
        "# Deploy\n\n## How to deploy\n\nSteps.\n",
    );
    let got = reg.mixed_types(&mixed, "consumer").unwrap();
    let ids: Vec<&str> = got.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, vec!["tutorial", "how-to"]);
    let single = PlainDocument::from_markdown("docs/tutorials/a.md", "# A\n\nBody.\n");
    assert_eq!(reg.mixed_types(&single, "consumer").unwrap().len(), 1);
}
