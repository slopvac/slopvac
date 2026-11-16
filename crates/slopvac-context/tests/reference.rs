//! The generated genre reference pages.

use std::collections::BTreeMap;

use slopvac_context::{builtin_genres, reference_pages};

fn pages() -> BTreeMap<String, String> {
    reference_pages().into_iter().collect()
}

fn page(id: &str) -> String {
    pages()[&format!("genres/{id}.md")].clone()
}

#[test]
fn there_is_an_index_and_one_page_per_builtin_genre() {
    let mut want: Vec<String> = builtin_genres()
        .unwrap()
        .ids()
        .iter()
        .map(|id| format!("genres/{id}.md"))
        .collect();
    want.push("genres/index.md".to_string());
    want.sort();
    assert_eq!(pages().keys().cloned().collect::<Vec<_>>(), want);
}

#[test]
fn the_diataxis_content_types_have_pages() {
    let pages = pages();
    for id in ["tutorial", "how-to", "reference-page", "explanation"] {
        assert!(pages.contains_key(&format!("genres/{id}.md")), "{id}");
    }
}

#[test]
fn the_index_links_every_genre_with_its_description() {
    let index = &pages()["genres/index.md"];
    assert!(index.starts_with("# Genres\n"));
    assert!(index.contains(
        "- [`change-comms`](change-comms.md): Change, rationale, verification. PR and MR text, \
         commits, release notes, migration guides.\n"
    ));
    assert!(index.contains("- [`security`](security.md): Security guidance"));
}

#[test]
fn a_page_lists_profile_minimum_score_and_signals_with_weights() {
    let page = page("informal");
    assert!(page.starts_with("# informal\n"));
    assert!(page.contains("Immediate context. Issue comments, discussions, blogs, drafts."));
    assert!(page.contains("- Profile: `relaxed`\n"));
    assert!(page.contains("Minimum score: 3\n"));
    assert!(page.contains(
        "- Path matches the glob `**/{blog,post,posts,draft,drafts,note,notes,discussion,\
         discussions}/**` (weight 3)\n"
    ));
    assert!(page.contains("- Front matter `status` equals `draft` (weight 3)\n"));
}

#[test]
fn heading_signals_show_pattern_and_weight() {
    assert!(page("consumer").contains(
        "- Heading matches `^\\W*(?:installation|usage|getting started|quick ?start|features)\\W*$` \
         (weight 1.5)\n"
    ));
}

#[test]
fn an_extending_genre_shows_its_parents_chain_inherited_profile_and_rule_levels() {
    let page = page("tutorial");
    assert!(page.contains("- Extends: `consumer`\n"));
    assert!(page.contains("- Resolved chain, lowest first: `consumer` → `tutorial`\n"));
    assert!(page.contains("- Profile: `normal`\n"));
    assert!(page.contains("- `prose-craft.future-tense`: `off`\n"));
}

#[test]
fn a_genre_inherits_the_profile_of_the_genre_it_extends() {
    assert!(page("agent-instructions").contains("- Profile: `strict`\n"));
}

#[test]
fn a_declared_genre_has_no_detection_and_shows_its_context_tag() {
    let page = page("security");
    assert!(page.contains("None. Config or front matter choose this genre."));
    assert!(page.contains("## Context tags\n\n- `security`\n"));
}

#[test]
fn empty_sections_say_so() {
    let page = page("reference");
    assert!(page.contains("- Extends: none\n"));
    assert!(page.contains("This genre sets no styles."));
    assert!(page.contains("This genre sets no category settings."));
    assert!(page.contains("This genre sets no rule levels."));
    assert!(page.contains("This genre sets no context tags."));
}
