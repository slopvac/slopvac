//! Confined and plain reads of regular files, through the public API.

use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use rstest::rstest;
use slopvac_fs::{
    InputKind, ReadOptions, read_capped_regular, read_capped_regular_in,
    read_capped_regular_to_string, read_capped_regular_to_string_in,
};

fn root(dir: &tempfile::TempDir) -> Dir {
    Dir::open_ambient_dir(dir.path(), ambient_authority()).unwrap()
}

/// Restores owner access to a directory on drop, so the temp directory that
/// holds it can be deleted even when the test panics.
struct Unlock<'a>(&'a Path);

impl Drop for Unlock<'_> {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(self.0, std::fs::Permissions::from_mode(0o700));
    }
}

/// A tree with a plain file, a nested file, links, a FIFO and a directory.
fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::write(p.join("a.txt"), b"hello").unwrap();
    std::fs::create_dir_all(p.join("d/e")).unwrap();
    std::fs::write(p.join("d/e/f.txt"), b"nested").unwrap();
    std::fs::write(p.join("big"), [b'x'; 11]).unwrap();
    std::fs::write(p.join("bad-utf8"), [0xff, 0xfe]).unwrap();
    symlink("a.txt", p.join("link")).unwrap();
    symlink("d", p.join("dlink")).unwrap();
    std::fs::hard_link(p.join("a.txt"), p.join("hard")).unwrap();
    let made = std::process::Command::new("mkfifo")
        .arg(p.join("fifo"))
        .status()
        .unwrap();
    assert!(made.success());
    drop(std::os::unix::net::UnixListener::bind(p.join("sock")).unwrap());
    dir
}

#[rstest]
#[case::plain("a.txt", b"hello")]
#[case::dot_prefix("./a.txt", b"hello")]
#[case::nested("d/e/f.txt", b"nested")]
fn confined_read_returns_the_content(#[case] rel: &str, #[case] want: &[u8]) {
    let t = tree();
    // `a.txt` has a second link, so use a copy for the hard-link-free cases.
    std::fs::remove_file(t.path().join("hard")).unwrap();
    assert_eq!(
        read_capped_regular_in(&root(&t), Path::new(rel), 100).unwrap(),
        want
    );
}

#[rstest]
#[case::missing_file("missing", InputKind::NotFound)]
#[case::missing_ancestor("nope/f", InputKind::NotFound)]
#[case::file_as_ancestor("a.txt/f", InputKind::NotFound)]
#[case::final_symlink("link", InputKind::Symlink)]
#[case::ancestor_symlink("dlink/e/f.txt", InputKind::Symlink)]
#[case::directory("d", InputKind::NotRegular)]
#[case::fifo("fifo", InputKind::NotRegular)]
#[case::socket("sock", InputKind::NotRegular)]
#[case::too_large("big", InputKind::TooLarge)]
#[case::hard_linked("hard", InputKind::MultiplyLinked)]
fn confined_read_refuses_with_a_matchable_kind(#[case] rel: &str, #[case] kind: InputKind) {
    let t = tree();
    let err = read_capped_regular_in(&root(&t), Path::new(rel), 10).unwrap_err();
    assert_eq!(err.input_kind(), Some(kind), "{err:?}");
    assert_eq!(err.path(), Some(Path::new(rel)));
}

#[rstest]
#[case::parent("../a.txt")]
#[case::inner_parent("d/../a.txt")]
#[case::absolute("/etc/passwd")]
#[case::empty("")]
fn confined_read_rejects_paths_that_leave_the_root(#[case] rel: &str) {
    let t = tree();
    let err = read_capped_regular_in(&root(&t), Path::new(rel), 100).unwrap_err();
    assert_eq!(err.input_kind(), Some(InputKind::UnsafePath), "{err:?}");
    assert_eq!(err.path(), Some(Path::new(rel)));
}

