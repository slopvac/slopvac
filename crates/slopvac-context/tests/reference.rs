//! The generated genre reference pages.

use std::collections::BTreeMap;

use slopvac_context::{builtin_genres, reference_pages};

fn pages() -> BTreeMap<String, String> {
    reference_pages().into_iter().collect()
}

#[test]
fn there_is_an_index_and_one_page_per_builtin_genre() {
    let pages = pages();
    let mut want: Vec<String> = builtin_genres()
        .unwrap()
        .ids()
        .iter()
        .map(|id| format!("genres/{id}.md"))
        .collect();
    want.push("genres/index.md".to_string());
    want.sort();
    assert_eq!(pages.keys().cloned().collect::<Vec<_>>(), want);
}

#[test]
fn the_index_links_every_genre_with_its_description() {
    let index = &pages()["genres/index.md"];
    assert!(index.starts_with("# Genres\n"));
    assert!(index.contains(
        "- [`change-comms`](change-comms.md): Change, rationale, verification. PR and MR text, \
         commits, release notes, migration guides."
    ));
    assert!(index.contains("- [`informal`](informal.md): "));
}

#[test]
fn a_page_lists_profile_threshold_signals_with_weights_and_the_prime_snippet() {
    let page = &pages()["genres/informal.md"];
    assert!(page.starts_with("# informal\n"));
    assert!(page.contains("Immediate context. Issue comments, discussions, blogs, drafts."));
    assert!(page.contains("- Profile: `relaxed`"));
    assert!(page.contains("- Detection threshold: 3"));
    assert!(page.contains(
        "- Path matches `(?:^|/)(?:blog|posts?|drafts?|notes?|discussions?)/` (weight 3)"
    ));
    assert!(page.contains("- Front matter `status` matches `^draft$` (weight 3)"));
    assert!(page.contains("> Keep it short and direct. Context matters more than polish."));
}

#[test]
fn a_genre_without_extends_or_tolerances_says_so() {
    let page = &pages()["genres/reference.md"];
    assert!(page.contains("- Extends: none"));
    assert!(page.contains("This genre sets no rule severities."));
    assert!(page.contains("This genre sets no context tags."));
}

#[test]
fn heading_signals_show_pattern_and_weight() {
    let page = &pages()["genres/consumer.md"];
    assert!(page.contains(
        "- Heading matches `^\\W*(?:installation|usage|getting started|quick ?start|features)\\W*$` (weight 1.5)"
    ));
}
