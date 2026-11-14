use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::{DiscoverError, Excludes, FileKind, Options, Selection, discover};

fn write(root: &Path, rel: &str, text: &str) -> PathBuf {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, text).unwrap();
    path
}

fn run(root: &Path, selection: Selection) -> Vec<PathBuf> {
    let opts = Options {
        selection,
        use_gitignore: false,
    };
    let found = discover(&[root.to_str().unwrap()], opts).unwrap();
    found.files.into_iter().map(|f| f.path).collect()
}

fn rel(root: &Path, paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|p| {
            p.strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

#[test]
fn prose_directory_walk_ignores_code_and_keeps_toml_out() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "README.md", "");
    write(dir.path(), "notes.txt", "");
    write(dir.path(), "sub/page.html", "");
    write(dir.path(), "source.py", "");
    write(dir.path(), "settings.toml", "");
    write(dir.path(), "mise.toml", "");
    let got = rel(dir.path(), &run(dir.path(), Selection::Prose));
    assert_eq!(got, ["README.md", "notes.txt", "sub/page.html"]);
}

#[test]
fn comment_directory_walk_keeps_toml_and_source_but_not_markdown() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "README.md", "");
    write(dir.path(), "source.py", "");
    write(dir.path(), "settings.toml", "");
    write(dir.path(), "mise.toml", "");
    let got = rel(dir.path(), &run(dir.path(), Selection::CodeComments));
    assert_eq!(got, ["mise.toml", "settings.toml", "source.py"]);
}

#[test]
fn comment_walk_notes_unsupported_files_but_not_prose() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "README.md", "");
    write(dir.path(), "image.bin", "");
    let found = discover(
        &[dir.path().to_str().unwrap()],
        Options {
            selection: Selection::CodeComments,
            use_gitignore: false,
        },
    )
    .unwrap();
    assert_eq!(found.files.len(), 0);
    assert_eq!(found.notes.len(), 1);
    assert!(found.notes[0].starts_with("unsupported source language skipped: "));
    assert!(found.notes[0].contains("image.bin"));
}

#[test]
fn walk_skips_build_vendor_and_dependency_directories() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "src/main.py", "");
    write(dir.path(), "vendor/third.py", "");
    write(dir.path(), "build/generated.py", "");
    write(dir.path(), "node_modules/pkg/index.js", "");
    write(dir.path(), "image.bin", "");
    let got = rel(dir.path(), &run(dir.path(), Selection::CodeComments));
    assert_eq!(got, ["src/main.py"]);
}

#[test]
fn skip_dir_names_apply_below_the_target_not_to_the_target() {
    let dir = TempDir::new().unwrap();
    let build = dir.path().join("build");
    write(&build, "doc.md", "");
    let got = run(&build, Selection::Prose);
    assert_eq!(got, [build.join("doc.md")]);
}

#[test]
fn hidden_directories_are_walked() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), ".github/PULL_REQUEST_TEMPLATE.md", "");
    let got = rel(dir.path(), &run(dir.path(), Selection::Prose));
    assert_eq!(got, [".github/PULL_REQUEST_TEMPLATE.md"]);
}

#[test]
fn files_of_a_directory_come_before_its_subdirectories() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a/inner.md", "");
    write(dir.path(), "b.md", "");
    write(dir.path(), "z.md", "");
    write(dir.path(), "c/deep/x.md", "");
    let got = rel(dir.path(), &run(dir.path(), Selection::Prose));
    assert_eq!(got, ["b.md", "z.md", "a/inner.md", "c/deep/x.md"]);
}

#[test]
fn the_project_config_file_is_never_prose_or_both_but_is_comment_text() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "slopvac.toml", "");
    write(dir.path(), ".slopvac.toml", "");
    write(dir.path(), "a.md", "");
    assert_eq!(
        rel(dir.path(), &run(dir.path(), Selection::Prose)),
        ["a.md"]
    );
    assert_eq!(rel(dir.path(), &run(dir.path(), Selection::Both)), ["a.md"]);
    assert_eq!(
        rel(dir.path(), &run(dir.path(), Selection::CodeComments)),
        [".slopvac.toml", "slopvac.toml"]
    );
}

