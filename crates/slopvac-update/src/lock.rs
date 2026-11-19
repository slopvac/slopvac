//! The update lock: one `self update` at a time per installed binary, plus the record that
//! makes an interrupted update recoverable.
//!
//! Two updates that race can roll a binary back: the slower, older one renames its archive over
//! the newer one. [`UpdateLock`] is an exclusive advisory lock on `<exe>.update.lock`, taken
//! before the download and held until after the rename.
//!
//! # The lock file
//!
//! The file sits beside the installed binary, not in the cache directory. The cache is per
//! user, but the binary can be shared, and the lock must be as wide as the file it protects.
//! The directory is attacker-reachable in the way any user-writable install directory is, so
//! the file is opened with `O_NOFOLLOW` and checked through the open handle before anything
//! is read or written: a regular file, one link, owned by the current user, mode `0600` or
//! tighter. A planted symlink, hard link or foreign file is refused with [`UpdateMessage::LockUnsafe`]
//! and never written.
//!
//! # The record
//!
//! The first line of the file records what the last lock holder did, with a SHA-256 fingerprint
//! of the executable, so the next holder learns the installed version without running an unknown
//! binary:
//!
//! - `installed <version> <sha256>`: the executable had this fingerprint when the update ended.
//! - `pending <to> <new sha256> <old sha256> <from>`: written before the rename. If the updater
//!   dies, the next holder compares the executable with both fingerprints and recovers: it
//!   matches `new` (the rename happened) or `old` (it did not). A third fingerprint means
//!   someone else changed the binary mid-update, and the next holder refuses with
//!   [`UpdateMessage::Interrupted`] rather than install over it.
//!
//! # The installed version
//!
//! The version at `exe` is established from evidence tied to the file's fingerprint, never from
//! the version of the process that runs the update: that process may be older than a build
//! another installer put on disk since it started. The floor is the highest version that any of
//! these vouches for:
//!
//! - an `installed` record, or a recovered `pending` record, whose fingerprint matches the file;
//! - the running version, only when the file's fingerprint equals the running binary's;
//! - otherwise, what the file reports when it is run (see [`crate::probe`]).
//!
//! A record whose fingerprint no longer matches is stale and counts for nothing. When no source
//! vouches for the file and it cannot report a version, the holder refuses with
//! [`UpdateMessage::VersionUnknown`].
//!
//! A record is replaced by writing the new text over the start of the file and then truncating,
//! so a crash never leaves an empty file where a record was. Text that cannot be parsed is
//! refused, never guessed at.

use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::messages::UpdateMessage;

/// How long [`UpdateLock::acquire`] waits for another update to finish.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
/// How often a waiting update retries the lock.
const POLL: Duration = Duration::from_millis(50);
/// The most of the executable that is fingerprinted. A real binary is far smaller.
const MAX_FINGERPRINT_BYTES: u64 = 512 * 1024 * 1024;
/// The most of the lock file that is read back. A record is under 300 bytes.
const MAX_RECORD_BYTES: u64 = 4096;

/// Why the lock was not taken, or the recorded state could not be trusted.
#[derive(Debug, Error)]
pub(crate) enum LockError {
    /// The lock, the record or the installed binary stopped the update. The message and its
    /// code are the catalog's.
    #[error(transparent)]
    Refused(#[from] UpdateMessage),
    /// The lock file or the executable could not be opened, read, written or locked.
    #[error("cannot use the update lock {file}: {source}")]
    Io {
        /// Lock file name.
        file: String,
        /// The failure.
        source: std::io::Error,
    },
}

impl LockError {
    /// An interrupted update. The advice is filled in by the caller that can release the lock.
    fn interrupted() -> Self {
        UpdateMessage::Interrupted {
            hint: String::new(),
        }
        .into()
    }

