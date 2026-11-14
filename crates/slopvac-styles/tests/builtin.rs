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
fn every_bundled_preset_loads() {
    for name in BuiltinSource::names() {
        resolve(&[name.to_owned()], &BuiltinSource, true).unwrap();
    }
}

#[test]
fn unknown_builtin_is_not_found() {
    assert!(resolve(&["zzz".to_owned()], &BuiltinSource, false).is_err());
}