#[test]
fn every_prose_kind_is_found_regardless_of_case_in_name_order() {
    let dir = TempDir::new().unwrap();
    for name in ["a.md", "b.MDX", "c.markdown", "d.TXT", "e.rst", "F.Html"] {
        write(dir.path(), name, "");
    }
    let found = discover(&[dir.path().to_str().unwrap()], Options::default()).unwrap();
    let kinds: Vec<_> = found.files.iter().map(|f| f.kind.clone()).collect();
    assert_eq!(
        kinds,
        [
            FileKind::Html,
            FileKind::Markdown,
            FileKind::Mdx,
            FileKind::Markdown,
            FileKind::Text,
            FileKind::Rst,
        ]
    );
}

#[test]
fn explicit_uppercase_source_extension_is_collected_in_comment_mode() {
    let dir = TempDir::new().unwrap();
    let source = write(dir.path(), "EXAMPLE.PY", "");
    let found = discover(
        &[source.to_str().unwrap()],
        Options {
            selection: Selection::CodeComments,
            use_gitignore: false,
        },
    )
    .unwrap();
    assert_eq!(found.files[0].path, source);
    assert_eq!(found.files[0].kind, FileKind::Source("python"));
}

#[test]
fn extensionless_source_names_map_to_a_language() {
    assert_eq!(
        FileKind::from_file_name("Gemfile"),
        Some(FileKind::Source("ruby"))
    );
    assert_eq!(
        FileKind::from_file_name("SConstruct"),
        Some(FileKind::Source("python"))
    );
}

#[test]
fn explicit_unsupported_file_is_an_error_in_comment_mode() {
    let dir = TempDir::new().unwrap();
    let doc = write(dir.path(), "README.md", "");
    let err = discover(
        &[doc.to_str().unwrap()],
        Options {
            selection: Selection::CodeComments,
            use_gitignore: false,
        },
    )
    .unwrap_err();
    assert!(matches!(err, DiscoverError::UnsupportedSource { .. }));
    assert!(
        err.to_string()
            .contains("unsupported source language for '.md'")
    );
}

#[test]
fn explicit_file_outside_the_selection_is_reported_not_dropped_silently() {
    let dir = TempDir::new().unwrap();
    let source = write(dir.path(), "a.py", "");
    let found = discover(&[source.to_str().unwrap()], Options::default()).unwrap();
    assert_eq!(found.files.len(), 0);
    assert_eq!(found.skipped.len(), 1);
    assert_eq!(found.skipped[0].path, source);
}

#[test]
fn a_missing_target_is_an_error() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("nope.md");
    let err = discover(&[missing.to_str().unwrap()], Options::default()).unwrap_err();
    assert!(matches!(err, DiscoverError::NoSuchTarget(_)));
}

#[test]
fn an_absolute_glob_target_matches_files_only_sorted() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "b.md", "");
    write(dir.path(), "a.md", "");
    write(dir.path(), "c.txt", "");
    write(dir.path(), "dir.md/inner.md", "");
    let pattern = format!("{}/*.md", dir.path().display());
    let found = discover(&[pattern.as_str()], Options::default()).unwrap();
    let got: Vec<_> = found.files.into_iter().map(|f| f.path).collect();
    assert_eq!(got, [dir.path().join("a.md"), dir.path().join("b.md")]);
}

#[test]
fn a_glob_with_no_match_is_an_error() {
    let dir = TempDir::new().unwrap();
    let pattern = format!("{}/*.md", dir.path().display());
    let err = discover(&[pattern.as_str()], Options::default()).unwrap_err();
    assert!(matches!(err, DiscoverError::NoSuchTarget(_)));
}

#[test]
fn a_file_named_twice_is_listed_once() {
    let dir = TempDir::new().unwrap();
    let doc = write(dir.path(), "a.md", "");
    let found = discover(
        &[dir.path().to_str().unwrap(), doc.to_str().unwrap()],
        Options::default(),
    )
    .unwrap();
    assert_eq!(found.files.len(), 1);
}

