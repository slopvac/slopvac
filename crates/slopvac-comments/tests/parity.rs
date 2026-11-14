#![cfg(feature = "all-languages")]
//! Parity with the 2.x reference.
//!
//! 2.x read code comments two ways. Vale read them by line and block delimiters (no
//! grammar was installed), which also matched delimiters inside strings and dropped some
//! comments; its output is not a usable reference. The native engine read a projection
//! that blanked code and strings and kept comments (`_comment_projection` in
//! `pipeline.py`). `fixtures/parity/reference_2x.txt` holds every `ZZZ` marker that
//! projection keeps in the `sample.*` files, regenerated with
//! `fixtures/parity/generate_reference_2x.py`.
//!
//! The contract: every position 2.x kept is still extracted, and the only additions are
//! the ones listed in `extra_over_2x.txt`, each with its reason.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use slopvac_comments::{Language, extract};

type Position = (String, usize, usize);

fn dir() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/parity"
    ))
}

fn language(file: &str) -> Language {
    let name = match file.rsplit_once('.').map(|(_, ext)| ext) {
        Some("py") => "python",
        Some("rs") => "rust",
        Some("js") => "javascript",
        Some("go") => "go",
        Some("c") => "c",
        Some("rb") => "ruby",
        Some("lua") => "lua",
        Some("java") => "java",
        Some("toml") => "toml",
        other => panic!("no language for {other:?}"),
    };
    Language::from_name(name).expect("known language")
}

fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let before = &source[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .map_or(0, |tail| tail.chars().count())
        + 1;
    (line, column)
}

/// Every `ZZZ` in extracted text, as the line and column of its first source byte.
fn extracted() -> BTreeSet<Position> {
    let mut found = BTreeSet::new();
    for entry in fs::read_dir(dir()).expect("fixtures exist") {
        let path = entry.expect("entry").path();
        let name = path
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        if !name.starts_with("sample.") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("utf-8 fixture");
        for fragment in extract(language(&name), &source).expect("extracts") {
            for (at, _) in fragment.text().match_indices("ZZZ") {
                let span = fragment.projection().span_for_bytes(at, at + 3);
                assert_eq!(&source[span.start()..span.end()], "ZZZ", "{name}");
                let (line, column) = line_col(&source, span.start());
                found.insert((name.clone(), line, column));
            }
        }
    }
    found
}

/// Read `file line column` rows, skipping blank lines and `#` notes.
fn rows(name: &str) -> BTreeSet<Position> {
    let text = fs::read_to_string(dir().join(name)).expect("golden exists");
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut parts = line.split_whitespace();
            let file = parts.next().expect("file").to_owned();
            let line_no = parts.next().expect("line").parse().expect("number");
            let column = parts.next().expect("column").parse().expect("number");
            (file, line_no, column)
        })
        .collect()
}

#[test]
fn everything_2x_kept_is_still_extracted() {
    let ours = extracted();
    let lost: Vec<_> = rows("reference_2x.txt")
        .difference(&ours)
        .cloned()
        .collect();
    assert!(lost.is_empty(), "2.x kept these, 3.0 drops them: {lost:?}");
}

#[test]
fn the_only_additions_over_2x_are_the_listed_ones() {
    let ours = extracted();
    let reference = rows("reference_2x.txt");
    let added: BTreeSet<_> = ours.difference(&reference).cloned().collect();
    assert_eq!(added, rows("extra_over_2x.txt"));
}
