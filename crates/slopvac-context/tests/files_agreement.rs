//! The `files` patterns of a `[[genres]]` block select the same documents in the config
//! stack and in genre resolution, because both use one matcher on the native path.

use std::path::Path;

use rstest::rstest;
use slopvac_config::{Inputs, NoPresets, load};
use slopvac_context::{
    GenreAssignment, GenreRegistry, PlainDocument, ResolveOptions, parse_genres,
};

/// Whether config resolution and genre resolution each assign `hit` to `path`, for one
/// `[[genres]]` block with the given `files`. The patterns are written into the TOML as
/// literal strings, so a backslash reaches the matcher as typed.
fn assigned(files: &[&str], path: &str) -> (bool, bool) {
    let list = files
        .iter()
        .map(|f| format!("'{f}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("slopvac.toml");
    std::fs::write(
        &file,
        format!("schema_version = 3\n\n[[genres]]\nfiles = [{list}]\ngenre = \"hit\"\n"),
    )
    .unwrap();
    let config = load(&file, &NoPresets).unwrap();
    let inputs = Inputs {
        presets: &NoPresets,
        front_matter: &[],
        directives: &[],
        cli: &[],
        env: &[],
    };
    let from_config = config
        .resolve(Path::new(path), &inputs)
        .get(&["genre"])
        .and_then(|v| v.as_str().map(str::to_owned))
        == Some("hit".to_owned());

    let assignments: Vec<GenreAssignment> = config
        .genre_assignments()
        .into_iter()
        .map(|(files, genre)| GenreAssignment {
            files: files.to_vec(),
            genre: genre.to_owned(),
        })
        .collect();
    let registry = GenreRegistry::new(
        parse_genres("- {schema_version: 3, id: base}\n- {schema_version: 3, id: hit}\n").unwrap(),
        "base",
    )
    .unwrap();
    let opts = ResolveOptions {
        assignments: &assignments,
        ..Default::default()
    };
    let from_context = registry
        .resolve(&PlainDocument::from_markdown(path, "t\n"), &opts)
        .unwrap()
        .id
        == "hit";
    (from_config, from_context)
}

// Expected values are worked out from gitignore semantics, so agreement is not agreement on
// a shared bug.
#[rstest]
#[case::plain(&["docs/**"], "docs/a.md", true)]
#[case::nested(&["docs/**"], "docs/guide/deep/b.md", true)]
#[case::negated(&["docs/**", "!docs/internal/**"], "docs/internal/x.md", false)]
#[case::negated_nested(&["docs/**", "!docs/internal/**"], "docs/internal/sub/y.md", false)]
#[case::re_included(&["docs/**", "!docs/internal/**", "docs/internal/keep.md"], "docs/internal/keep.md", true)]
#[case::unanchored(&["**/vendor/*.md"], "src/vendor/z.md", true)]
#[case::anchored(&["/top.md"], "sub/top.md", false)]
#[case::backslash_is_not_a_separator(&["guide/branch/**"], r"guide\branch/x.md", false)]
#[case::escaped_backslash_is_literal(&[r"guide\\branch/**"], r"guide\branch/x.md", true)]
#[case::leading_space_is_part_of_the_pattern(&[" docs/**"], "docs/a.md", false)]
#[case::leading_space_names_a_space_directory(&[" docs/**"], " docs/a.md", true)]
#[case::parent_path_is_outside_the_root(&["**/a.md"], "../docs/a.md", false)]
#[case::absolute_path_is_outside_the_root(&["**/a.md"], "/proj/docs/a.md", false)]
fn config_and_genre_resolution_agree(
    #[case] files: &[&str],
    #[case] path: &str,
    #[case] expected: bool,
) {
    assert_eq!(
        assigned(files, path),
        (expected, expected),
        "{files:?} on {path}"
    );
}