    fn version_unknown(reason: String) -> Self {
        UpdateMessage::VersionUnknown { reason }.into()
    }
}

/// An exclusive hold on the update lock. Dropping it releases the lock.
#[derive(Debug)]
pub(crate) struct UpdateLock {
    file: File,
    exe: PathBuf,
    name: String,
}

/// The lock file for `exe`: `<exe>.update.lock` in the same directory.
fn lock_path(exe: &Path) -> PathBuf {
    let mut name: OsString = exe.file_name().unwrap_or_default().to_owned();
    name.push(".update.lock");
    exe.with_file_name(name)
}

/// What the lock file says about the executable.
#[derive(Debug, PartialEq, Eq)]
enum Record {
    Installed {
        version: semver::Version,
        sha: String,
    },
    Pending {
        to: semver::Version,
        new_sha: String,
        old_sha: String,
        from: semver::Version,
    },
}

impl Record {
    fn render(&self) -> String {
        match self {
            Self::Installed { version, sha } => format!("installed {version} {sha}\n"),
            Self::Pending {
                to,
                new_sha,
                old_sha,
                from,
            } => format!("pending {to} {new_sha} {old_sha} {from}\n"),
        }
    }

    /// Parse the first line. `None` means the text is not a record.
    fn parse(text: &str) -> Option<Self> {
        let line = text.lines().next()?;
        let mut parts = line.split(' ');
        let kind = parts.next()?;
        let version = |s: Option<&str>| semver::Version::parse(s?).ok();
        let sha = |s: Option<&str>| {
            s.filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
                .map(str::to_owned)
        };
        let record = match kind {
            "installed" => Self::Installed {
                version: version(parts.next())?,
                sha: sha(parts.next())?,
            },
            "pending" => Self::Pending {
                to: version(parts.next())?,
                new_sha: sha(parts.next())?,
                old_sha: sha(parts.next())?,
                from: version(parts.next())?,
            },
            _ => return None,
        };
        parts.next().is_none().then_some(record)
    }
}

/// The lowercase hex SHA-256 of `reader`'s first `MAX_FINGERPRINT_BYTES`.
fn fingerprint(reader: impl Read) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    std::io::copy(&mut reader.take(MAX_FINGERPRINT_BYTES), &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn fingerprint_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The fingerprint of the file at `path`, as the lock records it.
pub(crate) fn fingerprint_file(path: &Path) -> std::io::Result<String> {
    File::open(path).and_then(fingerprint)
}

/// The version at `exe` and the fingerprint of the file that version was established for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Installed {
    /// The installed version.
    pub(crate) version: semver::Version,
    /// The fingerprint of the file the version was established for.
    sha: String,
}

/// What is known about the process that runs the update.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Running<'a> {
    /// Its version.
    pub(crate) version: &'a semver::Version,
    /// The fingerprint of its binary, taken when it started. `None` when it could not be taken,
    /// which means the running version vouches for nothing.
    pub(crate) sha: Option<&'a str>,
}

/// Open the lock file without following a symlink, creating it with mode `0600`.
fn open_nofollow(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let nofollow = i32::try_from(rustix::fs::OFlags::NOFOLLOW.bits())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        opts.mode(0o600).custom_flags(nofollow);
    }
    opts.open(path)
}

/// Check the open handle: a regular file, one link, ours, and private. Returns the problem.
#[cfg(unix)]
fn handle_problem(file: &File) -> std::io::Result<Option<&'static str>> {
    use std::os::unix::fs::MetadataExt;
    let meta = file.metadata()?;
    Ok(if !meta.is_file() {
        Some("it is not a regular file")
    } else if meta.nlink() != 1 {
        Some("it has more than one hard link")
    } else if meta.uid() != rustix::process::getuid().as_raw() {
        Some("it is owned by another user")
    } else if meta.mode() & 0o177 != 0 {
        Some("its mode is looser than 0600")
    } else {
        None
    })
}

