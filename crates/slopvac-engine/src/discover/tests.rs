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
        ..Options::default()
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
            ..Options::default()
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
            ..Options::default()
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
            ..Options::default()
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
            ..Options::default()
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
            ..Options::default()
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
fn a_path_outside_the_root_matches_no_exclude() {
    // docs/contract/config.md: the 2.x fallback to the bare file name is removed.
    let ex = Excludes::new(Path::new("/project"), &["/README.md", "**/CHANGELOG.md"]).unwrap();
    assert!(!ex.is_excluded(Path::new("/elsewhere/README.md")));
    assert!(!ex.is_excluded(Path::new("/elsewhere/CHANGELOG.md")));
}

// --- fix round 1 regressions ---------------------------------------------

/// Rows are generated from the 2.x `pathspec` reference by
/// `scripts/discover_exclude_parity.py`.
#[test]
fn exclude_matches_the_pathspec_reference() {
    let table = include_str!("../../tests/fixtures/exclude_parity.tsv");
    let root = Path::new("/project");
    let mut failures = Vec::new();
    for line in table.lines() {
        let mut cols = line.split('\t');
        let patterns: Vec<&str> = cols.next().unwrap().split('\u{1f}').collect();
        let path = cols.next().unwrap();
        let expected = cols.next().unwrap() == "1";
        let got = Excludes::new(root, &patterns)
            .unwrap()
            .is_excluded(&root.join(path));
        if got != expected {
            failures.push(format!(
                "{patterns:?} {path}: pathspec={expected} rust={got}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_relative_root_is_stripped_once() {
    let ex = Excludes::new(Path::new("docs"), &["/docs/**"]).unwrap();
    assert!(ex.is_excluded(Path::new("docs/docs/readme.md")));
    assert!(!ex.is_excluded(Path::new("docs/readme.md")));
}

#[test]
fn a_rejected_exclude_pattern_is_an_error() {
    let err = Excludes::new(Path::new("/project"), &["[z-a]"]).unwrap_err();
    assert!(err.to_string().starts_with("invalid exclude pattern"));
}

#[test]
fn a_malformed_glob_target_is_an_error() {
    let dir = TempDir::new().unwrap();
    let pattern = format!("{}/[unclosed", dir.path().display());
    let err = discover(&[pattern.as_str()], Options::default()).unwrap_err();
    assert!(matches!(err, DiscoverError::InvalidGlob { .. }));
}

#[test]
fn a_dot_slash_relative_glob_matches_like_the_bare_form() {
    // `cargo test` runs in the package directory.
    let opts = Options {
        selection: Selection::CodeComments,
        use_gitignore: false,
        ..Options::default()
    };
    let plain = discover(&["src/discover/*.rs"], opts.clone()).unwrap();
    let prefixed = discover(&["./src/discover/*.rs"], opts).unwrap();
    let sources = fs::read_dir("src/discover")
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "rs")
        })
        .count();
    assert_eq!(plain.files.len(), sources);
    assert_eq!(prefixed, plain);
}

#[cfg(unix)]
#[test]
fn a_glob_walk_reports_an_unreadable_directory() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    write(dir.path(), "ok.md", "");
    let locked = dir.path().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let unreadable = fs::read_dir(&locked).is_err();
    let pattern = format!("{}/**/*.md", dir.path().display());
    let result = discover(&[pattern.as_str()], Options::default());
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    if !unreadable {
        return; // running as root: permissions are not enforced
    }
    assert!(matches!(result, Err(DiscoverError::Walk { .. })));
}

#[test]
fn gitignore_in_a_parent_directory_applies_to_a_nested_target() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), ".git/HEAD", "");
    write(dir.path(), ".gitignore", "docs/skip.md\n");
    write(dir.path(), "docs/keep.md", "");
    write(dir.path(), "docs/skip.md", "");
    let nested = dir.path().join("docs");
    let found = discover(
        &[nested.to_str().unwrap()],
        Options {
            selection: Selection::Prose,
            use_gitignore: true,
            ..Options::default()
        },
    )
    .unwrap();
    let got: Vec<_> = found.files.into_iter().map(|f| f.path).collect();
    assert_eq!(got, [nested.join("keep.md")]);
}
fn prose_gitignore(target: &Path) -> Vec<PathBuf> {
    let found = discover(
        &[target.to_str().unwrap()],
        Options {
            use_gitignore: true,
            ..Options::default()
        },
    )
    .unwrap();
    found.files.into_iter().map(|f| f.path).collect()
}

#[test]
fn gitignore_above_the_repository_root_is_not_applied() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), ".gitignore", "*.md\n");
    let repo = dir.path().join("repo");
    write(&repo, ".git/HEAD", "");
    write(&repo, ".gitignore", "skip.md\n");
    write(&repo, "docs/keep.md", "");
    write(&repo, "docs/skip.md", "");
    assert_eq!(
        prose_gitignore(&repo.join("docs")),
        [repo.join("docs/keep.md")]
    );
}

