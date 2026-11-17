//! The update lock: one `self update` at a time per installed binary.
//!
//! Two updates that race can roll a binary back: the slower, older one renames its archive over
//! the newer one. [`UpdateLock`] is an exclusive advisory lock on `<exe>.update.lock`, taken
//! before the download and held until after the rename.
//!
//! The lock file sits beside the installed binary, not in the cache directory. The cache is per
//! user, but the binary can be shared, and the lock must be as wide as the file it protects. The
//! update already needs write access to that directory for its same-directory temp file.
//!
//! The lock file also records the version the last lock holder installed, with a stamp of the
//! binary it installed. The next holder reads the record under the lock, so it learns the
//! installed version without running an unknown binary. A record whose stamp no longer matches
//! the binary (someone replaced it by another route) is ignored.

use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use thiserror::Error;

/// How long [`UpdateLock::acquire`] waits for another update to finish.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
/// How often a waiting update retries the lock.
const POLL: Duration = Duration::from_millis(50);

/// Why the lock was not taken.
#[derive(Debug, Error)]
pub(crate) enum LockError {
    /// Another update held the lock for the whole wait.
    #[error("another slopvac update is running (lock file {0}); try again when it finishes")]
    Busy(String),
    /// The lock file could not be opened or locked.
    #[error("cannot take the update lock {file}: {source}")]
    Io {
        /// Lock file name.
        file: String,
        /// The failure.
        source: std::io::Error,
    },
}

/// An exclusive hold on the update lock. Dropping it releases the lock.
#[derive(Debug)]
pub(crate) struct UpdateLock {
    file: File,
    exe: PathBuf,
}

/// The lock file for `exe`: `<exe>.update.lock` in the same directory.
fn lock_path(exe: &Path) -> PathBuf {
    let mut name: OsString = exe.file_name().unwrap_or_default().to_owned();
    name.push(".update.lock");
    exe.with_file_name(name)
}

/// Identifies one installed file by size and modification time.
fn stamp(exe: &Path) -> Option<String> {
    let meta = std::fs::metadata(exe).ok()?;
    let nanos = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(format!("{} {nanos}", meta.len()))
}

impl UpdateLock {
    /// Take the lock for `exe`, waiting up to `timeout` for another update to release it.
    pub(crate) fn acquire(exe: &Path, timeout: Duration) -> Result<Self, LockError> {
        let path = lock_path(exe);
        let file_name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let io = |source| LockError::Io {
            file: file_name.clone(),
            source,
        };
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(io)?;
        let deadline = Instant::now() + timeout;
        loop {
            match file.try_lock() {
                Ok(()) => {
                    return Ok(Self {
                        file,
                        exe: exe.to_owned(),
                    });
                }
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(POLL);
                }
                Err(TryLockError::WouldBlock) => return Err(LockError::Busy(file_name)),
                Err(TryLockError::Error(e)) => return Err(io(e)),
            }
        }
    }

    /// The version the last lock holder installed, if the binary is still the file it installed.
    pub(crate) fn recorded_version(&mut self) -> Option<semver::Version> {
        let mut text = String::new();
        self.file.seek(SeekFrom::Start(0)).ok()?;
        self.file.read_to_string(&mut text).ok()?;
        let (version, recorded_stamp) = text.split_once('\n')?;
        (stamp(&self.exe)? == recorded_stamp)
            .then(|| semver::Version::parse(version).ok())
            .flatten()
    }

    /// Record that `version` is now installed. Call after the rename, while the lock is held.
    ///
    /// The record is a hint that a stamp guards, so a failure to write it never fails the
    /// update: the next holder then falls back to the version of the process that runs it.
    pub(crate) fn record(&mut self, version: &str) {
        let Some(stamp) = stamp(&self.exe) else {
            return;
        };
        let written = self
            .file
            .set_len(0)
            .and_then(|()| self.file.seek(SeekFrom::Start(0)).map(|_| ()))
            .and_then(|()| {
                self.file
                    .write_all(format!("{version}\n{stamp}").as_bytes())
            });
        drop(written);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("slopvac");
        std::fs::write(&exe, b"binary").unwrap();
        (dir, exe)
    }

    #[test]
    fn the_lock_file_sits_beside_the_binary() {
        let (dir, exe) = installed();
        let _lock = UpdateLock::acquire(&exe, Duration::ZERO).unwrap();
        assert!(dir.path().join("slopvac.update.lock").is_file());
    }

    #[test]
    fn a_second_holder_times_out_while_the_first_holds_the_lock() {
        let (_dir, exe) = installed();
        let first = UpdateLock::acquire(&exe, Duration::ZERO).unwrap();
        let err = UpdateLock::acquire(&exe, Duration::from_millis(120)).unwrap_err();
        assert!(
            matches!(err, LockError::Busy(ref f) if f == "slopvac.update.lock"),
            "{err}"
        );
        drop(first);
        UpdateLock::acquire(&exe, Duration::ZERO).expect("free once released");
    }

    #[test]
    fn a_waiter_gets_the_lock_when_the_holder_releases_in_time() {
        let (_dir, exe) = installed();
        let first = UpdateLock::acquire(&exe, Duration::ZERO).unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(first);
        });
        UpdateLock::acquire(&exe, Duration::from_secs(10)).expect("acquired after release");
        releaser.join().unwrap();
    }

    #[test]
    fn a_recorded_version_is_read_back_by_the_next_holder() {
        let (_dir, exe) = installed();
        let mut lock = UpdateLock::acquire(&exe, Duration::ZERO).unwrap();
        assert_eq!(lock.recorded_version(), None);
        lock.record("3.2.0");
        drop(lock);
        let mut next = UpdateLock::acquire(&exe, Duration::ZERO).unwrap();
        assert_eq!(next.recorded_version(), Some(semver::Version::new(3, 2, 0)));
    }

    // A crashed updater leaves its lock file behind, but the operating system drops its lock.
    #[test]
    fn a_stale_lock_file_from_a_dead_updater_does_not_block() {
        let (dir, exe) = installed();
        std::fs::write(dir.path().join("slopvac.update.lock"), b"3.9.0\n0 0").unwrap();
        let mut lock = UpdateLock::acquire(&exe, Duration::ZERO).expect("stale file is not held");
        assert_eq!(lock.recorded_version(), None, "its stamp no longer matches");
    }

    #[test]
    fn a_garbled_lock_file_reads_as_no_record() {
        let (dir, exe) = installed();
        std::fs::write(dir.path().join("slopvac.update.lock"), b"\xff\xfe junk").unwrap();
        let mut lock = UpdateLock::acquire(&exe, Duration::ZERO).unwrap();
        assert_eq!(lock.recorded_version(), None);
    }

    #[test]
    fn a_record_for_a_replaced_binary_is_ignored() {
        let (_dir, exe) = installed();
        let mut lock = UpdateLock::acquire(&exe, Duration::ZERO).unwrap();
        lock.record("3.2.0");
        std::fs::write(&exe, b"another installer wrote a longer binary").unwrap();
        assert_eq!(lock.recorded_version(), None);
    }
}
