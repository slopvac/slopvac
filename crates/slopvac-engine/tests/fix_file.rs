//! Fixing files on disk: what is refused, how the write is committed, and what survives a
//! failure.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![allow(
    clippy::allow_attributes_without_reason,
    reason = "TEMPORARY: burn-down in slopvac-8l4.3.50"
)]
#![allow(missing_docs, clippy::assert_is_empty)]

mod common;

#[cfg(unix)]
use std::io::Error;
use std::path::Path;

use common::{engine, rule};
use serde_json::json;
use slopvac_engine::{Engine, FileFix, FileResult, Kind, Refusal, fix_file};
#[cfg(unix)]
use slopvac_engine::{Step, fix_file_hooked};

const DOC: &str = "Use delta here.\n";
const FIXED: &str = "Use d here.\n";

fn swaps() -> Engine {
    engine(&[rule(
        "swap",
        &json!({"kind": "substitution", "substitutions": {"delta": "d"}}),
        &json!({}),
    )])
}

/// A project root holding `doc.md`.
fn project() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("doc.md"), DOC).unwrap();
    root
}

fn run(root: &Path, rel: &str, dry_run: bool) -> FileFix {
    fix_file(
        &swaps(),
        root,
        Path::new(rel),
        Kind::Markdown,
        dry_run,
        None,
    )
    .unwrap()
}

fn done(out: FileFix) -> Box<FileResult> {
    match out {
        FileFix::Done(result) => result,
        FileFix::Refused(why) => panic!("refused: {why:?}"),
    }
}