#[test]
fn a_git_file_marks_the_repository_root_like_a_git_directory() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), ".gitignore", "*.md\n");
    let repo = dir.path().join("repo");
    write(&repo, ".git", "gitdir: ../elsewhere\n");
    write(&repo, ".gitignore", "skip.md\n");
    write(&repo, "keep.md", "");
    write(&repo, "skip.md", "");
    assert_eq!(prose_gitignore(&repo), [repo.join("keep.md")]);
}

#[test]
fn without_a_repository_only_the_target_and_below_are_read() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), ".gitignore", "outer.md\n");
    write(dir.path(), "docs/.gitignore", "inner.md\n");
    write(dir.path(), "docs/outer.md", "");
    write(dir.path(), "docs/inner.md", "");
    write(dir.path(), "docs/keep.md", "");
    let got = prose_gitignore(&dir.path().join("docs"));
    assert_eq!(
        got,
        [
            dir.path().join("docs/keep.md"),
            dir.path().join("docs/outer.md")
        ]
    );
}

#[test]
fn git_info_exclude_applies_inside_a_repository() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), ".git/info/exclude", "private.md\n");
    write(dir.path(), "private.md", "");
    write(dir.path(), "public.md", "");
    assert_eq!(prose_gitignore(dir.path()), [dir.path().join("public.md")]);
}

// --- excludes inside discover ---------------------------------------------

fn with_excludes(root: &Path, patterns: &[&str]) -> Options {
    Options {
        excludes: Some(Excludes::new(root, patterns).unwrap()),
        ..Options::default()
    }
}

#[test]
fn walked_files_matching_an_exclude_are_dropped() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "keep.md", "");
    write(dir.path(), "CHANGELOG.md", "");
    write(dir.path(), "sub/CHANGELOG.md", "");
    let opts = with_excludes(dir.path(), &["**/CHANGELOG.md"]);
    let found = discover(&[dir.path().to_str().unwrap()], opts).unwrap();
    let got: Vec<_> = found.files.into_iter().map(|f| f.path).collect();
    assert_eq!(got, [dir.path().join("keep.md")]);
}

#[test]
fn a_negated_exclude_pattern_reincludes_a_walked_file() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.md", "");
    write(dir.path(), "keep/b.md", "");
    let opts = with_excludes(dir.path(), &["*.md", "!keep/"]);
    let found = discover(&[dir.path().to_str().unwrap()], opts).unwrap();
    let got: Vec<_> = found.files.into_iter().map(|f| f.path).collect();
    assert_eq!(got, [dir.path().join("keep/b.md")]);
}

#[test]
fn an_explicit_file_matching_an_exclude_is_skipped_with_a_reason() {
    // 2.x applied `exclude` to every collected path, explicit ones included
    // (pipeline.py `config.is_excluded`); config.md does not change that.
    let dir = TempDir::new().unwrap();
    let doc = write(dir.path(), "CHANGELOG.md", "");
    let opts = with_excludes(dir.path(), &["**/CHANGELOG.md"]);
    let found = discover(&[doc.to_str().unwrap()], opts).unwrap();
    assert_eq!(found.files.len(), 0);
    assert_eq!(found.skipped.len(), 1);
    assert_eq!(found.skipped[0].path, doc);
    assert!(found.skipped[0].reason.contains("exclude"));
}

#[test]
fn glob_matches_are_filtered_by_excludes() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.md", "");
    write(dir.path(), "b.md", "");
    let opts = with_excludes(dir.path(), &["b.md"]);
    let pattern = format!("{}/*.md", dir.path().display());
    let found = discover(&[pattern.as_str()], opts).unwrap();
    let got: Vec<_> = found.files.into_iter().map(|f| f.path).collect();
    assert_eq!(got, [dir.path().join("a.md")]);
}

#[cfg(unix)]
#[test]
fn exclude_roots_are_compared_after_resolving_symlinks() {
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("real");
    write(&real, "docs/a.md", "");
    let alias = dir.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let ex = Excludes::new(&alias, &["/docs/**"]).unwrap();
    assert!(ex.is_excluded(&real.join("docs/a.md")));
    assert!(!ex.is_excluded(&real.join("other/a.md")));
}

// --- glob traversal bounds ---------------------------------------------------

