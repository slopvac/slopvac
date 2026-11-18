//! The `files` patterns of a `[[genres]]` block select the same documents in the config
//! stack and in genre resolution, because both use one matcher.

use slopvac_config::{Inputs, NoPresets, load};
use slopvac_context::{
    GenreAssignment, GenreRegistry, PlainDocument, ResolveOptions, parse_genres,
};
use std::path::Path;

const CONFIG: &str = r#"schema_version = 3

[[genres]]
files = ["docs/**", "!docs/internal/**", "docs/internal/keep.md"]
genre = "hit"

[[genres]]
files = ["**/vendor/*.md", "!vendor/*.md"]
genre = "other"

[[genres]]
files = ["/top.md"]
genre = "top"
"#;

const PATHS: [&str; 10] = [
    "docs/a.md",
    "docs/guide/deep/b.md",
    "docs/internal/x.md",
    "docs/internal/keep.md",
    "docs/internal/sub/y.md",
    "src/vendor/z.md",
    "vendor/z.md",
    "top.md",
    "sub/top.md",
    "unrelated.md",
];

#[test]
fn config_and_genre_resolution_agree_on_negated_and_nested_patterns() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("slopvac.toml");
    std::fs::write(&file, CONFIG).unwrap();
    let config = load(&file, &NoPresets).unwrap();
    let assignments: Vec<GenreAssignment> = config
        .genre_assignments()
        .into_iter()
        .map(|(files, genre)| GenreAssignment {
            files: files.to_vec(),
            genre: genre.to_owned(),
        })
        .collect();
    let registry = GenreRegistry::new(
        parse_genres(
            "- {schema_version: 3, id: base}\n- {schema_version: 3, id: hit}\n- {schema_version: 3, id: other}\n- {schema_version: 3, id: top}\n",
        )
        .unwrap(),
        "base",
    )
    .unwrap();
    let inputs = Inputs {
        presets: &NoPresets,
        front_matter: &[],
        directives: &[],
        cli: &[],
        env: &[],
    };
    for path in PATHS {
        let from_config = config
            .resolve(Path::new(path), &inputs)
            .get(&["genre"])
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "base".to_owned());
        let opts = ResolveOptions {
            assignments: &assignments,
            relative_path: None,
        };
        let from_context = registry
            .resolve(&PlainDocument::from_markdown(path, "t\n"), &opts)
            .unwrap()
            .id;
        assert_eq!(from_context, from_config, "{path}");
    }
    // Worked out by hand from gitignore semantics, so agreement is not agreement on a bug.
    let expected = [
        ("docs/a.md", "hit"),
        ("docs/internal/x.md", "base"),
        ("docs/internal/keep.md", "hit"),
        ("src/vendor/z.md", "other"),
        ("vendor/z.md", "base"),
        ("top.md", "top"),
        ("sub/top.md", "base"),
    ];
    for (path, want) in expected {
        let opts = ResolveOptions {
            assignments: &assignments,
            relative_path: None,
        };
        let got = registry
            .resolve(&PlainDocument::from_markdown(path, "t\n"), &opts)
            .unwrap()
            .id;
        assert_eq!(got, want, "{path}");
    }
}