#[cfg(not(unix))]
fn handle_problem(file: &File) -> std::io::Result<Option<&'static str>> {
    Ok((!file.metadata()?.is_file()).then_some("it is not a regular file"))
}

/// What to tell the user after [`UpdateMessage::Interrupted`], decided by trying the lock.
///
/// The caller must have released its own hold. A free lock means no update is running, so the
/// stale file is safe to remove. A held lock means another update is running, and removing the
/// file under it would break the serialization, so the advice is to wait.
pub(crate) fn recovery_hint(exe: &Path) -> String {
    let name = lock_path(exe)
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    match UpdateLock::acquire(exe, Duration::ZERO) {
        Ok(_free) => format!(
            "No other update is running, so remove {name} beside it and run the update again."
        ),
        Err(LockError::Refused(UpdateMessage::InProgress { .. })) => {
            "Another slopvac update is running; wait for it to finish, \
             then run the update again."
                .to_owned()
        }
        Err(_) => format!(
            "Make sure that no slopvac update is running before you remove {name}, \
             then run the update again."
        ),
    }
}

impl UpdateLock {
    /// Take the lock for `exe`, waiting up to `timeout` for another update to release it.
    pub(crate) fn acquire(exe: &Path, timeout: Duration) -> Result<Self, LockError> {
        Self::acquire_observed(exe, timeout, &|| {})
    }