fn refused(out: &FileFix) -> Refusal {
    match out {
        FileFix::Refused(why) => *why,
        FileFix::Done(_) => panic!("not refused"),
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn read(path: impl AsRef<Path>) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn a_dry_run_reports_the_diff_and_leaves_the_file_alone() {
    let root = project();
    let out = done(run(root.path(), "doc.md", true));
    assert!(!out.written);
    assert_eq!(out.report.text, FIXED);
    assert!(
        out.diff.contains("-Use delta here.\n+Use d here.\n"),
        "{}",
        out.diff
    );
    assert_eq!(read(root.path().join("doc.md")), DOC);
}

#[cfg(unix)]
#[test]
fn a_write_fixes_the_file_and_lints_the_fixed_text_again() {
    let root = project();
    std::fs::write(root.path().join("doc.md"), "Use delta here, delta there.\n").unwrap();
    let out = done(run(root.path(), "doc.md", false));
    assert!(out.written);
    assert_eq!(read(root.path().join("doc.md")), "Use d here, d there.\n");
    assert!(out.document.findings.is_empty());
}

#[cfg(unix)]
#[test]
fn an_absolute_path_inside_the_root_works_and_a_nested_one_too() {
    let root = project();
    std::fs::create_dir(root.path().join("sub")).unwrap();
    std::fs::write(root.path().join("sub/n.md"), DOC).unwrap();
    assert!(done(run(root.path(), "sub/n.md", false)).written);
    let absolute = root.path().join("doc.md");
    let out = fix_file(
        &swaps(),
        root.path(),
        &absolute,
        Kind::Markdown,
        false,
        None,
    )
    .unwrap();
    assert!(done(out).written);
}

#[test]
fn a_file_with_nothing_to_fix_is_not_written() {
    let root = project();
    std::fs::write(root.path().join("doc.md"), "Clean text.\n").unwrap();
    let out = done(run(root.path(), "doc.md", false));
    assert!(!out.written);
    assert_eq!(out.diff, "");
}

#[test]
fn a_file_that_is_not_utf8_is_an_error_and_stays_as_it_was() {
    let root = project();
    std::fs::write(root.path().join("doc.md"), b"delta \xff\n").unwrap();
    let err = fix_file(
        &swaps(),
        root.path(),
        Path::new("doc.md"),
        Kind::Markdown,
        false,
        None,
    )
    .unwrap_err();
    assert!(matches!(err, slopvac_engine::FixError::NotUtf8(_)), "{err}");
    assert_eq!(
        std::fs::read(root.path().join("doc.md")).unwrap(),
        b"delta \xff\n"
    );
}

#[test]
fn a_file_over_the_size_cap_is_refused_unread_and_unchanged() {
    let root = project();
    let path = root.path().join("big.md");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(slopvac_engine::MAX_FILE_BYTES + 1).unwrap();
    drop(file);
    let out = run(root.path(), "big.md", false);
    assert_eq!(refused(&out), Refusal::TooLarge);
    assert_eq!(Refusal::TooLarge.code(), "document-too-large");
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        slopvac_engine::MAX_FILE_BYTES + 1
    );
    assert_eq!(names(root.path()), ["big.md", "doc.md"]);
}

#[test]
fn a_path_outside_the_root_is_refused() {
    let root = project();
    let outside = project();
    for path in [
        outside.path().join("doc.md").to_string_lossy().into_owned(),
        "../doc.md".to_owned(),
        "sub/../../doc.md".to_owned(),
    ] {
        let out = fix_file(
            &swaps(),
            root.path(),
            Path::new(&path),
            Kind::Markdown,
            false,
            None,
        )
        .unwrap();
        assert_eq!(refused(&out), Refusal::OutsideRoot, "{path}");
        assert_eq!(refused(&out).code(), "fix-outside-root", "{path}");
    }
    assert_eq!(read(outside.path().join("doc.md")), DOC);
}

/// Run `tamper` at `step` of a write to `doc.md`, with `swaps()`.
#[cfg(unix)]
fn at(
    root: &Path,
    step: Step,
    mut tamper: impl FnMut() -> std::io::Result<()>,
) -> Result<FileFix, slopvac_engine::FixError> {
    fix_file_hooked(
        &swaps(),
        root,
        Path::new("doc.md"),
        Kind::Markdown,
        false,
        None,
        &mut |now| if now == step { tamper() } else { Ok(()) },
    )
}

#[cfg(unix)]
#[test]
fn a_file_edited_before_the_commit_is_left_as_the_editor_saved_it() {
    let root = project();
    let path = root.path().join("doc.md");
    // Same length, so only the bytes tell.
    let out = at(root.path(), Step::Verify, || {
        std::fs::write(&path, "Use gamma here.\n")
    })
    .unwrap();
    assert_eq!(refused(&out), Refusal::Changed);
    assert_eq!(refused(&out).code(), "fix-changed");
    assert_eq!(read(&path), "Use gamma here.\n");
    assert_eq!(names(root.path()), ["doc.md"], "no temporary file is left");
}

#[cfg(unix)]
#[test]
fn a_write_keeps_the_original_when_any_step_fails() {
    for step in [
        Step::Create,
        Step::Write,
        Step::Flush,
        Step::Verify,
        Step::Rename,
    ] {
        let root = project();
        let out = at(root.path(), step, || Err(Error::other("injected")));
        assert!(
            matches!(out, Err(slopvac_engine::FixError::Io { .. })),
            "{step:?}"
        );
        assert_eq!(read(root.path().join("doc.md")), DOC, "{step:?}");
        assert_eq!(
            names(root.path()),
            ["doc.md"],
            "{step:?} left a temporary file"
        );
    }
}

#[cfg(windows)]
#[test]
fn a_write_is_refused_but_a_dry_run_and_a_diff_work() {
    let root = project();
    let out = done(run(root.path(), "doc.md", true));
    assert_eq!(out.report.text, FIXED);
    assert!(out.diff.contains("+Use d here.\n"));
    assert_eq!(
        refused(&run(root.path(), "doc.md", false)),
        Refusal::UnsupportedPlatform
    );
    assert_eq!(
        refused(&run(root.path(), "doc.md", false)).code(),
        "fix-unsupported-platform"
    );
    assert_eq!(read(root.path().join("doc.md")), DOC);
    assert_eq!(names(root.path()), ["doc.md"]);
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    fn fifo(path: &Path) {
        let status = std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn a_symlink_is_refused_without_being_read() {
        let root = project();
        let outside = project();
        symlink(outside.path().join("doc.md"), root.path().join("link.md")).unwrap();
        let out = run(root.path(), "link.md", false);
        assert_eq!(refused(&out), Refusal::Symlink);
        assert_eq!(refused(&out).code(), "fix-symlink");
        assert_eq!(read(outside.path().join("doc.md")), DOC);
    }

    #[test]
    fn a_refused_file_in_a_batch_stays_unchanged_while_the_others_are_fixed() {
        let root = project();
        std::fs::write(root.path().join("b.md"), DOC).unwrap();
        let outside = project();
        symlink(outside.path().join("doc.md"), root.path().join("link.md")).unwrap();

        let outcomes: Vec<(&str, FileFix)> = ["doc.md", "link.md", "b.md"]
            .into_iter()
            .map(|name| (name, run(root.path(), name, false)))
            .collect();

        let reported: Vec<(&str, Option<&str>)> = outcomes
            .iter()
            .map(|(name, out)| match out {
                FileFix::Refused(why) => (*name, Some(why.code())),
                FileFix::Done(_) => (*name, None),
            })
            .collect();
        assert_eq!(
            reported,
            [
                ("doc.md", None),
                ("link.md", Some("fix-symlink")),
                ("b.md", None)
            ]
        );
        assert_eq!(read(root.path().join("doc.md")), FIXED);
        assert_eq!(read(root.path().join("b.md")), FIXED);
        let link = root.path().join("link.md");
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            outside.path().join("doc.md")
        );
        assert_eq!(read(&link), DOC);
        assert_eq!(read(outside.path().join("doc.md")), DOC);
    }

    #[test]
    fn a_symlinked_directory_above_the_file_is_refused() {
        let root = project();
        let outside = project();
        symlink(outside.path(), root.path().join("linked")).unwrap();
        let out = run(root.path(), "linked/doc.md", false);
        assert_eq!(refused(&out), Refusal::Symlink);
        assert_eq!(refused(&out).code(), "fix-symlink");
        assert_eq!(read(outside.path().join("doc.md")), DOC);
        assert_eq!(names(outside.path()), ["doc.md"]);
    }

    #[test]
    fn a_fifo_and_a_symlink_to_one_return_at_once_with_a_refusal() {
        let root = project();
        fifo(&root.path().join("pipe.md"));
        symlink(root.path().join("pipe.md"), root.path().join("viapipe.md")).unwrap();
        assert_eq!(
            refused(&run(root.path(), "pipe.md", false)),
            Refusal::NotRegular
        );
        assert_eq!(
            refused(&run(root.path(), "pipe.md", false)).code(),
            "fix-not-regular"
        );
        assert_eq!(
            refused(&run(root.path(), "viapipe.md", false)),
            Refusal::Symlink
        );
        assert_eq!(
            refused(&run(root.path(), "viapipe.md", false)).code(),
            "fix-symlink"
        );
    }

    #[test]
    fn a_hard_linked_file_is_refused_and_both_names_are_unchanged() {
        let root = project();
        std::fs::hard_link(root.path().join("doc.md"), root.path().join("b.md")).unwrap();
        let out = run(root.path(), "doc.md", false);
        assert_eq!(refused(&out), Refusal::Hardlinked);
        assert_eq!(refused(&out).code(), "fix-hardlinked");
        assert_eq!(read(root.path().join("b.md")), DOC);
    }

    #[test]
    fn a_directory_others_can_write_to_is_refused_for_a_write_but_not_a_dry_run() {
        let root = project();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(!done(run(root.path(), "doc.md", true)).written);
        assert_eq!(
            refused(&run(root.path(), "doc.md", false)),
            Refusal::UnsafeDirectory
        );
        assert_eq!(
            refused(&run(root.path(), "doc.md", false)).code(),
            "fix-unsafe-directory"
        );
        assert_eq!(read(root.path().join("doc.md")), DOC);
        assert_eq!(names(root.path()), ["doc.md"]);
    }

    /// A group other than the primary one that the current user belongs to, if any.
    fn other_group(primary: u32) -> Option<u32> {
        let out = std::process::Command::new("id").arg("-G").output().ok()?;
        String::from_utf8(out.stdout)
            .ok()?
            .split_whitespace()
            .filter_map(|g| g.parse::<u32>().ok())
            .find(|g| *g != primary)
    }

    #[test]
    fn a_write_keeps_the_group_of_the_file() {
        use std::os::unix::fs::{MetadataExt, chown};
        let root = project();
        let path = root.path().join("doc.md");
        let Some(group) = other_group(std::fs::metadata(&path).unwrap().gid()) else {
            return;
        };
        if chown(&path, None, Some(group)).is_err() {
            return;
        }
        assert!(done(run(root.path(), "doc.md", false)).written);
        assert_eq!(std::fs::metadata(&path).unwrap().gid(), group);
    }

    #[test]
    fn a_failed_group_assignment_refuses_and_leaves_no_trace() {
        let root = project();
        let out = at(root.path(), Step::Group, || Err(Error::other("injected"))).unwrap();
        assert_eq!(refused(&out), Refusal::GroupMismatch);
        assert_eq!(refused(&out).code(), "fix-group-mismatch");
        assert_eq!(read(root.path().join("doc.md")), DOC);
        assert_eq!(
            names(root.path()),
            ["doc.md"],
            "the temporary file is removed"
        );
    }

    /// Run `setfacl` on `path`; false when the tool or the file system cannot.
    #[cfg(target_os = "linux")]
    fn setfacl(args: &[&str], path: &Path) -> bool {
        std::process::Command::new("setfacl")
            .args(args)
            .arg(path)
            .status()
            .is_ok_and(|s| s.success())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_default_acl_on_the_directory_or_an_access_acl_on_the_file_refuses_the_write() {
        for (on_directory, args) in [
            (true, ["-d", "-m", "u:nobody:r"]),
            (false, ["-m", "u:nobody:r", ""]),
        ] {
            let root = project();
            let target = if on_directory {
                root.path().to_path_buf()
            } else {
                root.path().join("doc.md")
            };
            let args: Vec<&str> = args.into_iter().filter(|a| !a.is_empty()).collect();
            if !setfacl(&args, &target) {
                return; // no setfacl, or no ACL support here
            }
            assert!(
                !done(run(root.path(), "doc.md", true)).written,
                "a dry run still works"
            );
            assert_eq!(
                refused(&run(root.path(), "doc.md", false)),
                Refusal::AclPresent
            );
            assert_eq!(
                refused(&run(root.path(), "doc.md", false)).code(),
                "fix-acl-present"
            );
            assert_eq!(read(root.path().join("doc.md")), DOC);
            assert_eq!(names(root.path()), ["doc.md"]);
        }
    }

    /// The ACL check reads attributes through descriptors, and Linux rejects some kinds of
    /// descriptor. A plain file in a plain directory must pass it, not fail with an error.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_acl_check_passes_a_file_and_directory_without_acls() {
        let root = project();
        let out = done(run(root.path(), "doc.md", false));
        assert!(out.written);
        assert_eq!(read(root.path().join("doc.md")), FIXED);
    }

    #[test]
    fn a_write_keeps_the_mode() {
        let root = project();
        let path = root.path().join("doc.md");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(done(run(root.path(), "doc.md", false)).written);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o640
        );
        assert_eq!(read(&path), FIXED);
        assert_eq!(names(root.path()), ["doc.md"]);
    }

    #[test]
    fn a_path_swapped_for_a_symlink_before_the_commit_is_not_written_through() {
        let root = project();
        let outside = project();
        let secret = outside.path().join("doc.md");
        let path = root.path().join("doc.md");
        let out = at(root.path(), Step::Verify, || {
            std::fs::remove_file(&path)?;
            symlink(&secret, &path)
        })
        .unwrap();
        assert_eq!(refused(&out), Refusal::Changed);
        assert_eq!(refused(&out).code(), "fix-changed");
        assert_eq!(read(&secret), DOC);
        assert_eq!(names(root.path()), ["doc.md"]);
    }

    #[test]
    fn a_path_swapped_for_another_file_before_the_commit_is_left_alone() {
        let root = project();
        let path = root.path().join("doc.md");
        let other = root.path().join("other.md");
        let out = at(root.path(), Step::Verify, || {
            std::fs::write(&other, "Use delta here.\n")?;
            std::fs::rename(&other, &path)
        })
        .unwrap();
        assert_eq!(refused(&out), Refusal::Changed);
        assert_eq!(refused(&out).code(), "fix-changed");
        assert_eq!(read(&path), DOC);
        assert_eq!(names(root.path()), ["doc.md"]);
    }

    #[test]
    fn a_hard_link_added_before_the_commit_stops_the_write() {
        let root = project();
        let path = root.path().join("doc.md");
        let out = at(root.path(), Step::Verify, || {
            std::fs::hard_link(&path, path.with_file_name("alias.md"))
        })
        .unwrap();
        assert_eq!(refused(&out), Refusal::Changed);
        assert_eq!(refused(&out).code(), "fix-changed");
        assert_eq!(read(root.path().join("alias.md")), DOC);
        assert_eq!(names(root.path()), ["alias.md", "doc.md"]);
    }

    #[test]
    fn a_temporary_name_taken_over_before_the_rename_is_never_moved_over_the_file() {
        let root = project();
        let dir = root.path().to_path_buf();
        let outside = project();
        let secret = outside.path().join("doc.md");
        let out = at(root.path(), Step::Rename, || {
            let temp = std::fs::read_dir(&dir)?
                .map(|e| e.unwrap().path())
                .find(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .contains(".slopvac-")
                })
                .unwrap();
            std::fs::remove_file(&temp)?;
            symlink(&secret, &temp)
        })
        .unwrap();
        assert_eq!(refused(&out), Refusal::Changed);
        assert_eq!(refused(&out).code(), "fix-changed");
        assert_eq!(read(dir.join("doc.md")), DOC);
        assert_eq!(read(&secret), DOC);
    }
}

#[rstest::rstest]
#[case::outside_root(Refusal::OutsideRoot, "fix-outside-root")]
#[case::symlink(Refusal::Symlink, "fix-symlink")]
#[case::not_regular(Refusal::NotRegular, "fix-not-regular")]
#[case::hardlinked(Refusal::Hardlinked, "fix-hardlinked")]
#[case::not_owner(Refusal::NotOwner, "fix-not-owner")]
#[case::too_large(Refusal::TooLarge, "document-too-large")]
#[case::unsafe_directory(Refusal::UnsafeDirectory, "fix-unsafe-directory")]
#[case::group_mismatch(Refusal::GroupMismatch, "fix-group-mismatch")]
#[case::acl_present(Refusal::AclPresent, "fix-acl-present")]
#[case::unsupported_platform(Refusal::UnsupportedPlatform, "fix-unsupported-platform")]
#[case::changed(Refusal::Changed, "fix-changed")]
fn each_refusal_has_its_stable_code(#[case] refusal: Refusal, #[case] code: &str) {
    assert_eq!(refusal.code(), code);
}
