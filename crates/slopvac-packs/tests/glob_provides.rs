//! A glob in `provides` expands against the files the pack holds.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::symlink;

use proptest::prelude::*;
use rstest::rstest;
use slopvac_packs::{
    DirSource, LimitKind, LoadedPack, MANIFEST_FILE, MAX_PACK_FILES, PackError, Packs, Source,
    SourceError,
};

fn pack(genres: &str, files: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let manifest = format!(
        "schema_version = 3\nid = \"acme\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"data\"\n\n[provides]\ngenres = {genres}\n"
    );
    fs::write(dir.path().join(MANIFEST_FILE), manifest).unwrap();
    for file in files {
        let target = dir.path().join(file);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let stem = target.file_stem().unwrap().to_str().unwrap().to_owned();
        fs::write(&target, format!("schema_version: 3\nid: {stem}\n")).unwrap();
    }
    dir
}

fn genre_ids(dir: &tempfile::TempDir) -> Result<Vec<String>, PackError> {
    let loaded = LoadedPack::load(&DirSource::new(dir.path()), Source::Pack)?;
    let packs = Packs::new(vec![loaded])?;
    Ok(packs.genres().map(|g| g.id.clone()).collect())
}

const FILES: [&str; 3] = ["genres/b.yaml", "genres/a.yaml", "genres/sub/c.yaml"];

#[rstest]
#[case::star_stays_in_one_directory(r#"["genres/*.yaml"]"#, &["a", "b"])]
#[case::double_star_crosses_directories(r#"["genres/**/*.yaml"]"#, &["a", "b", "c"])]
#[case::question_mark_is_one_character(r#"["genres/?.yaml"]"#, &["a", "b"])]
#[case::a_listed_file_is_not_repeated(r#"["genres/b.yaml", "genres/*.yaml"]"#, &["b", "a"])]
#[case::no_match_adds_nothing(r#"["genres/*.json"]"#, &[])]
fn a_glob_expands_to_the_files_it_matches(#[case] genres: &str, #[case] ids: &[&str]) {
    let dir = pack(genres, &FILES);

    assert_eq!(genre_ids(&dir).unwrap(), ids);
}

#[test]
fn a_glob_that_does_not_parse_is_refused() {
    let dir = pack(r#"["genres/[.yaml"]"#, &FILES);

    let err = genre_ids(&dir).unwrap_err();

    assert!(matches!(err, PackError::Glob { .. }), "{err}");
}

#[test]
fn a_link_in_a_pack_that_a_glob_lists_is_refused() {
    let dir = pack(r#"["genres/*.yaml"]"#, &FILES);
    symlink(
        dir.path().join("genres/a.yaml"),
        dir.path().join("genres/link.yaml"),
    )
    .unwrap();

    let err = genre_ids(&dir).unwrap_err();

    assert!(
        matches!(err, PackError::Source(SourceError::Link { .. })),
        "{err}"
    );
}

#[test]
fn a_pack_over_the_file_cap_is_refused_before_a_file_is_read() {
    let names: Vec<String> = (0..=MAX_PACK_FILES)
        .map(|i| format!("genres/g{i}.yaml"))
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let dir = pack(r#"["genres/*.yaml"]"#, &refs);
    // Unreadable content proves the cap is checked first.
    for name in &names {
        fs::write(dir.path().join(name), "not: a genre\n").unwrap();
    }

    let err = genre_ids(&dir).unwrap_err();

    assert!(
        matches!(
            err,
            PackError::Limit {
                what: LimitKind::Files,
                ..
            }
        ),
        "{err}"
    );
}

proptest! {
    /// Whatever files a directory holds, `dir/*.yaml` loads exactly those files, in path order.
    #[test]
    fn a_star_glob_loads_exactly_the_matching_files(
        stems in proptest::collection::btree_set("[a-z][a-z0-9]{0,5}", 0..8),
        other in proptest::collection::btree_set("[a-z][a-z0-9]{0,5}", 0..4),
    ) {
        let yaml: Vec<String> = stems.iter().map(|s| format!("genres/{s}.yaml")).collect();
        let text: Vec<String> = other.iter().map(|s| format!("genres/{s}.txt")).collect();
        let refs: Vec<&str> = yaml.iter().chain(&text).map(String::as_str).collect();
        let dir = pack(r#"["genres/*.yaml"]"#, &refs);

        let ids = genre_ids(&dir).unwrap();

        let expected: Vec<String> = stems.iter().cloned().collect::<BTreeSet<_>>().into_iter().collect();
        prop_assert_eq!(ids, expected);
    }
}