    /// Like [`UpdateLock::acquire`], calling `on_wait` each time the lock is held by another
    /// update and this one is about to wait for it.
    pub(crate) fn acquire_observed(
        exe: &Path,
        timeout: Duration,
        on_wait: &dyn Fn(),
    ) -> Result<Self, LockError> {
        let path = lock_path(exe);
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let io = |source| LockError::Io {
            file: name.clone(),
            source,
        };
        let unsafe_file = |problem: &str| {
            LockError::from(UpdateMessage::LockUnsafe {
                file: name.clone(),
                problem: problem.to_owned(),
            })
        };
        let file = match open_nofollow(&path) {
            Ok(f) => f,
            Err(e) => {
                let is_link =
                    std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink());
                return Err(if is_link {
                    unsafe_file("it is a symbolic link")
                } else {
                    io(e)
                });
            }
        };
        if let Some(problem) = handle_problem(&file).map_err(io)? {
            return Err(unsafe_file(problem));
        }
        let deadline = Instant::now() + timeout;
        loop {
            match file.try_lock() {
                Ok(()) => {
                    return Ok(Self {
                        file,
                        exe: exe.to_owned(),
                        name,
                    });
                }
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    on_wait();
                    std::thread::sleep(POLL);
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(UpdateMessage::InProgress { file: name }.into());
                }
                Err(TryLockError::Error(e)) => return Err(io(e)),
            }
        }
    }

    fn io(&self, source: std::io::Error) -> LockError {
        LockError::Io {
            file: self.name.clone(),
            source,
        }
    }

    /// The fingerprint of the executable as it is now.
    fn exe_fingerprint(&self) -> Result<String, LockError> {
        File::open(&self.exe)
            .and_then(fingerprint)
            .map_err(|e| self.io(e))
    }

    fn read_record(&mut self) -> Result<Option<Record>, LockError> {
        let mut text = Vec::new();
        self.file
            .seek(SeekFrom::Start(0))
            .and_then(|_| {
                (&self.file)
                    .take(MAX_RECORD_BYTES)
                    .read_to_end(&mut text)
                    .map(|_| ())
            })
            .map_err(|e| self.io(e))?;
        if text.is_empty() {
            return Ok(None);
        }
        std::str::from_utf8(&text)
            .ok()
            .and_then(Record::parse)
            .map(Some)
            .ok_or_else(LockError::interrupted)
    }

    /// Write `record` over the start of the file, truncate the tail, and sync.
    fn write_record(&mut self, record: &Record) -> Result<(), LockError> {
        let text = record.render();
        let len = u64::try_from(text.len())
            .map_err(|e| self.io(std::io::Error::new(std::io::ErrorKind::InvalidInput, e)))?;
        self.file
            .seek(SeekFrom::Start(0))
            .and_then(|_| self.file.write_all(text.as_bytes()))
            .and_then(|()| self.file.set_len(len))
            .and_then(|()| self.file.sync_all())
            .map_err(|e| self.io(e))
    }

    /// The version installed at `exe` right now, established under the lock.
    ///
    /// The result is the highest version that the file's fingerprint vouches for: a matching
    /// record, a `pending` record resolved from the fingerprint, or the running version when
    /// `running` carries the same fingerprint. With none of those, the executable is run for its
    /// version, and a failure is [`UpdateMessage::VersionUnknown`]. A `pending` record that fits
    /// neither build is [`UpdateMessage::Interrupted`].
    pub(crate) fn installed_version(
        &mut self,
        running: &Running<'_>,
        probe_timeout: Duration,
    ) -> Result<Installed, LockError> {
        let record = self.read_record()?;
        let now = self.exe_fingerprint()?;
        let mut floor: Option<semver::Version> = None;
        let mut vouch = |v: semver::Version| {
            floor = Some(match floor.take() {
                Some(f) => f.max(v),
                None => v,
            });
        };
        match record {
            Some(Record::Installed { version, sha }) if sha == now => vouch(version),
            Some(Record::Pending {
                to,
                new_sha,
                old_sha,
                from,
            }) => {
                let (version, sha) = if now == new_sha {
                    (to, new_sha)
                } else if now == old_sha {
                    (from, old_sha)
                } else {
                    return Err(LockError::interrupted());
                };
                self.write_record(&Record::Installed {
                    version: version.clone(),
                    sha,
                })?;
                vouch(version);
            }
            // No record, or a stale one: it vouches for nothing.
            Some(Record::Installed { .. }) | None => {}
        }
        if running.sha == Some(now.as_str()) {
            vouch(running.version.clone());
        }
        let version = match floor {
            Some(version) => version,
            None => crate::probe::installed_version(&self.exe, probe_timeout)
                .map_err(|e| LockError::version_unknown(format!("{}: {e}", self.name_of_exe())))?,
        };
        Ok(Installed { version, sha: now })
    }

    fn name_of_exe(&self) -> String {
        self.exe
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    }

    /// Record, before the rename, that `to` is about to replace the build `installed` describes.
    /// Returns the fingerprint of `new_binary`, which [`UpdateLock::finish`] expects on disk.
    ///
    /// The record is synced to disk before this returns. A failure aborts the update before it
    /// touches the executable.
    pub(crate) fn begin(
        &mut self,
        installed: &Installed,
        to: &semver::Version,
        new_binary: &[u8],
    ) -> Result<String, LockError> {
        self.require_on_disk(
            &installed.sha,
            "the installed binary changed after its version was checked",
        )?;
        let new_sha = fingerprint_bytes(new_binary);
        self.write_record(&Record::Pending {
            to: to.clone(),
            new_sha: new_sha.clone(),
            old_sha: installed.sha.clone(),
            from: installed.version.clone(),
        })?;
        Ok(new_sha)
    }

    /// Record, after the rename, that `version` is installed.
    pub(crate) fn finish(
        &mut self,
        version: &semver::Version,
        payload_sha: &str,
    ) -> Result<(), LockError> {
        self.require_on_disk(
            payload_sha,
            "the installed binary is not the build this update wrote",
        )?;
        self.write_record(&Record::Installed {
            version: version.clone(),
            sha: payload_sha.to_owned(),
        })
    }

    /// Refuse unless the executable on disk has fingerprint `sha`. A record pairs a version with
    /// the bytes it was established for, so it is never written for any other bytes.
    fn require_on_disk(&self, sha: &str, reason: &str) -> Result<(), LockError> {
        if self.exe_fingerprint()? == sha {
            return Ok(());
        }
        Err(LockError::version_unknown(format!(
            "{}: {reason}",
            self.name_of_exe()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> semver::Version {
        semver::Version::parse(s).unwrap()
    }

    fn installed() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("slopvac");
        std::fs::write(&exe, b"binary").unwrap();
        (dir, exe)
    }

    fn lock_file(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("slopvac.update.lock")
    }

    fn acquire(exe: &Path) -> UpdateLock {
        UpdateLock::acquire(exe, Duration::ZERO).unwrap()
    }

    /// Replace `exe` with a shell script that runs `body`, standing in for another installer's
    /// build.
    #[cfg(unix)]
    fn fake_exe(exe: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(exe, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn the_lock_file_sits_beside_the_binary() {
        let (dir, exe) = installed();
        let _lock = acquire(&exe);
        assert!(lock_file(&dir).is_file());
    }

    #[test]
    fn a_second_holder_is_busy_while_the_first_holds_the_lock() {
        let (_dir, exe) = installed();
        let first = acquire(&exe);
        let err = UpdateLock::acquire(&exe, Duration::ZERO).unwrap_err();
        assert!(
            matches!(err, LockError::Refused(UpdateMessage::InProgress { ref file }) if file == "slopvac.update.lock"),
            "{err}"
        );
        drop(first);
        acquire(&exe);
    }

    #[test]
    fn a_waiter_gets_the_lock_once_the_holder_releases() {
        let (_dir, exe) = installed();
        let first = acquire(&exe);
        let waiter_exe = exe.clone();
        let waiter =
            std::thread::spawn(move || UpdateLock::acquire(&waiter_exe, Duration::from_secs(30)));
        drop(first);
        waiter.join().unwrap().expect("acquired after release");
    }

    /// The installed version as the lock establishes it, for a process that runs `version`
    /// from a binary with fingerprint `sha` (`None`: unknown).
    fn checked(
        lock: &mut UpdateLock,
        version: &str,
        sha: Option<&str>,
    ) -> Result<Installed, LockError> {
        let version = v(version);
        lock.installed_version(
            &Running {
                version: &version,
                sha,
            },
            Duration::from_secs(300),
        )
    }

    /// Like [`checked`], keeping only the version.
    fn floor(
        lock: &mut UpdateLock,
        version: &str,
        sha: Option<&str>,
    ) -> Result<semver::Version, LockError> {
        checked(lock, version, sha).map(|i| i.version)
    }

    /// `exe` as a check would describe it today, at `version`.
    fn at(exe: &Path, version: &str) -> Installed {
        Installed {
            version: v(version),
            sha: sha_of(exe),
        }
    }

    /// The fingerprint of the file at `exe` as it is now: the running binary is that file.
    fn sha_of(exe: &Path) -> String {
        fingerprint_file(exe).unwrap()
    }

    #[test]
    fn a_running_binary_that_is_the_file_on_disk_vouches_for_its_version() {
        let (_dir, exe) = installed();
        let sha = sha_of(&exe);
        assert_eq!(
            floor(&mut acquire(&exe), "3.0.0", Some(&sha)).unwrap(),
            v("3.0.0")
        );
    }

    #[test]
    fn a_finished_update_is_read_back_by_the_next_holder() {
        let (_dir, exe) = installed();
        let started_as = sha_of(&exe);
        let mut lock = acquire(&exe);
        lock.begin(&at(&exe, "3.0.0"), &v("3.2.0"), b"new").unwrap();
        std::fs::write(&exe, b"new").unwrap();
        lock.finish(&v("3.2.0"), &fingerprint_bytes(b"new"))
            .unwrap();
        drop(lock);
        // The next holder started as the old binary, so only the record can vouch for the file.
        assert_eq!(
            floor(&mut acquire(&exe), "3.0.0", Some(&started_as)).unwrap(),
            v("3.2.0")
        );
    }

    // The updater died after the rename and before the final record.
    #[test]
    fn an_update_that_died_after_the_rename_is_recovered_as_installed() {
        let (_dir, exe) = installed();
        let started_as = sha_of(&exe);
        let mut dead = acquire(&exe);
        dead.begin(&at(&exe, "3.0.0"), &v("3.2.0"), b"new").unwrap();
        std::fs::write(&exe, b"new").unwrap();
        drop(dead);
        let mut next = acquire(&exe);
        assert_eq!(
            floor(&mut next, "3.0.0", Some(&started_as)).unwrap(),
            v("3.2.0")
        );
        assert_eq!(
            floor(&mut next, "3.0.0", Some(&started_as)).unwrap(),
            v("3.2.0"),
            "the recovery was written back"
        );
    }

    // The updater died after the pending record and before the rename.
    #[test]
    fn an_update_that_died_before_the_rename_leaves_the_old_build_installed() {
        let (_dir, exe) = installed();
        let sha = sha_of(&exe);
        let mut dead = acquire(&exe);
        dead.begin(&at(&exe, "3.0.0"), &v("3.2.0"), b"new").unwrap();
        drop(dead);
        assert_eq!(
            floor(&mut acquire(&exe), "3.0.0", Some(&sha)).unwrap(),
            v("3.0.0")
        );
    }

    #[test]
    fn a_pending_update_over_an_unknown_binary_is_refused() {
        let (_dir, exe) = installed();
        let sha = sha_of(&exe);
        let mut dead = acquire(&exe);
        dead.begin(&at(&exe, "3.0.0"), &v("3.2.0"), b"new").unwrap();
        std::fs::write(&exe, b"someone else's build").unwrap();
        drop(dead);
        let err = floor(&mut acquire(&exe), "3.0.0", Some(&sha)).unwrap_err();
        assert!(
            matches!(err, LockError::Refused(UpdateMessage::Interrupted { .. })),
            "{err}"
        );
    }

    #[test]
    fn a_garbled_record_is_refused_not_guessed() {
        let (dir, exe) = installed();
        let sha = sha_of(&exe);
        drop(acquire(&exe));
        std::fs::write(lock_file(&dir), b"\xff\xfe junk").unwrap();
        let err = floor(&mut acquire(&exe), "3.0.0", Some(&sha)).unwrap_err();
        assert!(
            matches!(err, LockError::Refused(UpdateMessage::Interrupted { .. })),
            "{err}"
        );
    }

    #[test]
    fn a_stale_tail_after_the_record_is_ignored() {
        let (dir, exe) = installed();
        let started_as = sha_of(&exe);
        let mut lock = acquire(&exe);
        lock.finish(&v("3.1.0"), &started_as).unwrap();
        drop(lock);
        let mut text = std::fs::read(lock_file(&dir)).unwrap();
        text.extend_from_slice(b"installed 9.9.9 left over from a longer record\n");
        std::fs::write(lock_file(&dir), text).unwrap();
        assert_eq!(
            floor(&mut acquire(&exe), "3.0.0", Some(&started_as)).unwrap(),
            v("3.1.0")
        );
    }

    // Another installer replaced the binary after this process started: the running version
    // says nothing about the file, the file has to say it.
    #[cfg(unix)]
    #[test]
    fn a_binary_replaced_since_start_is_asked_for_its_version() {
        let (_dir, exe) = installed();
        let started_as = sha_of(&exe);
        fake_exe(&exe, "echo '{\"name\":\"slopvac\",\"version\":\"3.3.0\"}'");
        // The running process is older (3.0.0) and no record exists.
        assert_eq!(
            floor(&mut acquire(&exe), "3.0.0", Some(&started_as)).unwrap(),
            v("3.3.0")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_stale_record_does_not_hide_a_newer_independent_install() {
        let (_dir, exe) = installed();
        let started_as = sha_of(&exe);
        let mut lock = acquire(&exe);
        lock.begin(&at(&exe, "3.0.0"), &v("3.1.0"), b"new").unwrap();
        std::fs::write(&exe, b"new").unwrap();
        lock.finish(&v("3.1.0"), &fingerprint_bytes(b"new"))
            .unwrap();
        fake_exe(&exe, "echo '{\"name\":\"slopvac\",\"version\":\"3.3.0\"}'");
        assert_eq!(
            floor(&mut lock, "3.0.0", Some(&started_as)).unwrap(),
            v("3.3.0")
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_unknown_running_fingerprint_never_vouches_for_the_running_version() {
        let (_dir, exe) = installed();
        fake_exe(&exe, "echo '{\"name\":\"slopvac\",\"version\":\"3.3.0\"}'");
        assert_eq!(
            floor(&mut acquire(&exe), "3.9.0", None).unwrap(),
            v("3.3.0")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_binary_that_cannot_report_its_version_fails_closed() {
        let started_as = sha_of(&installed().1);
        for body in [
            "echo garbage",
            "echo '{\"name\":\"slopvac\",\"version\":\"3\"}'",
            "exit 1",
        ] {
            let (_dir, exe) = installed();
            fake_exe(&exe, body);
            let err = floor(&mut acquire(&exe), "3.0.0", Some(&started_as)).unwrap_err();
            assert!(
                matches!(
                    err,
                    LockError::Refused(UpdateMessage::VersionUnknown { .. })
                ),
                "{body}: {err}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_binary_that_has_not_answered_by_the_deadline_fails_closed() {
        let (_dir, exe) = installed();
        let started_as = sha_of(&exe);
        fake_exe(&exe, "exec sleep 3600");
        let mut lock = acquire(&exe);
        let running = v("3.0.0");
        let err = lock
            .installed_version(
                &Running {
                    version: &running,
                    sha: Some(&started_as),
                },
                Duration::ZERO,
            )
            .unwrap_err();
        assert!(
            matches!(
                err,
                LockError::Refused(UpdateMessage::VersionUnknown { .. })
            ),
            "{err}"
        );
    }

    #[test]
    fn recovery_advice_says_to_remove_the_lock_only_when_no_update_holds_it() {
        let (_dir, exe) = installed();
        let held = acquire(&exe);
        let busy = recovery_hint(&exe);
        assert!(busy.contains("Another slopvac update is running"), "{busy}");
        assert!(!busy.contains("remove"), "{busy}");
        drop(held);
        let free = recovery_hint(&exe);
        assert!(
            free.contains("remove slopvac.update.lock") && free.contains("No other"),
            "{free}"
        );
    }

    #[test]
    fn recovery_advice_does_not_leave_the_lock_held() {
        let (_dir, exe) = installed();
        let _ = recovery_hint(&exe);
        acquire(&exe);
    }

    // Another installer replaces the file after its version was checked and before the
    // pending record is written. The record would pair the checked version with the new file.
    #[test]
    fn begin_refuses_a_file_that_changed_since_its_version_was_checked() {
        let (_dir, exe) = installed();
        let sha = sha_of(&exe);
        let mut lock = acquire(&exe);
        let from = checked(&mut lock, "3.0.0", Some(&sha)).unwrap();
        std::fs::write(&exe, b"swapped in by another installer").unwrap();
        let err = lock.begin(&from, &v("3.2.0"), b"new").unwrap_err();
        assert!(
            matches!(
                err,
                LockError::Refused(UpdateMessage::VersionUnknown { .. })
            ),
            "{err}"
        );
        assert_eq!(lock.read_record().unwrap(), None, "nothing was recorded");
    }

    // Another installer replaces the file after our rename and before the final record. The
    // final record would claim that file is the build this update wrote.
    #[test]
    fn finish_refuses_a_file_that_is_not_the_verified_payload() {
        let (_dir, exe) = installed();
        let sha = sha_of(&exe);
        let mut lock = acquire(&exe);
        let from = checked(&mut lock, "3.0.0", Some(&sha)).unwrap();
        let payload = lock.begin(&from, &v("3.2.0"), b"new").unwrap();
        std::fs::write(&exe, b"swapped in by another installer").unwrap();
        let err = lock.finish(&v("3.2.0"), &payload).unwrap_err();
        assert!(
            matches!(
                err,
                LockError::Refused(UpdateMessage::VersionUnknown { .. })
            ),
            "{err}"
        );
        assert!(
            matches!(lock.read_record().unwrap(), Some(Record::Pending { .. })),
            "the pending record stays, so the next holder refuses instead of trusting the file"
        );
    }

    #[test]
    fn a_record_write_failure_surfaces_as_an_error() {
        let (_dir, exe) = installed();
        let mut lock = acquire(&exe);
        // Reopen the handle read-only: the record write then fails with a real I/O error.
        lock.file = File::open(lock_path(&exe)).unwrap();
        let err = lock
            .begin(&at(&exe, "3.0.0"), &v("3.2.0"), b"new")
            .unwrap_err();
        assert!(matches!(err, LockError::Io { .. }), "{err}");
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::os::unix::fs::{PermissionsExt, symlink};

        #[test]
        fn a_planted_symlink_is_refused_and_its_target_is_untouched() {
            let (dir, exe) = installed();
            let target = dir.path().join("victim");
            std::fs::write(&target, b"precious").unwrap();
            symlink(&target, lock_file(&dir)).unwrap();
            let err = UpdateLock::acquire(&exe, Duration::ZERO).unwrap_err();
            assert!(
                matches!(err, LockError::Refused(UpdateMessage::LockUnsafe { ref problem, .. }) if problem.contains("symbolic link")),
                "{err}"
            );
            assert_eq!(std::fs::read(&target).unwrap(), b"precious");
        }

        #[test]
        fn a_dangling_symlink_is_refused_and_nothing_is_created_through_it() {
            let (dir, exe) = installed();
            let target = dir.path().join("not-yet");
            symlink(&target, lock_file(&dir)).unwrap();
            let err = UpdateLock::acquire(&exe, Duration::ZERO).unwrap_err();
            assert!(
                matches!(err, LockError::Refused(UpdateMessage::LockUnsafe { .. })),
                "{err}"
            );
            assert!(!target.exists());
        }

        #[test]
        fn a_looser_mode_than_0600_is_refused() {
            for mode in [0o644, 0o660, 0o602, 0o700] {
                let (dir, exe) = installed();
                drop(acquire(&exe));
                std::fs::set_permissions(lock_file(&dir), std::fs::Permissions::from_mode(mode))
                    .unwrap();
                let err = UpdateLock::acquire(&exe, Duration::ZERO).unwrap_err();
                assert!(
                    matches!(err, LockError::Refused(UpdateMessage::LockUnsafe { ref problem, .. }) if problem.contains("mode")),
                    "{mode:o}: {err}"
                );
            }
        }

        #[test]
        fn a_hard_linked_lock_file_is_refused() {
            let (dir, exe) = installed();
            let other = dir.path().join("other-name");
            std::fs::write(&other, b"").unwrap();
            std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o600)).unwrap();
            std::fs::hard_link(&other, lock_file(&dir)).unwrap();
            let err = UpdateLock::acquire(&exe, Duration::ZERO).unwrap_err();
            assert!(
                matches!(err, LockError::Refused(UpdateMessage::LockUnsafe { ref problem, .. }) if problem.contains("hard link")),
                "{err}"
            );
        }

        #[test]
        fn a_new_lock_file_is_private_to_the_owner() {
            let (dir, exe) = installed();
            let _lock = acquire(&exe);
            let mode = std::fs::metadata(lock_file(&dir))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
