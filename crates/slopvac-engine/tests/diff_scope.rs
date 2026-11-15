//! Changed-line scope against real git repositories.
//!
//! Every test builds a throwaway repository in a temp directory and drives the public API.
//! Expected ranges are worked out from the file contents written below, not from the code.
//! Tests return early when `git` is not on `PATH`.

// Lists of exactly one range are the common case under test.
#![allow(clippy::single_range_in_vec_init)]

use std::ffi::OsStr;
use std::fs;
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::process::Command;

use slopvac_engine::diff_scope::{
    ChangedLines, DiffError, DiffScope, Limits, changed_lines, changed_lines_with, finding_fixable,
    finding_in_scope, retain_in_scope,
};
use slopvac_model::Finding;
use tempfile::TempDir;

struct Repo {
    _dir: TempDir,
    /// Canonical path of the work tree.
    root: PathBuf,
    config: PathBuf,
}

impl Repo {
    fn new() -> Option<Self> {
        Command::new("git").arg("--version").output().ok()?;
        let dir = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        let root = base.join("repo");
        fs::create_dir(&root).unwrap();
        let config = base.join("gitconfig");
        fs::write(&config, "").unwrap();
        let repo = Self {
            _dir: dir,
            root,
            config,
        };
        repo.git(&["init", "-q"]);
        for (key, value) in [
            ("user.email", "test@example.com"),
            ("user.name", "Slopvac Tests"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
            ("core.symlinks", "true"),
        ] {
            repo.git(&["config", key, value]);
        }
        Some(repo)
    }

    fn git<S: AsRef<OsStr>>(&self, args: &[S]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env("GIT_CONFIG_GLOBAL", &self.config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn write(&self, name: &str, content: impl AsRef<[u8]>) {
        let path = self.path(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn commit(&self, message: &str) -> String {
        self.git(&["add", "--all"]);
        self.git(&["commit", "-qm", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    fn lines(&self, scope: &DiffScope) -> ChangedLines {
        changed_lines(&self.root, scope).unwrap()
    }

    fn ranges(&self, changed: &ChangedLines, name: &str) -> Vec<RangeInclusive<u32>> {
        changed.ranges_for(&self.path(name)).unwrap()
    }
}

fn since(rev: &str) -> DiffScope {
    DiffScope::Base(rev.to_owned())
}

macro_rules! repo {
    () => {
        match Repo::new() {
            Some(repo) => repo,
            None => return,
        }
    };
}

#[test]
fn working_tree_unions_staged_and_unstaged_changes() {
    let repo = repo!();
    repo.write("staged.md", "base\n");
    repo.write("unstaged.md", "base\n");
    repo.write("untouched.md", "base\n");
    repo.commit("base");
    repo.write("staged.md", "changed staged\n");
    repo.git(&["add", "staged.md"]);
    repo.write("unstaged.md", "changed unstaged\n");

    let changed = repo.lines(&DiffScope::WorkingTree);

    assert_eq!(repo.ranges(&changed, "staged.md"), vec![1..=1]);
    assert_eq!(repo.ranges(&changed, "unstaged.md"), vec![1..=1]);
    assert_eq!(repo.ranges(&changed, "untouched.md"), vec![]);
    assert_eq!(changed.files().len(), 2);
}

#[test]
fn working_tree_covers_a_file_edited_after_it_was_staged() {
    let repo = repo!();
    repo.write("doc.md", "one\ntwo\nthree\nfour\n");
    repo.commit("base");
    repo.write("doc.md", "ONE\ntwo\nthree\nfour\n");
    repo.git(&["add", "doc.md"]);
    repo.write("doc.md", "ONE\ntwo\nthree\nFOUR\n");

    let changed = repo.lines(&DiffScope::WorkingTree);

    assert_eq!(repo.ranges(&changed, "doc.md"), vec![1..=1, 4..=4]);
}

#[test]
fn working_tree_marks_an_untracked_file_as_changed_in_every_line() {
    let repo = repo!();
    repo.write("tracked.md", "base\n");
    repo.commit("base");
    repo.write("new.md", "first\nsecond\n");
    repo.write("ignored.md", "x\n");
    repo.write(".gitignore", "ignored.md\n");

    let changed = repo.lines(&DiffScope::WorkingTree);

    assert!(changed.is_whole_file(&repo.path("new.md")).unwrap());
    assert_eq!(repo.ranges(&changed, "new.md"), vec![1..=u32::MAX]);
    assert!(!changed.is_changed(&repo.path("ignored.md")).unwrap());
    assert!(!changed.is_whole_file(&repo.path("tracked.md")).unwrap());
}

#[test]
fn base_scope_reports_lines_the_branch_added_since_the_base() {
    let repo = repo!();
    repo.write("doc.md", "keep\nclean base\nkeep\n");
    let base = repo.commit("base");
    repo.write("doc.md", "keep\nclean base\nkeep\nnew line\nanother\n");
    repo.commit("branch");

    let changed = repo.lines(&since(&base));

    assert_eq!(repo.ranges(&changed, "doc.md"), vec![4..=5]);
}

#[test]
fn base_scope_ignores_uncommitted_work() {
    let repo = repo!();
    repo.write("doc.md", "one\n");
    let base = repo.commit("base");
    repo.write("doc.md", "one\nuncommitted\n");
    repo.write("untracked.md", "x\n");

    let changed = repo.lines(&since(&base));

    assert!(changed.files().is_empty());
}

#[test]
fn base_scope_compares_against_the_merge_base() {
    let repo = repo!();
    repo.write("doc.md", "a\nb\n");
    let fork = repo.commit("fork point");
    repo.git(&["checkout", "-qb", "feature"]);
    repo.write("doc.md", "a\nb\nfeature line\n");
    repo.commit("feature");
    repo.git(&["checkout", "-q", "-"]);
    repo.write("other.md", "main-only\n");
    repo.commit("main moves on");
    let main = repo.git(&["rev-parse", "HEAD"]);
    repo.git(&["checkout", "-q", "feature"]);
    assert_ne!(fork, main);

    let changed = repo.lines(&since(&main));

    assert_eq!(repo.ranges(&changed, "doc.md"), vec![3..=3]);
    assert!(!changed.is_changed(&repo.path("other.md")).unwrap());
}

#[test]
fn base_scope_accepts_a_branch_name_and_a_new_file() {
    let repo = repo!();
    repo.write("existing.md", "clean\n");
    repo.commit("base");
    repo.git(&["branch", "release"]);
    repo.write("new.md", "one\ntwo\nthree\n");
    repo.commit("add document");

    let changed = repo.lines(&since("release"));

    assert_eq!(repo.ranges(&changed, "new.md"), vec![1..=3]);
    assert!(!changed.is_whole_file(&repo.path("new.md")).unwrap());
}

#[test]
fn a_rename_with_edits_scopes_the_new_name_only() {
    let repo = repo!();
    repo.write("old.md", "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n");
    let base = repo.commit("base");
    repo.git(&["mv", "old.md", "new.md"]);
    repo.write("new.md", "l1\nl2\nl3\nCHANGED\nl5\nl6\nl7\nl8\n");
    repo.commit("rename and edit");

    let changed = repo.lines(&since(&base));

    assert_eq!(repo.ranges(&changed, "new.md"), vec![4..=4]);
    assert!(!changed.is_changed(&repo.path("old.md")).unwrap());
}

#[test]
fn a_pure_rename_has_no_changed_lines() {
    let repo = repo!();
    repo.write("old.md", "l1\nl2\nl3\n");
    let base = repo.commit("base");
    repo.git(&["mv", "old.md", "new.md"]);
    repo.commit("rename");

    assert!(repo.lines(&since(&base)).files().is_empty());
}

#[test]
fn binary_and_deleted_files_have_no_changed_lines() {
    let repo = repo!();
    repo.write("gone.md", "text\n");
    let base = repo.commit("base");
    fs::remove_file(repo.path("gone.md")).unwrap();
    repo.write("image.bin", [0u8, 159, 146, 150, 0, 1, 2]);
    repo.commit("delete and add binary");

    assert!(repo.lines(&since(&base)).files().is_empty());
}

#[test]
fn a_changed_submodule_pointer_is_not_a_changed_file() {
    let repo = repo!();
    repo.write("doc.md", "x\n");
    let base = repo.commit("base");
    let first = "1".repeat(40);
    let second = "2".repeat(40);
    repo.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{first},sub"),
    ]);
    repo.git(&["commit", "-qm", "add gitlink"]);
    repo.git(&[
        "update-index",
        "--cacheinfo",
        &format!("160000,{second},sub"),
    ]);
    repo.git(&["commit", "-qm", "move gitlink"]);

    assert!(repo.lines(&since(&base)).files().is_empty());
}

#[test]
fn crlf_lines_count_once_and_only_changed_lines_are_reported() {
    let repo = repo!();
    repo.write("doc.md", b"one\r\ntwo\r\nthree\r\n");
    let base = repo.commit("base");
    repo.write("doc.md", b"one\r\nTWO\r\nthree\r\nfour");
    repo.commit("edit");

    let changed = repo.lines(&since(&base));

    assert_eq!(repo.ranges(&changed, "doc.md"), vec![2..=2, 4..=4]);
}

#[test]
fn awkward_file_names_are_decoded() {
    let repo = repo!();
    let names = [
        "release notes.md",
        "caf\u{e9}.md",
        "dir with space/\u{4e2d}\u{6587}.md",
    ];
    repo.write("anchor.md", "x\n");
    let base = repo.commit("base");
    for name in names {
        repo.write(name, "one\ntwo\n");
    }
    repo.commit("add");

    let changed = repo.lines(&since(&base));

    for name in names {
        assert_eq!(repo.ranges(&changed, name), vec![1..=2], "{name}");
    }
}

#[cfg(unix)]
#[test]
fn quoted_file_names_are_decoded() {
    let repo = repo!();
    let names = ["say \"hi\".md", "tab\there.md", "back\\slash.md"];
    repo.write("anchor.md", "x\n");
    let base = repo.commit("base");
    for name in names {
        repo.write(name, "one\n");
    }
    repo.commit("add");

    let changed = repo.lines(&since(&base));

    for name in names {
        assert_eq!(repo.ranges(&changed, name), vec![1..=1], "{name}");
    }
}

#[test]
fn the_scope_resolves_from_a_subdirectory() {
    let repo = repo!();
    repo.write("docs/guide.md", "a\n");
    let base = repo.commit("base");
    repo.write("docs/guide.md", "a\nb\n");
    repo.write("top.md", "t\n");
    repo.commit("edit");

    let changed = changed_lines(&repo.path("docs"), &since(&base)).unwrap();

    assert_eq!(changed.root(), repo.root);
    assert_eq!(repo.ranges(&changed, "docs/guide.md"), vec![2..=2]);
    assert_eq!(repo.ranges(&changed, "top.md"), vec![1..=1]);
}

#[test]
fn an_invalid_base_is_an_error() {
    let repo = repo!();
    repo.write("doc.md", "x\n");
    repo.commit("base");

    for base in ["no-such-branch", "", "--output=owned"] {
        let err = changed_lines(&repo.root, &since(base)).unwrap_err();
        assert!(matches!(err, DiffError::InvalidBase(_)), "{base:?}: {err}");
    }
    assert!(!repo.path("owned").exists());
}

#[test]
fn a_base_that_is_not_a_commit_is_an_error() {
    let repo = repo!();
    repo.write("doc.md", "x\n");
    repo.commit("base");
    let blob = repo.git(&["rev-parse", "HEAD:doc.md"]);

    let err = changed_lines(&repo.root, &since(&blob)).unwrap_err();

    assert!(matches!(err, DiffError::InvalidBase(_)), "{err}");
}

#[test]
fn a_directory_outside_a_repository_is_an_error() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();

    let err = changed_lines(dir.path(), &DiffScope::WorkingTree).unwrap_err();

    assert!(matches!(err, DiffError::NotRepository(_)), "{err}");
}

#[test]
fn a_repository_without_a_commit_is_an_error() {
    let repo = repo!();
    repo.write("doc.md", "x\n");

    let err = changed_lines(&repo.root, &DiffScope::WorkingTree).unwrap_err();

    assert!(matches!(err, DiffError::Git(_)), "{err}");
}

#[cfg(unix)]
mod symlinks {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn a_changed_symlink_fails_only_when_it_is_asked_about() {
        let repo = repo!();
        repo.write("AGENTS.md", "rules\n");
        repo.write("doc.md", "base\n");
        let base = repo.commit("base");
        symlink("AGENTS.md", repo.path("CLAUDE.md")).unwrap();
        repo.write("doc.md", "base\nnew\n");
        repo.commit("add symlink");

        let changed = repo.lines(&since(&base));

        assert_eq!(repo.ranges(&changed, "doc.md"), vec![2..=2]);
        assert!(matches!(
            changed.ranges_for(&repo.path("CLAUDE.md")),
            Err(DiffError::Symlink(_))
        ));
    }

    #[test]
    fn a_working_tree_symlink_over_a_tracked_file_fails_when_it_is_asked_about() {
        let repo = repo!();
        repo.write("doc.md", "base\n");
        repo.commit("base");
        let external = repo.root.parent().unwrap().join("external.md");
        fs::write(&external, "outside\n").unwrap();
        fs::remove_file(repo.path("doc.md")).unwrap();
        symlink(&external, repo.path("doc.md")).unwrap();

        let changed = repo.lines(&DiffScope::WorkingTree);

        assert!(matches!(
            changed.is_changed(&repo.path("doc.md")),
            Err(DiffError::Symlink(_))
        ));
    }

    #[test]
    fn an_untracked_symlink_fails_when_it_is_asked_about() {
        let repo = repo!();
        repo.write("doc.md", "base\n");
        repo.commit("base");
        symlink("doc.md", repo.path("link.md")).unwrap();

        let changed = repo.lines(&DiffScope::WorkingTree);

        assert!(matches!(
            changed.ranges_for(&repo.path("link.md")),
            Err(DiffError::Symlink(_))
        ));
        assert_eq!(repo.ranges(&changed, "doc.md"), vec![]);
    }

    #[test]
    fn a_deleted_symlink_is_not_an_error() {
        let repo = repo!();
        repo.write("AGENTS.md", "rules\n");
        symlink("AGENTS.md", repo.path("CLAUDE.md")).unwrap();
        let base = repo.commit("base");
        fs::remove_file(repo.path("CLAUDE.md")).unwrap();
        repo.commit("delete symlink");

        let changed = repo.lines(&since(&base));

        assert!(changed.files().is_empty());
        assert_eq!(repo.ranges(&changed, "CLAUDE.md"), vec![]);
    }

    #[test]
    fn a_file_below_a_symlinked_directory_is_never_followed() {
        let repo = repo!();
        repo.write("real/doc.md", "x\n");
        repo.commit("base");
        symlink("real", repo.path("alias")).unwrap();

        let changed = repo.lines(&DiffScope::WorkingTree);

        assert!(matches!(
            changed.ranges_for(&repo.path("alias")),
            Err(DiffError::Symlink(_))
        ));
        assert!(changed.files().is_empty());
    }
}

/// A repository-controlled program must not run while git computes the diff. Each case
/// plants a program through a different channel and checks that it left no marker file.
#[cfg(unix)]
mod hardening {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    /// A script that records that it ran and, when used as a diff driver, prints nothing.
    fn plant(repo: &Repo) -> (PathBuf, PathBuf) {
        let marker = repo.root.parent().unwrap().join("ran");
        let script = repo.root.parent().unwrap().join("evil.sh");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\ntouch '{}'\ncat \"$1\" 2>/dev/null\n",
                marker.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        (script, marker)
    }

    fn exercise(repo: &Repo, marker: &Path) {
        repo.write("doc.md", "one\n");
        let base = repo.commit("base");
        repo.write("doc.md", "one\ntwo\n");
        repo.commit("edit");
        repo.write("doc.md", "one\ntwo\nthree\n");
        repo.write("untracked.md", "u\n");
        // The setup's own git calls are not hardened and may have run the program.
        let _ = fs::remove_file(marker);

        let committed = repo.lines(&since(&base));
        let working = repo.lines(&DiffScope::WorkingTree);

        assert_eq!(repo.ranges(&committed, "doc.md"), vec![2..=2]);
        assert_eq!(repo.ranges(&working, "doc.md"), vec![3..=3]);
        assert!(!marker.exists(), "a repository program ran");
    }

    #[test]
    fn an_external_diff_program_does_not_run() {
        let repo = repo!();
        let (script, marker) = plant(&repo);
        repo.git(&["config", "diff.external", script.to_str().unwrap()]);
        exercise(&repo, &marker);
    }

    #[test]
    fn a_textconv_program_does_not_run() {
        let repo = repo!();
        let (script, marker) = plant(&repo);
        repo.write(".gitattributes", "*.md diff=evil\n");
        repo.git(&["config", "diff.evil.textconv", script.to_str().unwrap()]);
        exercise(&repo, &marker);
    }

    #[test]
    fn an_fsmonitor_hook_does_not_run() {
        let repo = repo!();
        let (script, marker) = plant(&repo);
        repo.git(&["config", "core.fsmonitor", script.to_str().unwrap()]);
        exercise(&repo, &marker);
    }

    #[test]
    fn user_diff_configuration_does_not_change_the_ranges() {
        let repo = repo!();
        repo.write("doc.md", "a\nb\nc\nd\ne\nf\ng\n");
        let base = repo.commit("base");
        repo.write("doc.md", "A\nb\nc\nd\ne\nf\nG\n");
        repo.commit("edit");
        for (key, value) in [
            ("diff.noprefix", "true"),
            ("diff.mnemonicPrefix", "true"),
            ("diff.context", "5"),
            ("diff.interHunkContext", "10"),
            ("diff.relative", "true"),
            ("color.ui", "always"),
            ("core.quotepath", "true"),
        ] {
            repo.git(&["config", key, value]);
        }

        let changed = changed_lines(&repo.path("."), &since(&base)).unwrap();

        assert_eq!(repo.ranges(&changed, "doc.md"), vec![1..=1, 7..=7]);
    }
}

/// Build a finding spanning `start_line..=end_line`; a fix is present when `fix` is true.
fn finding(start_line: usize, end_line: usize, matched: &str, fix: bool) -> Finding {
    let mut value = serde_json::json!({
        "rule_id": "prose-test:rule",
        "pack": "prose-test",
        "category": "test",
        "severity": "warning",
        "message": "m",
        "line": start_line,
        "column": 1,
        "contexts": [],
        "ai_signal": "none",
        "matched_text": matched,
    });
    if !matched.is_empty() {
        value["span"] = serde_json::json!({"start": 0, "end": matched.len()});
        value["end_line"] = serde_json::json!(end_line);
        value["end_column"] = serde_json::json!(2);
    }
    if fix {
        value["fix"] =
            serde_json::json!({"edits": [{"span": {"start": 0, "end": 1}, "text": "x"}]});
    }
    serde_json::from_value(value).unwrap()
}

#[test]
fn a_finding_is_kept_when_any_line_of_its_span_is_changed() {
    let ranges = [5..=6];
    let cases = [
        (4, 4, false),
        (4, 5, true),
        (5, 5, true),
        (6, 6, true),
        (6, 8, true),
        (4, 8, true),
        (7, 9, false),
        (1, 3, false),
    ];
    for (first, last, expected) in cases {
        let f = finding(first, last, "bad", false);
        assert_eq!(finding_in_scope(&f, &ranges), expected, "{first}..={last}");
    }
}

#[test]
fn a_finding_without_a_source_range_is_never_in_scope() {
    let f = finding(5, 5, "", false);

    assert!(!finding_in_scope(&f, &[1..=u32::MAX]));
}

#[test]
fn retain_in_scope_drops_findings_outside_the_ranges() {
    let mut findings = vec![
        finding(1, 1, "bad", false),
        finding(8, 8, "bad", false),
        finding(2, 2, "", false),
        finding(2, 3, "bad", false),
    ];

    retain_in_scope(&mut findings, &[2..=2]);

    let lines: Vec<usize> = findings.iter().map(|f| f.start.line()).collect();
    assert_eq!(lines, vec![2]);
    assert!(findings[0].extent.is_some());
}

#[test]
fn a_fix_across_an_unchanged_line_is_reported_but_not_fixable() {
    let f = finding(2, 3, "bad\nold", true);
    let ranges = [2..=2];

    assert!(finding_in_scope(&f, &ranges));
    assert!(!finding_fixable(&f, &ranges));
    assert!(finding_fixable(&f, &[2..=3]));
    assert!(finding_fixable(&f, &[2..=2, 3..=3]));
}

#[test]
fn a_finding_without_a_fix_is_not_fixable() {
    let f = finding(2, 2, "bad", false);

    assert!(!finding_fixable(&f, &[1..=9]));
}

#[test]
fn an_empty_new_file_has_no_changed_lines() {
    let repo = repo!();
    repo.write("anchor.md", "x\n");
    let base = repo.commit("base");
    repo.write("empty.md", "");
    repo.commit("add empty file");

    assert!(repo.lines(&since(&base)).files().is_empty());
}

#[cfg(unix)]
#[test]
fn a_mode_only_change_has_no_changed_lines() {
    use std::os::unix::fs::PermissionsExt;

    let repo = repo!();
    repo.write("run.md", "x\n");
    let base = repo.commit("base");
    fs::set_permissions(repo.path("run.md"), fs::Permissions::from_mode(0o755)).unwrap();
    repo.commit("make executable");

    assert!(repo.lines(&since(&base)).files().is_empty());
}

#[test]
fn a_pure_deletion_inside_a_file_leaves_no_range() {
    let repo = repo!();
    repo.write("doc.md", "a\nb\nc\nd\n");
    let base = repo.commit("base");
    repo.write("doc.md", "a\nd\n");
    repo.commit("delete b and c");

    assert!(repo.lines(&since(&base)).files().is_empty());
}

#[test]
fn separated_edits_give_separate_ranges() {
    let repo = repo!();
    repo.write("doc.md", "a\nb\nc\nd\ne\n");
    let base = repo.commit("base");
    repo.write("doc.md", "A\nb\nc\nd\nE\nnew\n");
    repo.commit("edit");

    let changed = repo.lines(&since(&base));

    assert_eq!(repo.ranges(&changed, "doc.md"), vec![1..=1, 5..=6]);
}

#[test]
fn changed_text_that_looks_like_patch_headers_is_still_content() {
    let repo = repo!();
    repo.write("doc.md", "-- a/other.md\nkeep\n++ b/other.md\n");
    let base = repo.commit("base");
    repo.write("doc.md", "- a/other.md\nkeep\n+ b/other.md\n");
    repo.commit("edit");

    let changed = repo.lines(&since(&base));

    assert_eq!(repo.ranges(&changed, "doc.md"), vec![1..=1, 3..=3]);
    assert!(!changed.is_changed(&repo.path("other.md")).unwrap());
}

#[test]
fn a_form_feed_before_header_like_text_does_not_split_the_patch() {
    let repo = repo!();
    repo.write("doc.md", "keep\nold\n");
    let base = repo.commit("base");
    repo.write(
        "doc.md",
        "keep\nnote\x0cdiff --git a/ghost.md b/ghost.md\n--- a/ghost.md\n",
    );
    repo.commit("edit");

    let changed = repo.lines(&since(&base));

    assert_eq!(repo.ranges(&changed, "doc.md"), vec![2..=3]);
    assert!(!changed.is_changed(&repo.path("ghost.md")).unwrap());
}

#[test]
fn a_lone_cr_shifts_source_lines_but_not_git_lines() {
    let repo = repo!();
    // Source lines: one=1, two=2, three=3, four=4. Git counts only LF: 3 lines.
    repo.write("doc.md", "one\rtwo\nthree\r\nfour\n");
    let base = repo.commit("base");
    repo.write("doc.md", "one\rtwo\nthree\r\nFOUR\n");
    repo.commit("edit the last line");

    let changed = repo.lines(&since(&base));
    let ranges = repo.ranges(&changed, "doc.md");

    assert_eq!(ranges, vec![4..=4]);
    let mut findings = vec![finding(3, 3, "bad", false), finding(4, 4, "bad", false)];
    retain_in_scope(&mut findings, &ranges);
    let lines: Vec<usize> = findings.iter().map(|f| f.start.line()).collect();
    assert_eq!(lines, vec![4]);
}

#[test]
fn a_git_line_holding_a_lone_cr_covers_every_source_line_it_spans() {
    let repo = repo!();
    // Git line 1 is `one\rtwo` (source lines 1 and 2); git line 2 is `three`.
    repo.write("doc.md", "one\rtwo\nthree\nfour\n");
    let base = repo.commit("base");
    repo.write("doc.md", "one\rTWO\nthree\nfour\n");
    repo.commit("edit");

    let changed = repo.lines(&since(&base));

    assert_eq!(repo.ranges(&changed, "doc.md"), vec![1..=2]);
}

#[test]
fn mixed_line_terminators_map_each_edit_to_its_source_line() {
    let repo = repo!();
    // Source lines: a=1, b=2, c=3, d=4, e=5. Git lines: `a\r`=1, `b\rc`=2, `d\r`=3, `e`=4.
    repo.write("doc.md", "a\r\nb\rc\nd\r\ne");
    let base = repo.commit("base");
    repo.write("doc.md", "a\r\nb\rc\nd\r\nE");
    repo.commit("edit the last line");

    let changed = repo.lines(&since(&base));

    assert_eq!(repo.ranges(&changed, "doc.md"), vec![5..=5]);
}

#[test]
fn a_whole_file_range_survives_a_lone_cr() {
    let repo = repo!();
    repo.write("anchor.md", "x\n");
    repo.commit("base");
    repo.write("new.md", "a\rb\nc\n");

    let changed = repo.lines(&DiffScope::WorkingTree);

    assert!(changed.is_whole_file(&repo.path("new.md")).unwrap());
}

#[test]
fn output_over_the_cap_is_an_incomplete_error() {
    let repo = repo!();
    repo.write("doc.md", "x\n");
    repo.commit("base");
    let limits = Limits {
        max_stdout: 8,
        ..Limits::default()
    };

    let err = changed_lines_with(&repo.root, &DiffScope::WorkingTree, &limits).unwrap_err();

    assert!(matches!(err, DiffError::OutputTooLarge(8)), "{err}");
}

#[test]
fn a_run_past_the_deadline_is_an_incomplete_error() {
    let repo = repo!();
    repo.write("doc.md", "x\n");
    repo.commit("base");
    let limits = Limits {
        timeout: std::time::Duration::ZERO,
        ..Limits::default()
    };

    let err = changed_lines_with(&repo.root, &DiffScope::WorkingTree, &limits).unwrap_err();

    assert!(matches!(err, DiffError::TimedOut(_)), "{err}");
}

/// A partial clone that has no blobs. Reading a blob would make git fetch it from the
/// origin; the scope must report the object missing instead.
#[test]
fn a_missing_object_is_an_incomplete_error_and_is_never_fetched() {
    let repo = repo!();
    repo.write("doc.md", "a\n");
    let base = repo.commit("base");
    repo.write("doc.md", "a\nb\n");
    repo.commit("edit");
    repo.git(&["config", "uploadpack.allowFilter", "true"]);
    repo.git(&["config", "uploadpack.allowAnySHA1InWant", "true"]);
    let clone = repo.root.parent().unwrap().join("clone");
    let url = format!("file://{}", repo.root.display());
    repo.git(&[
        "clone",
        "-q",
        "--no-checkout",
        "--filter=blob:none",
        &url,
        clone.to_str().unwrap(),
    ]);
    let before = repo.git(&["-C", clone.to_str().unwrap(), "count-objects", "-v"]);

    let err = changed_lines(&clone, &since(&base)).unwrap_err();

    assert!(matches!(err, DiffError::MissingObject(_)), "{err}");
    let after = repo.git(&["-C", clone.to_str().unwrap(), "count-objects", "-v"]);
    assert_eq!(before, after, "the clone fetched objects");
}
