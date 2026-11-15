//! The bundled presets load and resolve.

use slopvac_styles::{BuiltinSource, CategoryDials, CategoryPatch, Level, resolve};

fn dials(p: &CategoryPatch) -> CategoryDials {
    match p {
        CategoryPatch::Dials(d) => d.clone(),
        CategoryPatch::Level(l) => CategoryDials {
            severity: Some(*l),
            ..CategoryDials::default()
        },
    }
}

#[test]
fn ste_preset_sets_the_normal_profile_levels() {
    let r = resolve(&["ste".to_owned()], &BuiltinSource, true).unwrap();
    let cats = &r.patch.categories;
    assert_eq!(cats.len(), 10);
    let safety = dials(&cats["ste-safety"]);
    assert_eq!(safety.severity, Some(Level::Error));
    assert_eq!(safety.max_per_100_words, Some(0.0));
    assert_eq!(safety.weight, Some(1.2));
    let words = dials(&cats["ste-words"]);
    assert_eq!(words.severity, Some(Level::Warning));
    assert_eq!(words.max_per_100_words, Some(1.5));
    assert_eq!(dials(&cats["ste-vocabulary"]).severity, Some(Level::Off));
}

#[test]
fn ste_preset_keeps_rules_that_two_x_ships_off_or_advisory() {
    let r = resolve(&["ste".to_owned()], &BuiltinSource, true).unwrap();
    let rules = &r.patch.rules;
    assert_eq!(rules.len(), 18);
    assert_eq!(rules["ste-practices.latin-abbreviation"], Level::Off);
    assert_eq!(rules["ste-verbs.passive-voice"], Level::Suggestion);
}

#[test]
fn google_preset_resolves_every_entry_to_its_documented_level() {
    let r = resolve(&["google".to_owned()], &BuiltinSource, true).unwrap();
    let expected = [
        ("ai-tells-formatting.title-case-heading", Level::Warning),
        ("ste-punctuation.semicolon-used", Level::Off),
        (
            "ste-sentences.complex-text-not-in-vertical-list",
            Level::Off,
        ),
        (
            "ste-sentences.vertical-list-lead-in-missing-colon",
            Level::Off,
        ),
        ("ste-sentences.vertical-list-item-punctuation", Level::Off),
        ("ste-practices.latin-abbreviation", Level::Warning),
        ("prose-inclusive.exclusive", Level::Warning),
        ("prose-inclusive.ableist", Level::Warning),
    ];
    assert_eq!(r.patch.rules.len(), expected.len());
    for (key, level) in expected {
        assert_eq!(r.patch.rules[key], level, "{key}");
    }
    assert!(r.patch.categories.is_empty());
}

#[test]
fn google_after_ste_wins_with_a_warning_and_strict_rejects_it() {
    let both = ["ste".to_owned(), "google".to_owned()];
    let r = resolve(&both, &BuiltinSource, false).unwrap();
    assert_eq!(r.conflicts.len(), 1);
    assert_eq!(r.conflicts[0].winner, "google");
    assert_eq!(r.patch.rules["ste-punctuation.semicolon-used"], Level::Off);
    assert_eq!(
        r.patch.rules["ste-practices.latin-abbreviation"],
        Level::Warning
    );
    assert!(resolve(&both, &BuiltinSource, true).is_err());
}

#[test]
fn ste_after_google_wins_the_overlapping_rules() {
    let both = ["google".to_owned(), "ste".to_owned()];
    let r = resolve(&both, &BuiltinSource, false).unwrap();
    assert_eq!(r.conflicts[0].winner, "ste");
    assert_eq!(
        r.patch.rules["ste-practices.latin-abbreviation"],
        Level::Off
    );
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