#[test]
fn a_real_failing_open_is_an_io_error_with_its_os_code_and_no_path_in_the_text() {
    if rustix::process::geteuid().is_root() {
        // Permissions do not apply to root.
        return;
    }
    let t = tree();
    let path = t.path().join("a.txt");
    std::fs::remove_file(t.path().join("hard")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let err = read_capped_regular(&path, 100, ReadOptions::new()).unwrap_err();
    assert_eq!(err.input_kind(), Some(InputKind::Io), "{err:?}");
    assert_eq!(err.path(), Some(path.as_path()));
    let source = std::error::Error::source(&err).expect("the I/O error is the source");
    let source = source.downcast_ref::<std::io::Error>().unwrap();
    assert_eq!(source.raw_os_error(), Some(13), "EACCES");
    assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
    let secret = t.path().to_string_lossy().into_owned();
    let text = format!("{err} | {err:?} | {source} | {source:?}");
    assert!(!text.contains(&secret) && !text.contains("a.txt"), "{text}");
}

#[test]
fn a_file_of_exactly_the_cap_is_accepted() {
    let t = tree();
    assert_eq!(
        read_capped_regular_in(&root(&t), Path::new("big"), 11)
            .unwrap()
            .len(),
        11
    );
}

#[test]
fn to_string_reports_invalid_utf8_and_passes_valid_text() {
    let t = tree();
    let r = root(&t);
    assert_eq!(
        read_capped_regular_to_string_in(&r, Path::new("d/e/f.txt"), 100).unwrap(),
        "nested"
    );
    let err = read_capped_regular_to_string_in(&r, Path::new("bad-utf8"), 100).unwrap_err();
    assert_eq!(err.input_kind(), Some(InputKind::NotUtf8));
    let err = read_capped_regular_to_string(&t.path().join("bad-utf8"), 100, ReadOptions::new())
        .unwrap_err();
    assert_eq!(err.input_kind(), Some(InputKind::NotUtf8));
}

#[test]
fn plain_reader_accepts_hard_links_unless_told_otherwise() {
    let t = tree();
    let hard = t.path().join("hard");
    assert_eq!(
        read_capped_regular(&hard, 100, ReadOptions::new()).unwrap(),
        b"hello"
    );
    let strict = ReadOptions::new().reject_multiply_linked(true);
    let err = read_capped_regular(&hard, 100, strict).unwrap_err();
    assert_eq!(err.input_kind(), Some(InputKind::MultiplyLinked));
}

#[rstest]
#[case::missing("missing", InputKind::NotFound)]
#[case::missing_dir("nope/f", InputKind::NotFound)]
#[case::final_symlink("link", InputKind::Symlink)]
#[case::directory("d", InputKind::NotRegular)]
#[case::fifo("fifo", InputKind::NotRegular)]
#[case::socket("sock", InputKind::NotRegular)]
#[case::too_large("big", InputKind::TooLarge)]
fn plain_reader_refuses_with_a_matchable_kind(#[case] rel: &str, #[case] kind: InputKind) {
    let t = tree();
    let path = t.path().join(rel);
    let err = read_capped_regular(&path, 10, ReadOptions::new()).unwrap_err();
    assert_eq!(err.input_kind(), Some(kind), "{err:?}");
    assert_eq!(err.path(), Some(path.as_path()));
    let secret = t.path().to_string_lossy().into_owned();
    assert!(!format!("{err} {err:?}").contains(&secret));
}

#[test]
fn plain_reader_follows_symlinked_directories_above_the_file() {
    let t = tree();
    let path = t.path().join("dlink/e/f.txt");
    assert_eq!(
        read_capped_regular(&path, 100, ReadOptions::new()).unwrap(),
        b"nested"
    );
}

#[test]
fn plain_reader_needs_a_file_name() {
    for path in ["/", "a/..", ""] {
        let err = read_capped_regular(Path::new(path), 10, ReadOptions::new()).unwrap_err();
        assert_eq!(
            err.input_kind(),
            Some(InputKind::UnsafePath),
            "{path}: {err:?}"
        );
    }
}

#[test]
fn an_unreadable_leaf_directory_is_not_regular_for_both_readers() {
    if rustix::process::geteuid().is_root() {
        // Permissions do not apply to root.
        return;
    }
    let t = tree();
    let locked = t.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let _unlock = Unlock(&locked);
    let confined = read_capped_regular_in(&root(&t), Path::new("locked"), 100).unwrap_err();
    let plain = read_capped_regular(&locked, 100, ReadOptions::new()).unwrap_err();
    assert_eq!(
        confined.input_kind(),
        Some(InputKind::NotRegular),
        "{confined:?}"
    );
    assert_eq!(plain.input_kind(), Some(InputKind::NotRegular), "{plain:?}");
}
