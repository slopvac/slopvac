//! A genre's declared contexts tag the whole document, so rules gated on a context see them.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

mod common;

use common::{category, ruleset, settings};
use rstest::rstest;
use serde_json::json;
use slopvac_context::GenreAssignment;
use slopvac_context::{DocumentView, Tag, TagRegistry, Tagger};
use slopvac_diagrams::Format;
use slopvac_engine::{Engine, EngineError, Input, Settings};
use slopvac_model::GenreId;

const BODY: &str = "We refactored the parser.\n";

/// A rule that fires on `refactored` only in a document with the `security` context.
fn gated_engine(settings: Settings) -> Engine {
    let only = json!({"contexts": {"only": ["security"]}});
    let rules = [common::tokens("gated", "refactored", &only)];
    Engine::new(vec![ruleset(vec![category("cat", &rules)])], settings).unwrap()
}

fn per_document() -> Settings {
    let mut s = settings();
    s.genre_assignments = Some(Vec::<GenreAssignment>::new());
    s
}

fn lint(engine: &Engine, genre: &str) -> slopvac_model::Document {
    let source = format!("---\ngenre: {genre}\n---\n{BODY}");
    engine
        .lint(&Input::markdown("a.md", &source).with_declared_genre(genre))
        .unwrap()
}

#[rstest]
#[case::declaring_genre("security", true)]
#[case::other_genre("reference", false)]
fn a_gated_rule_fires_only_when_the_genre_declares_the_context(
    #[case] genre: &str,
    #[case] fires: bool,
) {
    let doc = lint(&gated_engine(per_document()), genre);
    assert_eq!(
        doc.findings
            .iter()
            .any(|f| f.rule_id.as_str().ends_with("gated")),
        fires
    );
}

#[rstest]
#[case::declaring_genre("security", &["security"])]
#[case::other_genre("reference", &[])]
fn contexts_seen_lists_the_declared_context(#[case] genre: &str, #[case] seen: &[&str]) {
    let doc = lint(&gated_engine(per_document()), genre);
    let got: Vec<&str> = doc
        .contexts_seen
        .iter()
        .map(slopvac_model::ContextTag::as_str)
        .collect();
    assert_eq!(got, seen);
}

#[test]
fn the_genre_of_a_whole_run_declares_its_contexts_too() {
    let mut run = settings();
    run.genre.id = slopvac_model::GenreId::new("security").unwrap();
    let engine = gated_engine(run);
    let doc = engine.lint(&Input::markdown("a.md", BODY)).unwrap();
    assert!(
        doc.findings
            .iter()
            .any(|f| f.rule_id.as_str().ends_with("gated"))
    );
}

const LONG_LABEL_FILE: &str =
    "flowchart TD\n  A[This label has far too many words in it] --> B[End]\n";

fn security_run() -> Settings {
    let mut run = settings();
    run.genre.id = GenreId::new("security").unwrap();
    run
}

fn diagram_engine(settings: Settings) -> Engine {
    let only = json!({"contexts": {"only": ["security"]}});
    let rule = common::rule(
        "label-run-on",
        &json!({"kind": "structure", "check": "diagram-label-run-on"}),
        &only,
    );
    Engine::new(vec![ruleset(vec![category("diagram", &[rule])])], settings).unwrap()
}

#[rstest]
#[case::run_genre(security_run())]
#[case::per_document({
    let mut s = security_run();
    s.genre_assignments = Some(Vec::<GenreAssignment>::new());
    s
})]
fn a_standalone_diagram_sees_the_contexts_its_genre_declares(#[case] genre: Settings) {
    let doc = diagram_engine(genre)
        .lint(&Input::diagram(
            "flow.mmd",
            LONG_LABEL_FILE,
            Format::Mermaid,
        ))
        .unwrap();
    let seen: Vec<&str> = doc
        .contexts_seen
        .iter()
        .map(slopvac_model::ContextTag::as_str)
        .collect();
    assert_eq!(seen, ["diagram-label", "security"]);
    assert_eq!(doc.findings.len(), 1);
}

