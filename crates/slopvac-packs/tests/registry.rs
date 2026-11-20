//! Behaviors of `Registry`, `QualifiedName` and `Provenance`.

use rstest::rstest;
use slopvac_model::{PackId, RuleProvenance};
use slopvac_packs::{Provenance, QualifiedName, Reference, Registry, RegistryError, Source};

fn prov(pack: &str, version: &str, source: Source, file: &str) -> Provenance {
    Provenance::new(
        PackId::new(pack).unwrap(),
        version.parse().unwrap(),
        source,
        file,
    )
}

fn q(text: &str) -> QualifiedName {
    text.parse().unwrap()
}

fn registry() -> Registry<u32> {
    let mut r = Registry::new();
    r.insert(
        q("core:howto"),
        1,
        prov("core", "3.0.0", Source::Core, "genres/howto.toml"),
    )
    .unwrap();
    r.insert(
        q("core:memo"),
        2,
        prov("core", "3.0.0", Source::Core, "genres/memo.toml"),
    )
    .unwrap();
    r.insert(
        q("acme:howto"),
        3,
        prov("acme", "1.2.0", Source::Pack, "g/howto.toml"),
    )
    .unwrap();
    r
}

#[test]
fn get_finds_an_exact_qualified_name() {
    let r = registry();
    let e = r.get(&q("acme:howto")).unwrap();
    assert_eq!(e.value, 3);
    assert_eq!(e.provenance.pack.as_str(), "acme");
    assert!(r.get(&q("acme:memo")).is_none());
}

#[test]
fn a_duplicate_names_both_suppliers_and_keeps_the_first() {
    let mut r = registry();
    let err = r
        .insert(
            q("core:memo"),
            9,
            prov("core", "3.1.0", Source::Core, "other.toml"),
        )
        .unwrap_err();
    let shown = err.to_string();
    assert!(shown.contains("`core:memo`"), "{shown}");
    assert!(
        shown.contains("core@3.0.0 (core) genres/memo.toml"),
        "{shown}"
    );
    assert!(shown.contains("core@3.1.0 (core) other.toml"), "{shown}");
    assert_eq!(r.get(&q("core:memo")).unwrap().value, 2);
    assert_eq!(r.len(), 3);
}

#[rstest]
#[case::qualified_core("core:howto", 1)]
#[case::qualified_pack("acme:howto", 3)]
#[case::bare_with_one_claim("memo", 2)]
fn resolve_finds_one_entry(#[case] text: &str, #[case] want: u32) {
    assert_eq!(registry().resolve(text).unwrap().value, want);
}

#[test]
fn a_bare_name_claimed_by_two_packs_is_ambiguous_and_names_both() {
    let err = registry().resolve("howto").unwrap_err();
    let RegistryError::Ambiguous { name, claims, .. } = &err else {
        panic!("{err:?}");
    };
    assert_eq!(name, "howto");
    let names: Vec<String> = claims.iter().map(|(n, _)| n.to_string()).collect();
    assert_eq!(names, ["acme:howto", "core:howto"]);
    let shown = err.to_string();
    assert!(shown.contains("acme@1.2.0 (pack) g/howto.toml"), "{shown}");
    assert!(
        shown.contains("core@3.0.0 (core) genres/howto.toml"),
        "{shown}"
    );
}

#[rstest]
#[case::unknown_bare("missing")]
#[case::unknown_qualified("acme:memo")]
fn resolve_reports_an_unknown_name(#[case] text: &str) {
    assert!(matches!(
        registry().resolve(text),
        Err(RegistryError::Unknown(t)) if t == text
    ));
}

#[rstest]
#[case::empty("")]
#[case::space("a b")]
#[case::uppercase_pack("Acme:howto")]
#[case::empty_local("acme:")]
fn resolve_rejects_text_that_is_no_name(#[case] text: &str) {
    assert!(matches!(
        registry().resolve(text),
        Err(RegistryError::Name(_))
    ));
}

#[test]
fn control_characters_in_a_name_are_escaped_in_the_message() {
    let err = registry().resolve("a\u{1b}[2J").unwrap_err();
    assert!(!err.to_string().contains('\u{1b}'));
}

#[test]
fn a_qualified_name_prints_as_it_parses() {
    assert_eq!(q("acme:cat.rule_1").to_string(), "acme:cat.rule_1");
}

#[test]
fn provenance_converts_to_the_report_shape() {
    let p = prov("acme", "1.2.0", Source::Pack, "rules/a.yml");
    let r = RuleProvenance::from(&p);
    assert_eq!(r.pack.as_str(), "acme");
    assert_eq!(r.version.as_deref(), Some("1.2.0"));
    assert_eq!(r.source.as_deref(), Some("rules/a.yml"));
}

fn pack_vs_pack() -> Registry<u32> {
    let mut r = Registry::new();
    r.insert(
        q("acme:memo"),
        1,
        prov("acme", "1.0.0", Source::Pack, "g/memo.toml"),
    )
    .unwrap();
    r.insert(
        q("zed:memo"),
        2,
        prov("zed", "2.0.0", Source::Pack, "genres/memo.toml"),
    )
    .unwrap();
    r
}

#[rstest]
#[case::core_vs_pack(registry(), "howto", &["acme:howto", "core:howto"])]
#[case::pack_vs_pack(pack_vs_pack(), "memo", &["acme:memo", "zed:memo"])]
fn an_ambiguous_bare_name_has_a_stable_code_names_both_ids_and_points_at_the_reference(
    #[case] r: Registry<u32>,
    #[case] bare: &str,
    #[case] ids: &[&str],
) {
    let at = Reference::new("docs/config.toml", 7, 12);
    let err = r.resolve_at(bare, &at).unwrap_err();
    assert_eq!(err.code(), Some("pack-name-ambiguous"));
    let shown = err.to_string();
    assert!(
        shown.starts_with("pack-name-ambiguous at docs/config.toml:7:12: "),
        "{shown}"
    );
    for id in ids {
        assert!(shown.contains(&format!("`{id}`")), "{shown}");
    }
}

#[test]
fn a_reference_path_is_escaped_in_the_message() {
    let at = Reference::new("a\u{1b}[2Jb.toml", 1, 1);
    let err = registry().resolve_at("howto", &at).unwrap_err();
    assert!(!err.to_string().contains('\u{1b}'));
}
