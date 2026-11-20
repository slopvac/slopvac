//! Genre registration and graph errors never repeat a genre id or an `extends` value. Those are
//! artifact values, so an error holds the pack-relative file and a location only
//! (docs/contract/policies.md section 1.7).

use std::error::Error;
use std::fs;

use rstest::rstest;
use slopvac_packs::{DirSource, LoadedPack, MANIFEST_FILE, PackError, Packs, Source};

const SENTINEL: &str = "sentinel-t0ken";

fn genre(id: &str, extends: &str) -> String {
    format!("schema_version: 3\nid: {id}\nextends: [{extends}]\n")
}

fn every_text(error: &(dyn Error + 'static)) -> Vec<String> {
    let mut texts = Vec::new();
    let mut current: Option<&(dyn Error + 'static)> = Some(error);
    while let Some(e) = current {
        texts.push(e.to_string());
        texts.push(format!("{e:?}"));
        current = e.source();
    }
    texts
}

/// Load a pack of the given `(file, text)` genres, then register it.
fn load_then_register(genres: &[(&str, String)]) -> PackError {
    let dir = tempfile::tempdir().unwrap();
    let list = genres
        .iter()
        .map(|(file, _)| format!("\"{file}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let manifest = format!(
        "schema_version = 3\nid = \"acme\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"data\"\n\n[provides]\ngenres = [{list}]\n"
    );
    fs::write(dir.path().join(MANIFEST_FILE), manifest).unwrap();
    for (file, text) in genres {
        fs::write(dir.path().join(file), text).unwrap();
    }
    match LoadedPack::load(&DirSource::new(dir.path()), Source::Pack) {
        Err(error) => error,
        Ok(pack) => Packs::new(vec![pack]).expect_err("the genres are invalid"),
    }
}

#[rstest]
#[case::overlong_id(vec![("g.yaml", genre(&format!("{SENTINEL}-{}", "a".repeat(200)), ""))])]
#[case::missing_bare_parent(vec![("g.yaml", genre("g", SENTINEL))])]
#[case::missing_qualified_parent(vec![("g.yaml", genre("g", &format!("acme:{SENTINEL}")))])]
#[case::missing_parent_in_unknown_pack(vec![("g.yaml", genre("g", &format!("elsewhere:{SENTINEL}")))])]
#[case::cycle(vec![
    ("a.yaml", genre(&format!("{SENTINEL}-a"), &format!("{SENTINEL}-b"))),
    ("b.yaml", genre(&format!("{SENTINEL}-b"), &format!("{SENTINEL}-a"))),
])]
fn a_genre_error_never_repeats_a_genre_id_or_parent(#[case] genres: Vec<(&str, String)>) {
    let error = load_then_register(&genres);

    for text in every_text(&error) {
        assert!(!text.contains(SENTINEL), "{text}");
    }
}

#[test]
fn a_missing_parent_names_the_file_and_line_of_extends() {
    let error = load_then_register(&[("g.yaml", genre("g", "gone"))]);

    let shown = error.to_string();

    assert!(shown.contains("g.yaml"), "{shown}");
    assert!(shown.contains("line 3"), "{shown}");
}