#[test]
fn a_standalone_diagram_of_another_genre_stays_silent_for_a_gated_rule() {
    let doc = diagram_engine(settings())
        .lint(&Input::diagram(
            "flow.mmd",
            LONG_LABEL_FILE,
            Format::Mermaid,
        ))
        .unwrap();
    assert_eq!(doc.findings.len(), 0);
}

/// Tags `count` structural tags of distinct kinds over the first byte.
struct Many(usize);

impl Tagger for Many {
    fn id(&self) -> &'static str {
        "many"
    }

    fn tag(&self, _: &dyn DocumentView) -> Result<Vec<Tag>, slopvac_context::Error> {
        Ok((0..self.0)
            .map(|i| Tag {
                kind: format!("k{i}"),
                span: 0..1,
                confidence: 1.0,
                evidence: String::new(),
            })
            .collect())
    }
}

/// The ceiling in `docs/contract/policies.md`: one tag for every 64 bytes plus 65,536.
fn ceiling(source_len: usize) -> usize {
    source_len / 64 + 65_536
}

#[rstest]
#[case::at_the_ceiling(0, true)]
#[case::one_over(1, false)]
fn declared_tags_count_against_the_tag_ceiling(#[case] structural_over: usize, #[case] fits: bool) {
    let limit = ceiling(BODY.len());
    // One declared tag (`security`) joins the structural ones.
    let structural = limit - 1 + structural_over;
    let mut registry = TagRegistry::new();
    registry.register(Box::new(Many(structural))).unwrap();
    let engine = gated_engine(security_run()).with_taggers(registry);
    let result = engine.lint(&Input::markdown("a.md", BODY));
    if fits {
        assert!(result.is_ok());
    } else {
        assert_eq!(
            result.map(|d| d.findings.len()),
            Err(EngineError::TagBuildBudget { limit })
        );
    }
}

/// `count` copies of one tag: a kind and the end of its span, from byte 0.
struct Repeat {
    kind: &'static str,
    end: usize,
    count: usize,
}

impl Tagger for Repeat {
    fn id(&self) -> &'static str {
        "repeat"
    }

    fn tag(&self, _: &dyn DocumentView) -> Result<Vec<Tag>, slopvac_context::Error> {
        Ok((0..self.count)
            .map(|_| Tag {
                kind: self.kind.to_owned(),
                span: 0..self.end,
                confidence: 1.0,
                evidence: String::new(),
            })
            .collect())
    }
}

// The tag collector counts each emission against the ceiling before it drops duplicates, so
// the declared tag counts the same way: as one more emission, whatever it equals.
#[rstest]
#[case::duplicates_fill_the_ceiling("dup", 1, 0, true)]
#[case::duplicates_over_the_ceiling("dup", 1, 1, false)]
#[case::copies_of_the_declared_tag_fill_the_ceiling("security", BODY.len(), 0, true)]
#[case::copies_of_the_declared_tag_over_the_ceiling("security", BODY.len(), 1, false)]
fn duplicates_and_copies_of_a_declared_tag_count_as_emissions(
    #[case] kind: &'static str,
    #[case] end: usize,
    #[case] over: usize,
    #[case] fits: bool,
) {
    let limit = ceiling(BODY.len());
    // The declared `security` tag is one more emission.
    let count = limit - 1 + over;
    let mut registry = TagRegistry::new();
    registry
        .register(Box::new(Repeat { kind, end, count }))
        .unwrap();
    let engine = gated_engine(security_run()).with_taggers(registry);
    let result = engine.lint(&Input::markdown("a.md", BODY));
    if fits {
        assert!(result.is_ok());
    } else {
        assert_eq!(
            result.map(|d| d.findings.len()),
            Err(EngineError::TagBuildBudget { limit })
        );
    }
}

#[test]
fn a_run_genre_the_registry_does_not_know_declares_no_context() {
    let mut run = settings();
    run.genre.id = GenreId::new("my-house-style").unwrap();
    let doc = gated_engine(run)
        .lint(&Input::markdown("a.md", BODY))
        .unwrap();
    assert!(doc.contexts_seen.is_empty());
    assert_eq!(doc.findings.len(), 0);
}