#[test]
fn rst_is_discovered_without_any_converter_check() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "guide.rst", "");
    let found = discover(&[dir.path().to_str().unwrap()], Options::default()).unwrap();
    assert_eq!(found.files[0].kind, FileKind::Rst);
}

// --- .gitignore ---------------------------------------------------------

fn gitignore_run(dir: &TempDir, use_gitignore: bool) -> Vec<String> {
    let found = discover(
        &[dir.path().to_str().unwrap()],
        Options {
            selection: Selection::Prose,
            use_gitignore,
        },
    )
    .unwrap();
    let paths: Vec<_> = found.files.into_iter().map(|f| f.path).collect();
    rel(dir.path(), &paths)
}

fn gitignore_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    write(dir.path(), ".gitignore", "drafts/\nscratch.md\n");
    write(dir.path(), "keep.md", "");
    write(dir.path(), "scratch.md", "");
    write(dir.path(), "drafts/wip.md", "");
    dir
}

#[test]
fn gitignore_is_honoured_when_the_setting_is_on() {
    let dir = gitignore_fixture();
    assert_eq!(gitignore_run(&dir, true), ["keep.md"]);
}

#[test]
fn gitignore_is_not_read_when_the_setting_is_off() {
    let dir = gitignore_fixture();
    assert_eq!(
        gitignore_run(&dir, false),
        ["keep.md", "scratch.md", "drafts/wip.md"]
    );
}

#[test]
fn an_explicit_file_is_linted_even_when_gitignored() {
    let dir = gitignore_fixture();
    let ignored = dir.path().join("scratch.md");
    let found = discover(
        &[ignored.to_str().unwrap()],
        Options {
            selection: Selection::Prose,
            use_gitignore: true,
        },
    )
    .unwrap();
    assert_eq!(found.files.len(), 1);
}

// --- symlinks -----------------------------------------------------------

#[cfg(unix)]
mod symlinks {
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn walks_neither_list_nor_enter_symlinks() {
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        write(outside.path(), "ext.md", "");
        write(dir.path(), "real.md", "");
        symlink(dir.path().join("real.md"), dir.path().join("link.md")).unwrap();
        symlink(outside.path(), dir.path().join("linkdir")).unwrap();
        let got = rel(dir.path(), &run(dir.path(), Selection::Prose));
        assert_eq!(got, ["real.md"]);
    }

    #[test]
    fn a_symlink_named_as_a_target_is_accepted() {
        let dir = TempDir::new().unwrap();
        let real = write(dir.path(), "real.md", "");
        let link = dir.path().join("link.md");
        symlink(&real, &link).unwrap();
        let found = discover(&[link.to_str().unwrap()], Options::default()).unwrap();
        assert_eq!(found.files[0].path, link);
    }

    #[test]
    fn a_symlinked_directory_named_as_a_target_is_walked() {
        let dir = TempDir::new().unwrap();
        write(dir.path(), "docs/a.md", "");
        let link = dir.path().join("link");
        symlink(dir.path().join("docs"), &link).unwrap();
        let got = run(&link, Selection::Prose);
        assert_eq!(got, [link.join("a.md")]);
    }
}

// --- excludes -----------------------------------------------------------

#[test]
fn exclude_patterns_use_gitignore_semantics_relative_to_the_root() {
    let root = Path::new("/project");
    let ex = Excludes::new(
        root,
        &[
            "**/node_modules/**",
            "**/CHANGELOG.md",
            "**/.git/**",
            "drafts/",
        ],
    )
    .unwrap();
    assert!(ex.is_excluded(&root.join("web/node_modules/x/readme.md")));
    assert!(ex.is_excluded(&root.join("CHANGELOG.md")));
    assert!(ex.is_excluded(&root.join("drafts/a.md")));
    assert!(!ex.is_excluded(&root.join("docs/guide.md")));
}

#[test]
fn a_path_outside_the_root_is_excluded_by_file_name() {
    let ex = Excludes::new(Path::new("/project"), &["**/CHANGELOG.md"]).unwrap();
    assert!(ex.is_excluded(Path::new("/elsewhere/CHANGELOG.md")));
}
