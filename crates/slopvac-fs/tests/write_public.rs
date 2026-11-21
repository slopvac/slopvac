//! Atomic writes through the public API: visibility, cleanup and path-free errors.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use std::fmt::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use slopvac_fs::{Error, Reason, WriteOptions, write_atomic};

fn root(dir: &tempfile::TempDir) -> Dir {
    Dir::open_ambient_dir(dir.path(), ambient_authority()).unwrap()
}

fn chain(err: &Error) -> String {
    let mut text = format!("{err} | {err:?}");
    let mut src = std::error::Error::source(err);
    while let Some(e) = src {
        write!(text, " | {e} | {e:?}").unwrap();
        src = e.source();
    }
    text
}

#[test]
fn a_concurrent_reader_sees_the_old_or_the_new_content_never_a_mix() {
    const LEN: usize = 256 * 1024;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("f"), vec![b'a'; LEN]).unwrap();
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let r = root(&dir);
            for i in 0..100 {
                let byte = if i % 2 == 0 { b'b' } else { b'a' };
                write_atomic(&r, Path::new("f"), &vec![byte; LEN], WriteOptions::new()).unwrap();
            }
            done.store(true, Ordering::Release);
        });
        let mut reads = 0_u32;
        while !done.load(Ordering::Acquire) || reads == 0 {
            let seen = std::fs::read(dir.path().join("f")).unwrap();
            assert_eq!(seen.len(), LEN, "a reader saw a short file");
            assert!(
                seen.iter().all(|b| *b == seen[0]),
                "a reader saw a mix of old and new content"
            );
            reads += 1;
        }
    });
}

#[test]
fn builder_options_apply() {
    let dir = tempfile::tempdir().unwrap();
    let opts = WriteOptions::new()
        .create_parents(true)
        .new_file_mode(0o644);
    write_atomic(&root(&dir), Path::new("a/b/f"), b"x", opts).unwrap();
    let mode = std::fs::metadata(dir.path().join("a/b/f"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o644);
}

#[test]
fn write_errors_keep_the_path_out_of_their_text_and_reach_it_through_path() {
    let dir = tempfile::tempdir().unwrap();
    let rel = Path::new("secret-name/f");
    let err = write_atomic(&root(&dir), rel, b"x", WriteOptions::new()).unwrap_err();
    assert!(matches!(err, Error::Io { .. }), "{err:?}");
    assert_eq!(err.path(), Some(rel));
    let text = chain(&err);
    assert!(!text.contains("secret-name"), "{text}");
    assert!(
        !text.contains(&dir.path().to_string_lossy().into_owned()),
        "{text}"
    );
}

#[test]
fn refused_targets_keep_the_path_out_of_their_text() {
    let dir = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink("elsewhere", dir.path().join("secret-link")).unwrap();
    let r = root(&dir);
    for (rel, want) in [
        ("secret-link", Reason::DestinationSymlink),
        ("../secret-up", Reason::NotRelative),
    ] {
        let err = write_atomic(&r, Path::new(rel), b"x", WriteOptions::new()).unwrap_err();
        assert!(
            matches!(&err, Error::UnsafeTarget { reason, .. } if *reason == want),
            "{rel}: {err:?}"
        );
        assert!(!chain(&err).contains("secret"), "{}", chain(&err));
    }
}
