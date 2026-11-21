//! The core pack loads from the copy in the binary and from a copy on disk.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use std::collections::BTreeSet;
use std::fs;

use rstest::rstest;
use slopvac_config::Presets;
use slopvac_model::PackId;
use slopvac_packs::{
    DirSource, EmbeddedSource, LoadedPack, MANIFEST_FILE, PackError, PackSource, Packs, Source,
    core_source,
};

fn rule_ids(pack: &LoadedPack) -> BTreeSet<String> {
    pack.rules().rules().map(|r| r.id().to_string()).collect()
}

/// Write the embedded core pack to `dir` as a pack folder.
fn write_copy(source: &EmbeddedSource, dir: &std::path::Path) {
    fs::write(dir.join(MANIFEST_FILE), source.manifest_text().unwrap()).unwrap();
    for (path, text) in source.files() {
        let target = dir.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, text).unwrap();
    }
}

#[test]
fn embedded_and_on_disk_copies_load_the_same_rule_ids() {
    let embedded = core_source();
    let dir = tempfile::tempdir().unwrap();
    write_copy(&embedded, dir.path());

    let from_binary = LoadedPack::load(&embedded, Source::Core).unwrap();
    let from_disk = LoadedPack::load(&DirSource::new(dir.path()), Source::Core).unwrap();

    let ids = rule_ids(&from_binary);
    assert!(
        ids.len() > 100,
        "the core pack carries the converted 2.x rules"
    );
    assert!(ids.iter().all(|id| id.starts_with("core:")));
    assert_eq!(ids, rule_ids(&from_disk));
}

#[test]
fn core_genres_keep_the_registration_order_of_the_built_ins() {
    let packs = Packs::core().unwrap();
    let ids: Vec<&str> = packs.genres().map(|g| g.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "consumer",
            "reference",
            "change-comms",
            "internal",
            "informal",
            "security",
            "ui-copy",
            "agent-instructions",
            "tutorial",
            "how-to",
            "reference-page",
            "explanation",
        ]
    );
}

#[test]
fn core_taggers_load_in_file_order() {
    let packs = Packs::core().unwrap();
    assert_eq!(packs.taggers().count(), 15);
}

#[test]
fn a_genre_chain_lists_the_parent_before_the_genre() {
    let packs = Packs::core().unwrap();
    let chain = packs.genre_chain("core:tutorial");
    let sets_rule = |layer: &Vec<slopvac_config::Setting>| {
        layer
            .iter()
            .any(|s| s.path == ["rules", "core:prose-craft.future-tense"] && s.value == "off")
    };
    assert!(
        sets_rule(chain.last().unwrap()),
        "the tutorial layer comes last"
    );
    assert!(!chain[..chain.len() - 1].iter().any(sets_rule));
    assert!(chain.len() >= 2, "the consumer parent comes first");
}

#[test]
fn an_unknown_genre_has_no_chain() {
    let chain = Packs::core().unwrap().genre_chain("core:nope");
    assert_eq!(chain, Vec::<Vec<slopvac_config::Setting>>::new());
}

#[test]
fn core_styles_resolve_by_bare_and_qualified_name() {
    let packs = Packs::core().unwrap();
    assert!(packs.style("ste").is_some());
    assert!(packs.style("core:google").is_some());
    assert!(packs.style("nope").is_none());
}

#[test]
fn the_core_pack_takes_the_binary_version() {
    let pack = LoadedPack::load(&core_source(), Source::Core).unwrap();
    assert_eq!(
        pack.manifest().version.to_string(),
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(pack.manifest().id, PackId::new("core").unwrap());
}

#[rstest]
#[case::plugin(Source::Plugin)]
#[case::pack(Source::Pack)]
fn the_core_manifest_is_refused_from_any_other_origin(#[case] origin: Source) {
    let err = LoadedPack::load(&core_source(), origin).unwrap_err();
    assert!(matches!(err, PackError::ReservedId));
}

fn write_pack(provides: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let manifest = format!(
        "schema_version = 3\nid = \"acme\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"data\"\n\n[provides]\n{provides}\n"
    );
    fs::write(dir.path().join(MANIFEST_FILE), manifest).unwrap();
    fs::create_dir(dir.path().join("taggers")).unwrap();
    // Valid YAML that would fail the tagger schema: the loader must not read it.
    fs::write(dir.path().join("taggers/t.yaml"), "not: a tagger\n").unwrap();
    dir
}

#[test]
fn a_non_core_pack_loads_no_taggers() {
    let dir = write_pack("taggers = [\"taggers/t.yaml\"]");
    let pack = LoadedPack::load(&DirSource::new(dir.path()), Source::Pack).unwrap();
    assert!(pack.taggers().is_empty());
}
