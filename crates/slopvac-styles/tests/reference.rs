//! The generated style reference pages.
#![cfg(test)]

use slopvac_styles::{BuiltinSource, StyleSource, reference_pages};

fn page(path: &str) -> String {
    reference_pages()
        .into_iter()
        .find(|(p, _)| p == path)
        .unwrap_or_else(|| panic!("no page {path}"))
        .1
}

#[test]
fn there_is_one_page_per_bundled_preset_and_an_index() {
    let paths: Vec<String> = reference_pages().into_iter().map(|(p, _)| p).collect();
    let mut expected = vec!["styles/index.md".to_owned()];
    expected.extend(BuiltinSource::names().map(|n| format!("styles/{n}.md")));
    assert_eq!(paths, expected);
}

#[test]
fn index_lists_every_preset_with_its_description() {
    let index = page("styles/index.md");
    for name in BuiltinSource::names() {
        let style = BuiltinSource.load(name).unwrap();
        assert!(
            index.contains(&format!(
                "| [`{name}`]({name}.md) | {} |",
                style.description.as_deref().unwrap_or_default()
            )),
            "{name}"
        );
    }
}

#[test]
fn every_rule_and_category_entry_appears_on_its_page() {
    for name in BuiltinSource::names() {
        let style = BuiltinSource.load(name).unwrap();
        let text = page(&format!("styles/{name}.md"));
        for key in style.rules.keys() {
            assert!(text.contains(&format!("| `{key}` |")), "{name}: {key}");
        }
        for key in style.categories.keys() {
            assert!(text.contains(&format!("| `{key}` |")), "{name}: {key}");
        }
    }
}

#[test]
fn google_page_shows_conflict_levels_and_attribution() {
    let text = page("styles/google.md");
    assert!(text.starts_with("# Style: google\n"), "{text}");
    assert!(text.contains("- Conflicts with: [`ste`](ste.md)"), "{text}");
    assert!(
        text.contains("| `ste-punctuation.semicolon-used` | `off` |"),
        "{text}"
    );
    assert!(text.contains("CC BY 4.0"), "{text}");
    assert!(text.contains("styles = [\"google\"]"), "{text}");
}

#[test]
fn ste_page_shows_category_dials() {
    let text = page("styles/ste.md");
    assert!(
        text.contains("| `ste-safety` | severity `error`, max per 100 words `0`, weight `1.2` |"),
        "{text}"
    );
    assert!(
        text.contains("| `ste-vocabulary` | severity `off` |"),
        "{text}"
    );
}
