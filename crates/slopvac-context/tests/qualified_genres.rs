//! Genres from several packs keep qualified keys `<pack>/<genre>`.

use rstest::rstest;
use slopvac_context::{GenreRegistry, GenreSpec};

fn spec(pack: &str, id: &str, extends: &[&str]) -> GenreSpec {
    let list = extends
        .iter()
        .map(|e| format!("\"{e}\""))
        .collect::<Vec<_>>()
        .join(", ");
    GenreSpec::from_yaml(&format!(
        "schema_version: 3\nid: {id}\ndescription: {pack} {id}\nextends: [{list}]\n"
    ))
    .unwrap()
}

/// `core/base`, `core/howto`, `acme/howto` (extends the bare `base` of core) and `acme/memo`.
fn registry() -> GenreRegistry {
    GenreRegistry::from_packs(
        vec![
            ("core".into(), spec("core", "base", &[])),
            ("core".into(), spec("core", "howto", &["base"])),
            ("acme".into(), spec("acme", "howto", &["base"])),
            ("acme".into(), spec("acme", "memo", &["howto"])),
        ],
        "core/base",
    )
    .unwrap()
}

#[test]
fn qualified_ids_list_every_genre_in_definition_order() {
    assert_eq!(
        registry().ids(),
        ["core/base", "core/howto", "acme/howto", "acme/memo"]
    );
}

#[rstest]
#[case::core("core/howto", "howto")]
#[case::other_pack("acme/howto", "howto")]
#[case::unique_bare("memo", "memo")]
fn a_qualified_or_unique_name_finds_its_genre(#[case] key: &str, #[case] id: &str) {
    assert_eq!(registry().spec(key).unwrap().id, id);
}

#[rstest]
#[case::missing_pack("zed/howto")]
#[case::missing_genre("core/memo")]
#[case::unknown("nothing")]
fn an_unknown_name_finds_nothing(#[case] key: &str) {
    assert!(registry().spec(key).is_none());
}

#[test]
fn a_bare_name_that_two_packs_define_finds_nothing_and_lists_both_claims() {
    let r = registry();
    assert!(r.spec("howto").is_none());
    assert_eq!(r.claims("howto"), ["core/howto", "acme/howto"]);
}

#[test]
fn a_bare_parent_names_a_genre_of_the_same_pack_first() {
    // `acme/memo` extends the bare `howto`: acme's own, not core's, though both exist.
    let chain: Vec<Option<String>> = registry()
        .chain("acme/memo")
        .unwrap()
        .iter()
        .map(|s| s.description.clone())
        .collect();
    let expected = ["core base", "acme howto", "acme memo"].map(|d| Some(d.to_owned()));
    assert_eq!(chain, expected);
}

#[test]
fn the_same_genre_twice_in_one_pack_is_refused() {
    let err = GenreRegistry::from_packs(
        vec![
            ("core".into(), spec("core", "base", &[])),
            ("core".into(), spec("core", "base", &[])),
        ],
        "core/base",
    )
    .err()
    .unwrap();
    assert!(err.to_string().contains("defined twice"), "{err}");
}

#[test]
fn a_parent_no_pack_supplies_is_refused() {
    let err = GenreRegistry::from_packs(
        vec![("core".into(), spec("core", "base", &["missing"]))],
        "core/base",
    )
    .err()
    .unwrap();
    assert!(err.to_string().contains("extends unknown genre"), "{err}");
}
