//! The steering-file write path through `Project`'s public interface: atomic replace, the
//! compare-and-swap refusal, refusal of special files and no leftover temporary file.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use std::path::Path;

use slopvac_agents::AgentsError;
use slopvac_agents::fsx::Project;

/// The names in `dir`, sorted, so a leftover temporary file shows up.
fn names(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

#[test]
fn a_write_replaces_the_expected_bytes_and_reports_a_change() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.md"), "old").unwrap();
    let project = Project::open(tmp.path()).unwrap();

    let changed = project
        .write_atomic(Path::new("a.md"), Some(b"old"), b"new")
        .unwrap();

    assert!(changed);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("a.md")).unwrap(),
        "new"
    );
    assert_eq!(names(tmp.path()), ["a.md"]);
}

#[test]
fn a_write_creates_a_missing_file_and_its_parent_directories() {
    let tmp = tempfile::tempdir().unwrap();
    let project = Project::open(tmp.path()).unwrap();

    let changed = project
        .write_atomic(Path::new("docs/deep/a.md"), None, b"fresh")
        .unwrap();

    assert!(changed);
    let made = tmp.path().join("docs/deep");
    assert_eq!(std::fs::read_to_string(made.join("a.md")).unwrap(), "fresh");
    assert_eq!(names(&made), ["a.md"]);
}

#[test]
fn a_write_of_unchanged_bytes_reports_no_change() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.md"), "same").unwrap();
    let project = Project::open(tmp.path()).unwrap();

    let changed = project
        .write_atomic(Path::new("a.md"), Some(b"same"), b"same")
        .unwrap();

    assert!(!changed);
    assert_eq!(names(tmp.path()), ["a.md"]);
}

#[rstest::rstest]
#[case::stale_bytes(Some(&b"stale"[..]))]
#[case::expected_absence(None)]
fn a_write_refuses_a_file_that_is_not_what_the_caller_saw(#[case] expected: Option<&[u8]>) {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.md"), "theirs").unwrap();
    let project = Project::open(tmp.path()).unwrap();

    let got = project.write_atomic(Path::new("a.md"), expected, b"ours");

    assert!(matches!(got, Err(AgentsError::Conflict(p)) if p == Path::new("a.md")));
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("a.md")).unwrap(),
        "theirs"
    );
    assert_eq!(names(tmp.path()), ["a.md"]);
}

#[test]
fn a_write_to_a_directory_is_an_unsafe_target_and_leaves_no_temporary_file() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("a.md")).unwrap();
    let project = Project::open(tmp.path()).unwrap();

    let got = project.write_atomic(Path::new("a.md"), None, b"ours");

    assert!(matches!(got, Err(AgentsError::UnsafeTarget(_))));
    assert_eq!(names(tmp.path()), ["a.md"]);
}

#[cfg(unix)]
mod special_files {
    use super::*;

    /// A write over the entry made by the closure must be an unsafe target and leave no file.
    ///
    /// The write runs on a thread with a bounded wait, so a write that blocks on a FIFO
    /// fails the test instead of hanging it.
    fn refuses(tmp: &tempfile::TempDir, make: impl FnOnce(&Path)) {
        make(&tmp.path().join("a.md"));
        let before = names(tmp.path());
        let base = tmp.path().to_owned();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let project = Project::open(&base).unwrap();
            let outcome = project.write_atomic(Path::new("a.md"), None, b"ours");
            // The receiver may have timed out and gone; the send result does not matter.
            let _ = tx.send(outcome);
        });

        let got = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the write blocked on the destination");

        assert!(
            matches!(got, Err(AgentsError::UnsafeTarget(_))),
            "got {got:?}"
        );
        assert_eq!(names(tmp.path()), before);
    }

    #[test]
    fn a_fifo_destination_is_refused_without_blocking() {
        let tmp = tempfile::tempdir().unwrap();
        refuses(&tmp, |path| {
            let made = std::process::Command::new("mkfifo")
                .arg(path)
                .status()
                .unwrap();
            assert!(made.success(), "mkfifo failed");
        });
    }

    #[test]
    fn a_socket_destination_is_refused() {
        // A socket path is limited to about 100 bytes, so a long TMPDIR must not set the base.
        let tmp = tempfile::Builder::new()
            .prefix("sv")
            .tempdir_in("/tmp")
            .unwrap();
        // Keep the listener bound until the write returns.
        let listener = std::cell::RefCell::new(None);
        refuses(&tmp, |path| {
            *listener.borrow_mut() = Some(std::os::unix::net::UnixListener::bind(path).unwrap());
        });
    }

    #[test]
    fn a_symlink_destination_is_refused_and_its_referent_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("real.txt"), "referent").unwrap();
        std::os::unix::fs::symlink("real.txt", tmp.path().join("a.md")).unwrap();
        let project = Project::open(tmp.path()).unwrap();

        // The link resolves to `real.txt`, which breaks the `.md` rule.
        let got = project.write_atomic(Path::new("a.md"), None, b"ours");

        assert!(matches!(got, Err(AgentsError::UnsafeTarget(_))));
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("real.txt")).unwrap(),
            "referent"
        );
    }

    #[test]
    fn a_link_to_a_markdown_file_is_followed_inside_the_project() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("real.md"), "old").unwrap();
        std::os::unix::fs::symlink("real.md", tmp.path().join("a.md")).unwrap();
        let project = Project::open(tmp.path()).unwrap();

        project
            .write_atomic(Path::new("a.md"), Some(b"old"), b"new")
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(tmp.path().join("real.md")).unwrap(),
            "new"
        );
        assert!(
            std::fs::symlink_metadata(tmp.path().join("a.md"))
                .unwrap()
                .is_symlink()
        );
    }
}

#[cfg(unix)]
#[test]
fn a_write_keeps_the_mode_of_the_file_it_replaces() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("a.md");
    std::fs::write(&target, "old").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
    let project = Project::open(tmp.path()).unwrap();

    project
        .write_atomic(Path::new("a.md"), Some(b"old"), b"new")
        .unwrap();

    let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o644);
}