#[cfg(unix)]
mod locked {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// Run `f` with `dir` unreadable; `None` when permissions are not enforced.
    fn with_locked<T>(dir: &Path, f: impl FnOnce() -> T) -> Option<T> {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o000)).unwrap();
        let enforced = fs::read_dir(dir).is_err();
        let out = f();
        fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).unwrap();
        enforced.then_some(out)
    }

    #[test]
    fn a_single_level_glob_does_not_descend_into_subdirectories() {
        let dir = TempDir::new().unwrap();
        write(dir.path(), "docs/a.md", "");
        let locked = dir.path().join("docs/locked");
        fs::create_dir(&locked).unwrap();
        let pattern = format!("{}/docs/*.md", dir.path().display());
        let result = with_locked(&locked, || {
            discover(&[pattern.as_str()], Options::default())
        });
        if let Some(result) = result {
            let got: Vec<_> = result.unwrap().files.into_iter().map(|f| f.path).collect();
            assert_eq!(got, [dir.path().join("docs/a.md")]);
        }
    }

    #[test]
    fn directories_that_cannot_match_a_prefix_are_not_read() {
        let dir = TempDir::new().unwrap();
        write(dir.path(), "a1/x.md", "");
        let locked = dir.path().join("b1");
        fs::create_dir(&locked).unwrap();
        let pattern = format!("{}/a*/x.md", dir.path().display());
        let result = with_locked(&locked, || {
            discover(&[pattern.as_str()], Options::default())
        });
        if let Some(result) = result {
            let got: Vec<_> = result.unwrap().files.into_iter().map(|f| f.path).collect();
            assert_eq!(got, [dir.path().join("a1/x.md")]);
        }
    }

    #[test]
    fn a_brace_glob_without_recursion_does_not_descend() {
        let dir = TempDir::new().unwrap();
        write(dir.path(), "docs/a.md", "");
        write(dir.path(), "docs/b.txt", "");
        let locked = dir.path().join("docs/locked");
        fs::create_dir(&locked).unwrap();
        let pattern = format!("{}/docs/*.{{md,txt}}", dir.path().display());
        let result = with_locked(&locked, || {
            discover(&[pattern.as_str()], Options::default())
        });
        if let Some(result) = result {
            let got: Vec<_> = result.unwrap().files.into_iter().map(|f| f.path).collect();
            assert_eq!(
                got,
                [dir.path().join("docs/a.md"), dir.path().join("docs/b.txt")]
            );
        }
    }

    #[test]
    fn a_recursive_glob_still_reports_an_unreadable_directory() {
        let dir = TempDir::new().unwrap();
        write(dir.path(), "docs/a.md", "");
        let locked = dir.path().join("docs/locked");
        fs::create_dir(&locked).unwrap();
        let pattern = format!("{}/docs/**/*.md", dir.path().display());
        let result = with_locked(&locked, || {
            discover(&[pattern.as_str()], Options::default())
        });
        if let Some(result) = result {
            assert!(matches!(result, Err(DiscoverError::Walk { .. })));
        }
    }
}

#[test]
fn brace_alternatives_may_hold_separators_and_nest() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a/x.md", "");
    write(dir.path(), "b/c/x.md", "");
    write(dir.path(), "d/x.md", "");
    write(dir.path(), "b/x.md", "");
    for pattern in ["{a,b/c}/x.md", "{a,{b/c,zz}}/x.md"] {
        let full = format!("{}/{pattern}", dir.path().display());
        let found = discover(&[full.as_str()], Options::default()).unwrap();
        let got: Vec<_> = found.files.into_iter().map(|f| f.path).collect();
        assert_eq!(
            got,
            [dir.path().join("a/x.md"), dir.path().join("b/c/x.md")],
            "{pattern}"
        );
    }
}

#[test]
fn overlapping_brace_alternatives_list_each_file_once() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "a.md", "");
    let full = format!("{}/{{a,a*}}.md", dir.path().display());
    let found = discover(&[full.as_str()], Options::default()).unwrap();
    assert_eq!(found.files.len(), 1);
}

#[test]
fn too_many_brace_alternatives_are_rejected() {
    let dir = TempDir::new().unwrap();
    let groups = ["{a,b}"; 11].join("/");
    let full = format!("{}/{groups}", dir.path().display());
    let err = discover(&[full.as_str()], Options::default()).unwrap_err();
    assert!(matches!(
        err,
        DiscoverError::TooManyAlternatives { limit: 1024, .. }
    ));
}

#[test]
fn parent_traversal_in_a_target_does_not_leak_into_exclude_matching() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "drafts/wip.md", "");
    write(dir.path(), "docs/guide.md", "");
    let target = dir.path().join("drafts/../docs");
    let opts = with_excludes(dir.path(), &["drafts/"]);
    let found = discover(&[target.to_str().unwrap()], opts).unwrap();
    let got: Vec<_> = found.files.into_iter().map(|f| f.path).collect();
    assert_eq!(got, [dir.path().join("docs/guide.md")]);
}

#[test]
fn lexical_dot_dot_is_collapsed_when_the_path_does_not_exist() {
    let root = Path::new("/project");
    let ex = Excludes::new(root, &["drafts/"]).unwrap();
    assert!(!ex.is_excluded(Path::new("/project/drafts/../docs/guide.md")));
    assert!(ex.is_excluded(Path::new("/project/docs/../drafts/wip.md")));
}