#[test]
fn a_path_assigned_context_survives_a_front_matter_genre() {
    let mut s = settings();
    s.genre_assignments = Some(vec![GenreAssignment {
        files: vec!["policies/**".into()],
        genre: "security".into(),
    }]);
    let engine = gated_engine(s);
    let source = format!("---\ngenre: consumer\n---\n{BODY}");
    let lint = |path: &str| {
        engine
            .lint(&Input::markdown(path, &source).with_declared_genre("consumer"))
            .unwrap()
    };
    let assigned = lint("policies/a.md");
    assert!(
        assigned
            .findings
            .iter()
            .any(|f| f.rule_id.as_str().ends_with("gated"))
    );
    assert_eq!(assigned.genre.id.as_str(), "consumer");
    let elsewhere = lint("docs/a.md");
    assert_eq!(elsewhere.findings.len(), 0);
}

#[cfg(unix)]
mod wrapped {
    use super::*;
    use common::{places, wrapped_rst, wrapped_table};

    /// One rule per cell: `Must` fills the left cell, `x` sits in the right one.
    fn engine(settings: Settings) -> Engine {
        let only = json!({"contexts": {"only": ["security"]}});
        let rules = [
            common::tokens("left", "Must", &only),
            common::tokens("right", "x", &only),
        ];
        Engine::new(vec![ruleset(vec![category("cat", &rules)])], settings).unwrap()
    }

    #[test]
    fn a_declared_context_reaches_both_cells_of_a_wrapped_table() {
        let lines = 6;
        let doc = engine(security_run())
            .with_rst(wrapped_rst(lines))
            .lint(&Input::rst("t.rst", &wrapped_table(lines)))
            .unwrap();
        // Each line of the left cell after the border and `| `, then `x` on the first line.
        let mut want: Vec<_> = (0..lines)
            .map(|line| ("core:cat.left".to_owned(), 2 + line, 3))
            .collect();
        want.push(("core:cat.right".to_owned(), 2, 16));
        let mut got = places(&doc);
        got.sort();
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn a_table_of_another_genre_is_not_gated_in_by_a_declared_context() {
        let lines = 6;
        let doc = engine(settings())
            .with_rst(wrapped_rst(lines))
            .lint(&Input::rst("t.rst", &wrapped_table(lines)))
            .unwrap();
        assert_eq!(doc.findings.len(), 0);
    }

    /// Tags of distinct kinds over the left cell: `kinds` taggers, each matching `Must`.
    fn cell_taggers(kinds: usize) -> TagRegistry {
        let mut registry = TagRegistry::new();
        for i in 0..kinds {
            let yaml = format!(
                "schema_version: 3\nid: t{i}\ntag: k{i}\nunit: table-cell\nmatch:\n  - pattern: 'Must'\n"
            );
            let tagger = slopvac_context::TaggerSpec::from_yaml(&yaml)
                .unwrap()
                .compile()
                .unwrap();
            registry.register(Box::new(tagger)).unwrap();
        }
        registry
    }

    #[rstest]
    #[case::no_declared_context(settings(), true)]
    #[case::declared_context(security_run(), false)]
    fn pieces_of_a_declared_tag_count_against_the_ceiling_after_the_split(
        #[case] genre: Settings,
        #[case] fits: bool,
    ) {
        // 2,731 tags on a cell of 24 lines split into 2,731 * 24 = 65,544 pieces, exactly the
        // ceiling for this table. The declared tag spans the whole document, so it stays one
        // piece, and it is the one that passes the ceiling.
        const KINDS: usize = 2_731;
        let lines = 24;
        let source = wrapped_table(lines);
        let limit = ceiling(source.len());
        let result = engine(genre)
            .with_rst(wrapped_rst(lines))
            .with_taggers(cell_taggers(KINDS))
            .lint(&Input::rst("t.rst", &source));
        if fits {
            assert!(result.is_ok());
        } else {
            assert_eq!(
                result.map(|d| d.findings.len()),
                Err(EngineError::TagBuildBudget { limit })
            );
        }
    }
}
