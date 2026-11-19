//! The bundled presets load and resolve.

use serde_json::{Value, json};
use slopvac_styles::{BuiltinSource, Resolution, resolve};

/// The merged value at `path`.
fn get<'a>(r: &'a Resolution, path: &[&str]) -> &'a Value {
    r.resolved
        .get(path)
        .unwrap_or_else(|| panic!("no value at {path:?}"))
}

/// How many keys the merged settings hold under `table`.
fn count(r: &Resolution, table: &str) -> usize {
    r.resolved.to_json()[table]
        .as_object()
        .map_or(0, serde_json::Map::len)
}

fn rule<'a>(r: &'a Resolution, key: &str) -> &'a Value {
    get(r, &["rules", key])
}

#[test]
fn ste_preset_sets_the_normal_profile_levels() {
    let r = resolve(&["ste".to_owned()], &BuiltinSource, true).unwrap();
    assert_eq!(count(&r, "categories"), 10);
    let cat = |c: &str, f: &str| get(&r, &["categories", c, f]).clone();
    assert_eq!(cat("ste-safety", "severity"), json!("error"));
    assert_eq!(cat("ste-safety", "max_per_100_words"), json!(0.0));
    assert_eq!(cat("ste-safety", "weight"), json!(1.2));
    assert_eq!(cat("ste-words", "severity"), json!("warning"));
    assert_eq!(cat("ste-words", "max_per_100_words"), json!(1.5));
    assert_eq!(cat("ste-vocabulary", "severity"), json!("off"));
}

#[test]
fn ste_preset_keeps_rules_that_two_x_ships_off_or_advisory() {
    let r = resolve(&["ste".to_owned()], &BuiltinSource, true).unwrap();
    assert_eq!(count(&r, "rules"), 17);
    assert_eq!(rule(&r, "ste-practices.latin-abbreviation"), &json!("off"));
    assert_eq!(rule(&r, "ste-verbs.passive-voice"), &json!("suggestion"));
}

#[test]
fn google_preset_resolves_every_entry_to_its_documented_level() {
    let r = resolve(&["google".to_owned()], &BuiltinSource, true).unwrap();
    let expected = [
        ("ai-tells-formatting.title-case-heading", "warning"),
        ("ste-punctuation.semicolon-used", "off"),
        ("ste-sentences.complex-text-not-in-vertical-list", "off"),
        ("ste-practices.latin-abbreviation", "warning"),
        ("prose-inclusive.exclusive", "warning"),
        ("prose-inclusive.ableist", "warning"),
    ];
    assert_eq!(count(&r, "rules"), expected.len());
    for (key, level) in expected {
        assert_eq!(rule(&r, key), &json!(level), "{key}");
    }
    assert_eq!(count(&r, "categories"), 0);
}

#[test]
fn google_after_ste_wins_with_a_warning_and_strict_rejects_it() {
    let both = ["ste".to_owned(), "google".to_owned()];
    let r = resolve(&both, &BuiltinSource, false).unwrap();
    assert_eq!(r.conflicts.len(), 1);
    assert_eq!(r.conflicts[0].winner, "google");
    assert_eq!(rule(&r, "ste-punctuation.semicolon-used"), &json!("off"));
    assert_eq!(
        rule(&r, "ste-practices.latin-abbreviation"),
        &json!("warning")
    );
    assert!(resolve(&both, &BuiltinSource, true).is_err());
}

#[test]
fn ste_after_google_wins_the_overlapping_rules() {
    let both = ["google".to_owned(), "ste".to_owned()];
    let r = resolve(&both, &BuiltinSource, false).unwrap();
    assert_eq!(r.conflicts[0].winner, "ste");
    assert_eq!(rule(&r, "ste-practices.latin-abbreviation"), &json!("off"));
}

#[test]
fn every_bundled_preset_loads() {
    for name in BuiltinSource::names() {
        resolve(&[name.to_owned()], &BuiltinSource, true).unwrap();
    }
}

#[test]
fn unknown_builtin_is_not_found() {
    assert!(resolve(&["zzz".to_owned()], &BuiltinSource, false).is_err());
}
