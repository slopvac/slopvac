//! A pack folder is confined: no symlink on the way to a pack file reaches outside it.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use rstest::rstest;
use slopvac_packs::{DirSource, LoadedPack, MANIFEST_FILE, PackError, Source, SourceError};

const GENRE: &str = "schema_version: 3\nid: outside\n";

fn manifest(genre_path: &str) -> String {
    format!(
        "schema_version = 3\nid = \"acme\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"data\"\n\n[provides]\ngenres = [\"{genre_path}\"]\n"
    )
}

/// A pack folder and a sibling folder that holds a valid genre file the pack must not reach.
fn fixture(genre_path: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let base = tempfile::tempdir().unwrap();
    let pack = base.path().join("pack");
    let outside = base.path().join("outside");
    fs::create_dir(&pack).unwrap();
    fs::create_dir_all(outside.join("sub")).unwrap();
    fs::write(outside.join("g.yaml"), GENRE).unwrap();
    fs::write(outside.join("sub/g.yaml"), GENRE).unwrap();
    fs::write(pack.join(MANIFEST_FILE), manifest(genre_path)).unwrap();
    (base, pack)
}

fn load(pack: &Path) -> Result<LoadedPack, PackError> {
    LoadedPack::load(&DirSource::new(pack), Source::Pack)
}

#[rstest]
#[case::intermediate_directory("genres/g.yaml", "genres", "../outside")]
#[case::nested_directory("genres/sub/g.yaml", "genres", "../outside")]
#[case::final_file("g.yaml", "g.yaml", "../outside/g.yaml")]
fn a_symlink_on_the_way_to_a_pack_file_is_refused(
    #[case] provided: &str,
    #[case] link: &str,
    #[case] target: &str,
) {
    let (base, pack) = fixture(provided);
    symlink(target, pack.join(link)).unwrap();

    let err = load(&pack).expect_err("the pack must not reach outside itself");

    assert!(
        matches!(err, PackError::Source(SourceError::Read(_))),
        "{err}"
    );
    drop(base);
}

#[test]
fn a_symlink_below_a_real_directory_is_refused() {
    let (base, pack) = fixture("genres/inner/g.yaml");
    fs::create_dir(pack.join("genres")).unwrap();
    symlink("../../outside/sub", pack.join("genres/inner")).unwrap();

    let err = load(&pack).expect_err("the pack must not reach outside itself");

    assert!(
        matches!(err, PackError::Source(SourceError::Read(_))),
        "{err}"
    );
    drop(base);
}

#[test]
fn a_pack_folder_that_is_itself_reached_through_a_symlink_loads() {
    let (base, pack) = fixture("genres/g.yaml");
    fs::create_dir(pack.join("genres")).unwrap();
    fs::write(
        pack.join("genres/g.yaml"),
        "schema_version: 3\nid: inside\n",
    )
    .unwrap();
    let alias = base.path().join("alias");
    symlink(&pack, &alias).unwrap();

    let loaded = load(&alias).expect("the root is whatever the caller opened");

    assert_eq!(loaded.manifest().id.as_str(), "acme");
}

#[test]
fn a_missing_file_stays_missing_not_a_read_failure() {
    let (_base, pack) = fixture("genres/none.yaml");

    let err = load(&pack).unwrap_err();

    assert!(
        matches!(err, PackError::Source(SourceError::Missing { .. })),
        "{err}"
    );
}
