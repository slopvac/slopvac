//! Genres from several packs keep qualified keys `<pack>:<genre>`.

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

/// `core:base`, `core:howto`, `acme:howto` (extends the bare `base` of core) and `acme:memo`.
fn registry() -> GenreRegistry {
    GenreRegistry::from_packs(
        vec![
            ("core".into(), spec("core", "base", &[])),
            ("core".into(), spec("core", "howto", &["base"])),
            ("acme".into(), spec("acme", "howto", &["base"])),
            ("acme".into(), spec("acme", "memo", &["howto"])),
        ],
        "core:base",
    )
    .unwrap()
}

#[test]
fn qualified_ids_list_every_genre_in_definition_order() {
    assert_eq!(
        registry().ids(),
        ["core:base", "core:howto", "acme:howto", "acme:memo"]
    );
}

#[rstest]
#[case::core("core:howto", "howto")]
#[case::other_pack("acme:howto", "howto")]
#[case::unique_bare("memo", "memo")]
fn a_qualified_or_unique_name_finds_its_genre(#[case] key: &str, #[case] id: &str) {
    assert_eq!(registry().spec(key).unwrap().id, id);
}

#[rstest]
#[case::missing_pack("zed:howto")]
#[case::missing_genre("core:memo")]
#[case::unknown("nothing")]
#[case::other_pack_than_the_bare_claim("missing:base")]
#[case::bare_claim_in_another_pack("acme:base")]
fn an_unknown_name_finds_nothing(#[case] key: &str) {
    assert!(registry().spec(key).is_none());
}

#[test]
fn a_bare_name_that_two_packs_define_finds_nothing_and_lists_both_claims() {
    let r = registry();
    assert!(r.spec("howto").is_none());
    assert_eq!(r.claims("howto"), ["core:howto", "acme:howto"]);
}

#[test]
fn a_bare_parent_names_a_genre_of_the_same_pack_first() {
    // `acme:memo` extends the bare `howto`: acme's own, not core's, though both exist.
    let chain: Vec<Option<String>> = registry()
        .chain("acme:memo")
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
        "core:base",
    )
    .err()
    .unwrap();
    assert!(err.to_string().contains("defined twice"), "{err}");
}

#[test]
fn a_parent_no_pack_supplies_is_refused() {
    let err = GenreRegistry::from_packs(
        vec![("core".into(), spec("core", "base", &["missing"]))],
        "core:base",
    )
    .err()
    .unwrap();
    assert!(err.to_string().contains("extends unknown genre"), "{err}");
}

#[rstest]
#[case::qualified("core:base")]
#[case::bare("base")]
fn contexts_added_for_a_genre_follow_it_under_any_reference(#[case] reference: &str) {
    let r = registry().with_contexts(reference, &["security"]).unwrap();

    // Declared on `core:base`, so `core:howto`, which extends it, inherits the tag.
    assert_eq!(r.contexts("core:howto").unwrap(), ["security"]);
    assert_eq!(r.contexts("core:base").unwrap(), ["security"]);
    assert!(
        r.contexts("acme:memo")
            .unwrap()
            .contains(&"security".to_owned())
    );
}

#[test]
fn contexts_added_to_one_pack_genre_do_not_reach_a_same_named_genre_of_another() {
    let r = registry().with_contexts("acme:howto", &["legal"]).unwrap();

    assert_eq!(r.contexts("acme:howto").unwrap(), ["legal"]);
    assert_eq!(r.contexts("core:howto").unwrap(), Vec::<String>::new());
}

#[rstest]
#[case::other_pack("missing:consumer", false)]
#[case::the_core_pack("core:consumer", true)]
#[case::bare("consumer", true)]
fn a_qualified_reference_to_the_builtin_genres_is_exact(
    #[case] reference: &str,
    #[case] found: bool,
) {
    let r = slopvac_context::builtin_genres().unwrap();

    assert_eq!(r.spec(reference).is_some(), found);
}

#[rstest]
#[case::any_pack("core:base")]
#[case::other_pack("acme:base")]
fn a_registry_that_names_no_pack_resolves_no_qualified_reference(#[case] reference: &str) {
    let r = GenreRegistry::new(vec![spec("core", "base", &[])], "base").unwrap();

    assert!(r.spec(reference).is_none());
    assert!(r.spec("base").is_some());
}

#[test]
fn naming_the_pack_of_a_registry_makes_its_qualified_references_resolve() {
    let r = GenreRegistry::new(vec![spec("core", "base", &[])], "base")
        .unwrap()
        .in_pack("core")
        .unwrap();

    assert!(r.spec("core:base").is_some());
    assert!(r.spec("acme:base").is_none());
}
