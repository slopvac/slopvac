//! The loader checks what the schema states beyond field types, and the genre graph it builds.

use std::fs;
use std::num::NonZeroUsize;

use rstest::rstest;
use slopvac_config::Presets;
use slopvac_packs::{
    ArtifactFault, DirSource, LoadedPack, MANIFEST_FILE, PackError, Packs, Source,
};

fn pack(id: &str, provides: &str, files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let manifest = format!(
        "schema_version = 3\nid = \"{id}\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"data\"\n\n[provides]\n{provides}\n"
    );
    fs::write(dir.path().join(MANIFEST_FILE), manifest).unwrap();
    for (name, text) in files {
        let target = dir.path().join(name);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, text).unwrap();
    }
    dir
}

fn load(dir: &tempfile::TempDir, origin: Source) -> Result<LoadedPack, PackError> {
    LoadedPack::load(&DirSource::new(dir.path()), origin)
}

fn genre(id: &str, extends: &str) -> String {
    format!("schema_version: 3\nid: {id}\nextends: [{extends}]\n")
}

#[rstest]
#[case::categories("categories")]
#[case::styles("styles")]
#[case::genres("genres")]
#[case::terms("terms")]
#[case::taggers("taggers")]
#[case::configs("configs")]
#[case::prime("prime")]
#[case::blocklists("blocklists")]
fn a_glob_in_any_provides_list_is_refused(
    #[case] kind: &str,
    #[values(("acme", Source::Pack), ("core", Source::Core))] origin: (&str, Source),
) {
    let dir = pack(origin.0, &format!("{kind} = [\"dir/*.yaml\"]"), &[]);

    let error = load(&dir, origin.1).unwrap_err();

    assert!(matches!(error, PackError::Glob { .. }), "{error}");
}

#[rstest]
#[case::bad_id("schema_version: 3\nid: Bad_Id\n")]
#[case::wrong_version("schema_version: 2\nid: g\n")]
#[case::empty_title("schema_version: 3\nid: g\ntitle: \"\"\n")]
#[case::repeated_parent("schema_version: 3\nid: g\nextends: [a, a]\n")]
#[case::negative_weight("schema_version: 3\nid: g\ncategories:\n  c:\n    weight: -1\n")]
fn a_genre_that_breaks_a_schema_rule_is_refused(#[case] text: &str) {
    let dir = pack("acme", "genres = [\"g.yaml\"]", &[("g.yaml", text)]);

    let error = load(&dir, Source::Pack).unwrap_err();

    assert!(
        matches!(
            error,
            PackError::Artifact {
                fault: ArtifactFault::Invalid,
                ..
            }
        ),
        "{error}"
    );
}

#[rstest]
#[case::confidence_out_of_range(
    "schema_version: 3\nid: t\ntag: procedure\nconfidence: 2\nmatch:\n  - pattern: 'a'\n"
)]
#[case::bad_tag("schema_version: 3\nid: t\ntag: Not_A_Tag\nmatch:\n  - pattern: 'a'\n")]
fn a_core_tagger_that_breaks_a_schema_rule_is_refused(#[case] text: &str) {
    let dir = pack("core", "taggers = [\"t.yaml\"]", &[("t.yaml", text)]);

    let error = load(&dir, Source::Core).unwrap_err();

    assert!(matches!(error, PackError::Artifact { .. }), "{error}");
}

fn packs_with(genres: &[(&str, &str)]) -> Result<Packs, PackError> {
    let files: Vec<(String, String)> = genres
        .iter()
        .map(|(id, extends)| (format!("{id}.yaml"), genre(id, extends)))
        .collect();
    let list = files
        .iter()
        .map(|(n, _)| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(n, t)| (n.as_str(), t.as_str()))
        .collect();
    let dir = pack("acme", &format!("genres = [{list}]"), &refs);
    Packs::new(vec![load(&dir, Source::Pack).unwrap()])
}

#[rstest]
#[case::self_loop(&[("a", "a")])]
#[case::two_step(&[("a", "b"), ("b", "a")])]
#[case::three_step(&[("a", "b"), ("b", "c"), ("c", "a")])]
fn a_genre_cycle_is_refused(#[case] genres: &[(&str, &str)]) {
    let error = packs_with(genres).unwrap_err();

    assert!(matches!(error, PackError::GenreCycle { .. }), "{error}");
}

#[rstest]
#[case::bare(&[("a", "gone")])]
#[case::qualified(&[("a", "acme:gone")])]
#[case::other_pack(&[("a", "nopack:gone")])]
fn a_missing_parent_is_refused(#[case] genres: &[(&str, &str)]) {
    let error = packs_with(genres).unwrap_err();

    assert!(matches!(error, PackError::MissingParent { .. }), "{error}");
}

#[test]
fn a_diamond_of_parents_is_not_a_cycle() {
    let packs = packs_with(&[("a", "b, c"), ("b", "d"), ("c", "d"), ("d", "")]).unwrap();

    assert_eq!(packs.genres().count(), 4);
}

#[test]
fn a_genre_setting_keeps_the_line_and_column_of_its_key() {
    let text = "schema_version: 3\nid: g\nprofile: strict\ncategories:\n  prose:\n    weight: 2\nrules:\n  acme:r: off\n";
    let dir = pack("acme", "genres = [\"g.yaml\"]", &[("g.yaml", text)]);
    let packs = Packs::new(vec![load(&dir, Source::Pack).unwrap()]).unwrap();

    let chain = packs.genre_chain("g");

    let at = |path: &[&str]| {
        let s = chain
            .iter()
            .flatten()
            .find(|s| s.path == path)
            .unwrap_or_else(|| panic!("no setting {path:?}"));
        (
            s.source.line.map(NonZeroUsize::get),
            s.source.column.map(NonZeroUsize::get),
        )
    };
    assert_eq!(at(&["profile", "name"]), (Some(3), Some(1)));
    assert_eq!(at(&["categories", "prose", "weight"]), (Some(6), Some(5)));
    assert_eq!(at(&["rules", "acme:r"]), (Some(8), Some(3)));
}
