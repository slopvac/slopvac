//! Applying a plan and undoing it, safe against a hostile checkout.
//!
//! The author of a pull request controls every file in the tree, the backup directory
//! included. This module defends against a hostile checkout that stays the same while
//! migrate runs:
//!
//! - No symbolic link is followed. Every path is walked from one handle on the project
//!   root, one component at a time. A symbolic link, a non-regular file and a file with
//!   more than one hard link are refused.
//! - Each run writes its backup to a new directory below [`BACKUP_DIR`], named for its UTC
//!   start time. It creates `.slopvac` and `migrate-backup` owner-only when they are missing,
//!   and refuses either one when it is a link or not a directory.
//! - The trusted restore record lives in the per-user cache directory, outside the project.
//!   It is bound to the checkout's identity, the root directory's device and inode, and to
//!   one migration instance, a random id that the backup manifest repeats. It names the
//!   run's backup directory by name and identity, and the directories above it by identity.
//!   Restore uses only that directory, never the newest one on disk, and refuses it when it
//!   does not match the record exactly.
//! - A target counts as the migration's own only when both its identity and its digest
//!   match the record: it is the file the migration staged, by device and inode, with the
//!   bytes the migration wrote. Same bytes alone prove nothing. `--force` also puts back a
//!   target that is still the file the migration installed, by device and inode, but whose
//!   bytes changed. It never replaces any other object.
//! - Otherwise it fails closed: it stops with an error, or it keeps the target as it is.
//!
//! Each change stages its bytes in an owner-only holding directory with a random name, and
//! moves the target there to check it through the held handle.
//!
//! Accepted risk: a process that runs as the same user and changes the checkout during the
//! run can defeat these checks. It can change the tree between a check and the next step.
//! For example, it can swap a file in a holding directory between the identity check and
//! the unlink by name that follows it, so the swapped file is removed. It can also move a
//! parent directory out of the project during a change, so the write lands at the new
//! place.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::hash::{BuildHasher, Hasher};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
use cap_std::fs::{
    Dir, DirBuilder, DirBuilderExt, Metadata, MetadataExt, OpenOptions, OpenOptionsExt,
    Permissions, PermissionsExt,
};
use rustix::fs::RenameFlags;
use sha2::{Digest, Sha256};
use slopvac_fs::{WriteOptions, write_atomic};

use crate::Error;
use crate::messages::{MigratePhrase, MigrateText, path_reason};
use crate::plan::{BACKUP_DIR, BACKUPS_NAME, ChangeKind, PROJECT_DIR, Plan};

/// The list of backup copies, for people. Restore checks its digest and never reads it.
const MANIFEST: &str = "manifest.txt";
const MANIFEST_HEADER: &str = "slopvac-migrate-backup";
const RECORD_HEADER: &str = "slopvac-migrate-record 4";
/// The directory below the per-user cache that holds one record per project.
const RECORD_DIR: &str = "migrate-backups";
/// Permission bits for a file that did not exist before.
const NEW_FILE_MODE: u32 = 0o644;
/// Permission bits for the backup directories, and for the files in them and the record.
const BACKUP_DIR_MODE: u32 = 0o700;
const BACKUP_FILE_MODE: u32 = 0o600;
/// How many random names to try before giving up.
const NAME_ATTEMPTS: usize = 16;

/// A moment in [`apply_hooked`] or [`restore_hooked`] where a test can act. Each carries a
/// path below the project.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Point<'a> {
    /// A new file exists, empty, and its bytes are about to be written: a staged target or a
    /// backup file.
    Write(&'a Path),
    /// The new bytes of this target are staged, and it is about to be taken and checked.
    Check(&'a Path),
    /// The target was checked, and the staged file is about to take its name.
    Install(&'a Path),
    /// The staged file took the name, and the checked original is about to be removed.
    Discard(&'a Path),
    /// This backup file was moved into a holding directory and is about to be removed.
    Cleanup(&'a Path),
}

/// Runs at each [`Point`]. An error from it fails the step. Tests swap objects here.
type Hook<'a> = &'a mut dyn FnMut(Point<'_>) -> std::io::Result<()>;

type Sha = [u8; 32];

fn sha(bytes: &[u8]) -> Sha {
    Sha256::digest(bytes).into()
}

fn hex(digest: &Sha) -> String {
    let (high, low) = digest.split_at(16);
    let half = |b: &[u8]| u128::from_be_bytes(b.try_into().unwrap_or_default());
    format!("{:032x}{:032x}", half(high), half(low))
}

fn unhex(text: &str) -> Option<Sha> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// A path as error messages name it. It stays raw: the CLI escapes the whole message once.
fn shown(path: &Path) -> String {
    path.display().to_string()
}

fn refused(path: &Path, why: MigratePhrase) -> Error {
    Error::Unsafe(path_reason(&shown(path), why.as_static()))
}

fn changed(path: &Path, why: MigratePhrase) -> Error {
    changed_with(path, why.as_static())
}

/// [`changed`] for a reason that carries a parameter.
fn changed_with(path: &Path, why: &str) -> Error {
    Error::Changed(path_reason(&shown(path), why))
}

fn unverified() -> Error {
    Error::Unsafe(
        MigrateText::BackupUnverified {
            directory: BACKUP_DIR.to_owned(),
        }
        .to_string(),
    )
}

fn exists(backup: &Path) -> Error {
    Error::Unsafe(
        MigrateText::BackupExists {
            directory: shown(backup),
        }
        .to_string(),
    )
}

/// A record in the per-user cache, at `record` below it, that exists but does not parse.
fn unreadable(record: &Path) -> Error {
    Error::Unsafe(
        MigrateText::RecordUnreadable {
            record: shown(record),
        }
        .to_string(),
    )
}

/// The name of a backup directory made at `now`, to the second. A clock before the epoch
/// is refused, because the name would not tell the time.
fn name_at(now: SystemTime) -> Result<String, Error> {
    let since = now.duration_since(UNIX_EPOCH).map_err(|_| {
        Error::io(
            Path::new(BACKUP_DIR),
            std::io::Error::other(MigratePhrase::IoClockBeforeEpoch.as_static()),
        )
    })?;
    Ok(timestamp(since.as_secs()))
}

/// The UTC time `secs` seconds after the epoch as `YYYYMMDDTHHMMSSZ`, which sorts by time.
fn timestamp(secs: u64) -> String {
    let (days, time) = (secs / 86_400, secs % 86_400);
    // The civil date of a day count from 1970-01-01, after Howard Hinnant's
    // `civil_from_days`, with eras of 400 years that start on 1 March.
    let shifted = days + 719_468;
    let (era, day_of_era) = (shifted / 146_097, shifted % 146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = era * 400 + year_of_era + u64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        time / 3600,
        time / 60 % 60,
        time % 60
    )
}

/// Whether `name` can name a backup directory: one component of ASCII letters, digits and
/// hyphens, as [`timestamp`] and its suffixes make.
fn is_backup_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A file or directory by device and inode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Id {
    dev: u64,
    ino: u64,
}

fn id_of(meta: &Metadata) -> Id {
    Id {
        dev: meta.dev(),
        ino: meta.ino(),
    }
}

/// A regular, singly linked file as it was read.
struct Seen {
    id: Id,
    mode: u32,
    digest: Sha,
    bytes: Vec<u8>,
}

fn random() -> u64 {
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}

/// 16 random bytes from the kernel, as hex.
fn nonce() -> Result<String, Error> {
    const SOURCE: &str = "/dev/urandom";
    let mut bytes = [0u8; 16];
    std::fs::File::open(SOURCE)
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| Error::io(Path::new(SOURCE), e))?;
    Ok(format!("{:032x}", u128::from_be_bytes(bytes)))
}

/// Open without blocking, so a FIFO swapped in after the check cannot hang the run.
fn nonblocking(options: &mut OpenOptions) -> std::io::Result<()> {
    let flag = i32::try_from(rustix::fs::OFlags::NONBLOCK.bits())
        .map_err(|_| std::io::Error::other(MigratePhrase::IoNonblock.as_static()))?;
    options.custom_flags(flag);
    Ok(())
}

/// Rename `from` in `from_dir` to `to` in `to_dir`, failing with `AlreadyExists` instead of
/// replacing `to`.
fn rename_noreplace(from_dir: &Dir, from: &OsStr, to_dir: &Dir, to: &OsStr) -> std::io::Result<()> {
    rustix::fs::renameat_with(from_dir, from, to_dir, to, RenameFlags::NOREPLACE)
        .map_err(Into::into)
}

fn sync_dir(dir: &Dir) -> std::io::Result<()> {
    dir.open(".")?.sync_all()
}

/// The components of a plain relative path. Absolute paths, `..`, newlines, names that are
/// not UTF-8 and an empty path are refused, because the record must name each file exactly
/// once.
fn plain(path: &Path) -> Result<Vec<&OsStr>, Error> {
    let mut names = Vec::new();
    for comp in path.components() {
        let Component::Normal(part) = comp else {
            return Err(refused(path, MigratePhrase::WhyPathLeaves));
        };
        if part.to_str().is_none() {
            return Err(refused(path, MigratePhrase::WhyPathNotUtf8));
        }
        if part.as_bytes().contains(&b'\n') {
            return Err(refused(path, MigratePhrase::WhyPathNewline));
        }
        names.push(part);
    }
    if names.is_empty() {
        return Err(Error::Unsafe(
            MigratePhrase::WhyEmptyPath.as_static().to_owned(),
        ));
    }
    Ok(names)
}

/// Open the directory that holds `rel` below `dir`, one component at a time without
/// following links. `None` when a directory on the way does not exist.
fn open_parent(dir: &Dir, rel: &Path) -> Result<Option<(Dir, OsString)>, Error> {
    let mut names = plain(rel)?;
    let Some(name) = names.pop() else {
        return Err(Error::Unsafe(
            MigratePhrase::WhyEmptyPath.as_static().to_owned(),
        ));
    };
    let mut parent = dir.try_clone().map_err(|e| Error::io(rel, e))?;
    for part in names {
        parent = match parent.open_dir_nofollow(part) {
            Ok(next) => next,
            Err(e) => {
                return match parent.symlink_metadata(part) {
                    Err(m) if m.kind() == ErrorKind::NotFound => Ok(None),
                    Ok(m) if m.is_symlink() => {
                        Err(refused(rel, MigratePhrase::WhyComponentSymlink))
                    }
                    Ok(m) if !m.is_dir() => Err(refused(rel, MigratePhrase::WhyComponentNotDir)),
                    _ => Err(Error::io(rel, e)),
                };
            }
        };
    }
    Ok(Some((parent, name.to_owned())))
}

/// Read `name` in `parent` without following a link. A symlink, a non-regular file and a
/// file with more than one link are refused; a missing file is `None`.
// TODO(8l4.12.37): replace with the compare-and-swap primitives in slopvac-fs when they land.
fn read_entry(parent: &Dir, name: &OsStr, rel: &Path) -> Result<Option<Seen>, Error> {
    let io = |e| Error::io(rel, e);
    let entry = match parent.symlink_metadata(name) {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io(e)),
    };
    if entry.is_symlink() {
        return Err(refused(rel, MigratePhrase::WhySymlink));
    }
    if !entry.is_file() {
        return Err(refused(rel, MigratePhrase::WhyNotRegular));
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    nonblocking(&mut options).map_err(io)?;
    let file = parent.open_with(name, &options).map_err(io)?;
    let held = file.metadata().map_err(io)?;
    if !held.is_file() || id_of(&held) != id_of(&entry) {
        return Err(changed(rel, MigratePhrase::WhyChangedWhileRead));
    }
    if held.nlink() > 1 {
        return Err(refused(rel, MigratePhrase::WhyHardLinks));
    }
    let bytes =
        slopvac_fs::read_capped(file, slopvac_fs::MAX_DOCUMENT_BYTES).map_err(|e| match e {
            slopvac_fs::Error::CapExceeded { .. } => refused(rel, MigratePhrase::WhyTooLarge),
            slopvac_fs::Error::Read { source, .. } => io(source),
            other => io(std::io::Error::other(other.to_string())),
        })?;
    Ok(Some(Seen {
        id: id_of(&held),
        mode: held.mode() & 0o777,
        digest: sha(&bytes),
        bytes,
    }))
}

/// Read `rel` below `dir`, as [`read_entry`] does.
fn look(dir: &Dir, rel: &Path) -> Result<Option<Seen>, Error> {
    match open_parent(dir, rel)? {
        Some((parent, name)) => read_entry(&parent, &name, rel),
        None => Ok(None),
    }
}
/// What a target must be when a change acts on it.
#[derive(Debug, Clone, Copy)]
enum Want {
    /// No entry under the name.
    Absent,
    /// The file the preflight saw, with these bytes; any bytes for `None` (`--force`).
    File { id: Id, digest: Option<Sha> },
}

impl Want {
    fn of(seen: Option<&Seen>, check_bytes: bool) -> Self {
        seen.map_or(Self::Absent, |s| Self::File {
            id: s.id,
            digest: check_bytes.then_some(s.digest),
        })
    }
}

/// The prefix of a holding directory's name.
const HOLD_PREFIX: &str = ".slopvac-hold-";
/// The staged new file inside a holding directory.
const NEW: &str = "new";
/// The checked original inside a holding directory.
const OLD: &str = "old";

/// Create `name` in `parent` as an owner-only directory and open it without following a
/// link. Fails when what opens is not an empty, owner-only directory.
fn private_dir(parent: &Dir, name: &OsStr) -> std::io::Result<(Dir, Id)> {
    let mut builder = DirBuilder::new();
    builder.mode(BACKUP_DIR_MODE);
    parent.create_dir_with(name, &builder)?;
    let dir = match parent.open_dir_nofollow(name) {
        Ok(dir) => dir,
        Err(e) => {
            // Removes only an empty directory, so nothing with content is lost.
            let _ = parent.remove_dir(name);
            return Err(e);
        }
    };
    let meta = dir.dir_metadata()?;
    if meta.mode() & 0o077 != 0 || dir.entries()?.next().is_some() {
        return Err(std::io::Error::other(
            MigratePhrase::IoCreatedReplaced.as_static(),
        ));
    }
    Ok((dir, id_of(&meta)))
}

/// Whether `name` in `parent` is still the directory `id`.
fn same_dir(parent: &Dir, name: &str, id: Id) -> bool {
    parent
        .symlink_metadata(name)
        .is_ok_and(|m| m.is_dir() && id_of(&m) == id)
}

/// A directory on the way to a backup, and whether this migration created it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Level {
    id: Id,
    made: bool,
}

/// Open the directory `name` in `parent` without following a link. `None` when nothing has
/// the name. A link and anything other than a directory are refused.
fn open_level(parent: &Dir, name: &str, shown: &Path) -> Result<Option<Dir>, Error> {
    match parent.open_dir_nofollow(name) {
        Ok(dir) => Ok(Some(dir)),
        Err(e) => match parent.symlink_metadata(name) {
            Err(m) if m.kind() == ErrorKind::NotFound => Ok(None),
            Ok(m) if m.is_symlink() => Err(refused(shown, MigratePhrase::WhySymlink)),
            Ok(m) if !m.is_dir() => Err(refused(shown, MigratePhrase::WhyNotDirectory)),
            _ => Err(Error::io(shown, e)),
        },
    }
}

/// Open the directory `name` in `parent` as [`open_level`] does, or create it owner-only
/// when it is missing.
fn ensure_level(parent: &Dir, name: &str, shown: &Path) -> Result<(Dir, Level), Error> {
    let io = |e| Error::io(shown, e);
    // A second pass covers a directory that appeared between the two calls.
    for _ in 0..2 {
        if let Some(dir) = open_level(parent, name, shown)? {
            let id = id_of(&dir.dir_metadata().map_err(io)?);
            return Ok((dir, Level { id, made: false }));
        }
        match private_dir(parent, OsStr::new(name)) {
            Ok((dir, id)) => return Ok((dir, Level { id, made: true })),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io(e)),
        }
    }
    Err(changed(shown, MigratePhrase::WhyChangedWhileCreated))
}

/// Remove the directory `name` from `parent` if this migration created it, it is still that
/// directory and it is empty. Best effort: a directory that stays is no error, because what
/// led here is complete.
fn remove_made(parent: &Dir, name: &str, level: Level) {
    if level.made && same_dir(parent, name, level.id) {
        // Removes only an empty directory.
        let _ = parent.remove_dir(name);
    }
}

/// Create `name` in `dir` without following a link and note it in `owned` at once. Then
/// write `bytes` and set `mode`. A failure after the create leaves the file in `owned`, so
/// the caller's cleanup removes it. `shown` names the file to the hook.
fn write_new(
    dir: &Dir,
    name: &OsStr,
    bytes: &[u8],
    mode: u32,
    owned: &mut Vec<(OsString, Id)>,
    shown: &Path,
    hook: Hook<'_>,
) -> std::io::Result<Id> {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No)
        .mode(BACKUP_FILE_MODE);
    let mut file = dir.open_with(name, &options)?;
    let id = id_of(&file.metadata()?);
    owned.push((name.to_owned(), id));
    hook(Point::Write(shown))?;
    file.write_all(bytes)?;
    file.set_permissions(Permissions::from_mode(mode))?;
    file.sync_all()?;
    Ok(id)
}

/// Remove `name` from `dir` if it is still the regular file `id`. Returns whether it was
/// removed. The check and the removal are two calls; see the module docs.
fn remove_if(dir: &Dir, name: &OsStr, id: Id) -> std::io::Result<bool> {
    match dir.symlink_metadata(name) {
        Ok(m) if m.is_file() && id_of(&m) == id => dir.remove_file(name).map(|()| true),
        Ok(_) => Ok(false),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// A directory this run made under a random name, owner-only and held open.
///
/// Objects move into it before they are checked, so the check and the step after it reach
/// them through this handle, not through a name in the checkout. On drop it removes the
/// files it created, if they are still those files, and then itself if it is empty.
/// Anything else stays, so in a checkout that stays the same during the run, a failure
/// never removes what it did not check.
struct Holding<'a> {
    parent: &'a Dir,
    name: OsString,
    dir: Dir,
    id: Id,
    /// Files it may remove, by name, with the identity each had when it was created.
    owned: Vec<(OsString, Id)>,
    closed: bool,
}

impl<'a> Holding<'a> {
    fn create(parent: &'a Dir) -> std::io::Result<Self> {
        for _ in 0..NAME_ATTEMPTS {
            let name = OsString::from(format!("{HOLD_PREFIX}{:016x}", random()));
            match private_dir(parent, &name) {
                Ok((dir, id)) => {
                    return Ok(Self {
                        parent,
                        name,
                        dir,
                        id,
                        owned: Vec::new(),
                        closed: false,
                    });
                }
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::other(
            MigratePhrase::IoNoFreeName.as_static(),
        ))
    }

    /// The path of `inner` for messages, where `at` is the parent directory below the project.
    fn shown(&self, at: &Path, inner: &str) -> String {
        shown(&at.join(&self.name).join(inner))
    }

    /// Stage `bytes` with `mode` as [`NEW`].
    fn stage(
        &mut self,
        bytes: &[u8],
        mode: u32,
        rel: &Path,
        hook: Hook<'_>,
    ) -> std::io::Result<Id> {
        write_new(
            &self.dir,
            OsStr::new(NEW),
            bytes,
            mode,
            &mut self.owned,
            rel,
            hook,
        )
    }

    /// Move `name` in `from` in here as `inner`. Fails instead of replacing anything.
    fn take(&self, from: &Dir, name: &OsStr, inner: &OsStr) -> std::io::Result<()> {
        rename_noreplace(from, name, &self.dir, inner)
    }

    /// Move `inner` out to `name` in `to`. Fails instead of replacing anything.
    fn give(&mut self, inner: &str, to: &Dir, name: &OsStr) -> std::io::Result<()> {
        rename_noreplace(&self.dir, OsStr::new(inner), to, name)?;
        self.owned.retain(|(n, _)| n != inner);
        Ok(())
    }

    /// Put the taken original back under `name` in `to`. Fails, and keeps it here, when the
    /// name was taken again.
    fn give_back(&mut self, to: &Dir, name: &OsStr, rel: &Path) -> Result<(), Error> {
        match self.give(OLD, to, name) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(changed_with(
                rel,
                &MigrateText::ReplacedTwice {
                    kept: self.shown(rel.parent().unwrap_or(Path::new("")), OLD),
                }
                .to_string(),
            )),
            Err(e) => Err(Error::io(rel, e)),
        }
    }

    /// Remove every file it created, if each is still that file, then the directory itself.
    fn close(mut self) -> std::io::Result<()> {
        self.closed = true;
        for (name, id) in std::mem::take(&mut self.owned) {
            remove_if(&self.dir, &name, id)?;
        }
        let same = self
            .parent
            .symlink_metadata(&self.name)
            .is_ok_and(|m| m.is_dir() && id_of(&m) == self.id);
        if !same {
            return Err(std::io::Error::other(
                MigratePhrase::IoHoldingReplaced.as_static(),
            ));
        }
        self.parent.remove_dir(&self.name)
    }
}

impl Drop for Holding<'_> {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        // Best effort: the error that led here is the one to report. A directory that still
        // holds something is not empty, so it stays.
        for (name, id) in &self.owned {
            let _ = remove_if(&self.dir, name, *id);
        }
        if self
            .parent
            .symlink_metadata(&self.name)
            .is_ok_and(|m| m.is_dir() && id_of(&m) == self.id)
        {
            let _ = self.parent.remove_dir(&self.name);
        }
    }
}

/// Move each of `files` from `dir` into a holding directory, check it there and remove it.
/// A file that is no longer the one recorded stays in the holding directory, and the step
/// fails. `at` is `dir` below the project, for messages and the hook.
fn remove_checked(
    dir: &Dir,
    files: &[(OsString, Id)],
    at: &Path,
    hook: Hook<'_>,
) -> Result<(), Error> {
    let io = |e| Error::io(at, e);
    let hold = Holding::create(dir).map_err(io)?;
    for (name, id) in files {
        let rel = at.join(name);
        hold.take(dir, name, name).map_err(|e| Error::io(&rel, e))?;
        hook(Point::Cleanup(&rel)).map_err(|e| Error::io(&rel, e))?;
        if !remove_if(&hold.dir, name, *id).map_err(|e| Error::io(&rel, e))? {
            return Err(changed_with(
                &rel,
                &MigrateText::NotRemovedKept {
                    kept: shown(&at.join(&hold.name).join(name)),
                }
                .to_string(),
            ));
        }
    }
    hold.close().map_err(io)
}

/// A change that failed, and whether the target had already changed when it did.
struct Failed {
    error: Error,
    committed: bool,
}

impl From<Error> for Failed {
    fn from(error: Error) -> Self {
        Self {
            error,
            committed: false,
        }
    }
}

/// Whether the taken original is the file the preflight saw, with `digest`.
fn check_taken(hold: &Holding<'_>, rel: &Path, id: Id, digest: Option<Sha>) -> Result<(), Error> {
    let Some(seen) = read_entry(&hold.dir, OsStr::new(OLD), rel)? else {
        return Err(changed(rel, MigratePhrase::WhyVanishedReplaced));
    };
    if seen.id != id {
        return Err(changed(rel, MigratePhrase::WhyReplacedAfterCheck));
    }
    if digest.is_some_and(|d| d != seen.digest) {
        return Err(changed(rel, MigratePhrase::WhyChangedAfterCheck));
    }
    Ok(())
}

/// Replace `rel` below `root` with `new`, or remove it for `None`, if and only if it is what
/// `want` says at that moment.
///
/// The new bytes are staged in a holding directory next to the target, and `staged` learns
/// the staged file's identity before anything changes. The target then moves into the
/// holding directory and is checked there, through its handle. An unexpected object goes
/// back under its name. The staged file takes the name, and the object under the name is
/// checked to be the staged file before the original is removed. Every rename refuses to
/// replace an existing name, so a rename never overwrites an object. The original is
/// removed by a check and an unlink by name in the holding directory; the module docs name
/// the risk between the two.
// TODO(8l4.12.37): move to the compare-and-swap write in slopvac-fs when it lands.
fn mutate(
    root: &Dir,
    rel: &Path,
    want: Want,
    new: Option<(&[u8], u32)>,
    hook: Hook<'_>,
    staged: &mut dyn FnMut(Id) -> Result<(), Error>,
) -> Result<(), Failed> {
    let io = |e| Error::io(rel, e);
    let Some((parent, name)) = open_parent(root, rel)? else {
        return Err(changed(rel, MigratePhrase::WhyDirectoryVanished).into());
    };
    let at = rel.parent().unwrap_or(Path::new(""));
    let mut hold = Holding::create(&parent).map_err(io)?;
    let new_id = match new {
        Some((bytes, mode)) => {
            let id = hold.stage(bytes, mode, rel, hook).map_err(io)?;
            staged(id)?;
            Some(id)
        }
        None => None,
    };
    hook(Point::Check(rel)).map_err(io)?;
    let original = match want {
        Want::Absent => None,
        Want::File { id, digest } => {
            match hold.take(&parent, &name, OsStr::new(OLD)) {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    return Err(changed(rel, MigratePhrase::WhyVanishedAfterCheck).into());
                }
                Err(e) => return Err(io(e).into()),
            }
            if let Err(e) = check_taken(&hold, rel, id, digest) {
                return Err(hold
                    .give_back(&parent, &name, rel)
                    .err()
                    .unwrap_or(e)
                    .into());
            }
            Some(id)
        }
    };
    if let Err(e) = hook(Point::Install(rel)) {
        let e = io(e);
        return Err(match original {
            Some(_) => hold.give_back(&parent, &name, rel).err().unwrap_or(e),
            None => e,
        }
        .into());
    }
    if let Some(new_id) = new_id {
        if let Err(e) = hold.give(NEW, &parent, &name) {
            let e = if e.kind() == ErrorKind::AlreadyExists {
                changed(rel, MigratePhrase::WhyAppearedAfterCheck)
            } else {
                io(e)
            };
            return Err(match original {
                Some(_) => hold.give_back(&parent, &name, rel).err().unwrap_or(e),
                None => e,
            }
            .into());
        }
        let placed = parent
            .symlink_metadata(&name)
            .is_ok_and(|m| m.is_file() && id_of(&m) == new_id);
        if !placed {
            let why = match original {
                Some(_) => MigrateText::ReplacedWhileWrittenKept {
                    kept: hold.shown(at, OLD),
                }
                .to_string(),
                None => MigratePhrase::WhyReplacedWhileWritten
                    .as_static()
                    .to_owned(),
            };
            return Err(Failed {
                error: changed_with(rel, &why),
                committed: true,
            });
        }
    } else if original.is_none() && parent.symlink_metadata(&name).is_ok() {
        return Err(changed(rel, MigratePhrase::WhyAppearedAfterCheck).into());
    }
    let commit = |error| Failed {
        error,
        committed: true,
    };
    hook(Point::Discard(rel)).map_err(|e| commit(io(e)))?;
    if let Some(id) = original {
        let removed = remove_if(&hold.dir, OsStr::new(OLD), id).map_err(|e| commit(io(e)))?;
        if !removed {
            return Err(commit(changed_with(
                rel,
                &MigrateText::HeldChanged {
                    path: hold.shown(at, OLD),
                }
                .to_string(),
            )));
        }
    }
    hold.close().map_err(|e| commit(io(e)))?;
    sync_dir(&parent).map_err(|e| commit(io(e)))
}

/// One file the migration touches.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    path: String,
    before: Option<Sha>,
    after: Option<Sha>,
    /// Permission bits of the original, or of the new file when there was none.
    mode: u32,
    /// The change happened.
    done: bool,
    /// The file the migration staged for this path, saved before it took the name. A
    /// target is the migration's own only with this identity and the `after` digest.
    installed: Option<Id>,
}

/// The name of the backup copy of entry `index`.
fn copy_name(index: usize) -> OsString {
    OsString::from(index.to_string())
}

/// Where one migration keeps its backup: its own directory below `.slopvac/migrate-backup`,
/// by name and identity, and the identity of each directory above it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Location {
    project: Level,
    backups: Level,
    name: String,
    id: Id,
}

impl Location {
    /// The backup directory below the project, for messages and the hook.
    fn shown(&self) -> PathBuf {
        Path::new(BACKUP_DIR).join(&self.name)
    }
}

fn render_level(level: Level) -> String {
    format!("{} {}", opt_id(Some(level.id)), u8::from(level.made))
}

fn parse_level(text: &str) -> Option<Level> {
    let (id, made) = text.rsplit_once(' ')?;
    let made = match made {
        "0" => false,
        "1" => true,
        _ => return None,
    };
    Some(Level {
        id: parse_id(id)?,
        made,
    })
}

/// The trusted state of one migration.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Record {
    instance: String,
    root: Id,
    backup: Option<Location>,
    manifest: Option<Sha>,
    entries: Vec<Entry>,
}

fn opt_hex(d: Option<&Sha>) -> String {
    d.map_or_else(|| "-".to_owned(), hex)
}

fn opt_id(id: Option<Id>) -> String {
    id.map_or_else(|| "-".to_owned(), |i| format!("{} {}", i.dev, i.ino))
}

fn parse_id(text: &str) -> Option<Id> {
    let (dev, ino) = text.split_once(' ')?;
    Some(Id {
        dev: dev.parse().ok()?,
        ino: ino.parse().ok()?,
    })
}

/// `-` for no file, else a hex digest. `Err` for anything else.
fn parse_digest(text: &str) -> Result<Option<Sha>, ()> {
    if text == "-" {
        Ok(None)
    } else {
        unhex(text).map(Some).ok_or(())
    }
}

impl Record {
    fn render(&self) -> String {
        let (backup, project, backups) = self.backup.as_ref().map_or_else(
            || ("-".to_owned(), "-".to_owned(), "-".to_owned()),
            |l| {
                (
                    format!("{} {}", opt_id(Some(l.id)), l.name),
                    render_level(l.project),
                    render_level(l.backups),
                )
            },
        );
        // ast-grep-ignore: user-text-literal
        let head = format!(
            "{RECORD_HEADER}\ninstance {}\nroot {}\nbackup {backup}\nproject {project}\n\
             backups {backups}\nmanifest {}\n",
            self.instance,
            opt_id(Some(self.root)),
            opt_hex(self.manifest.as_ref()),
        );
        let entries = self.entries.iter().map(|e| {
            // ast-grep-ignore: user-text-literal
            format!(
                "entry {} {} {} {:o} {} {}\n",
                u8::from(e.done),
                opt_hex(e.before.as_ref()),
                opt_hex(e.after.as_ref()),
                e.mode,
                e.installed
                    .map_or_else(|| "-".to_owned(), |i| format!("{}:{}", i.dev, i.ino)),
                e.path
            )
        });
        std::iter::once(head).chain(entries).collect()
    }

    fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next()? != RECORD_HEADER {
            return None;
        }
        let mut field = |key: &str| lines.next()?.strip_prefix(key)?.strip_prefix(' ');
        let instance = field("instance")?.to_owned();
        let root = parse_id(field("root")?)?;
        let (backup, project, backups) = (field("backup")?, field("project")?, field("backups")?);
        let backup = if (backup, project, backups) == ("-", "-", "-") {
            None
        } else {
            let mut parts = backup.splitn(3, ' ');
            let id = Id {
                dev: parts.next()?.parse().ok()?,
                ino: parts.next()?.parse().ok()?,
            };
            let name = parts.next().filter(|n| is_backup_name(n))?;
            Some(Location {
                project: parse_level(project)?,
                backups: parse_level(backups)?,
                name: name.to_owned(),
                id,
            })
        };
        let manifest = parse_digest(field("manifest")?).ok()?;
        let mut entries = Vec::new();
        let mut paths = BTreeSet::new();
        for line in lines {
            let mut parts = line.strip_prefix("entry ")?.splitn(6, ' ');
            let done = match parts.next()? {
                "0" => false,
                "1" => true,
                _ => return None,
            };
            let before = parse_digest(parts.next()?).ok()?;
            let after = parse_digest(parts.next()?).ok()?;
            let mode = u32::from_str_radix(parts.next()?, 8)
                .ok()
                .filter(|m| *m <= 0o777)?;
            let installed = match parts.next()? {
                "-" => None,
                text => {
                    let (dev, ino) = text.split_once(':')?;
                    Some(Id {
                        dev: dev.parse().ok()?,
                        ino: ino.parse().ok()?,
                    })
                }
            };
            let path = parts.next()?.to_owned();
            if plain(Path::new(&path)).is_err() || !paths.insert(path.clone()) {
                return None;
            }
            entries.push(Entry {
                path,
                before,
                after,
                mode,
                done,
                installed,
            });
        }
        Some(Self {
            instance,
            root,
            backup,
            manifest,
            entries,
        })
    }

    /// [`Record::parse`] over the bytes of a saved record.
    fn from_bytes(bytes: Vec<u8>) -> Option<Self> {
        match String::from_utf8(bytes) {
            Ok(text) => Self::parse(&text),
            Err(_) => None,
        }
    }

    fn manifest_text(&self) -> String {
        let head = format!("{MANIFEST_HEADER} {}\n", self.instance);
        let lines = self.entries.iter().enumerate().map(|(i, e)| {
            let copy = e.before.map_or_else(|| "-".to_owned(), |_| i.to_string());
            format!("{copy} {}\n", e.path)
        });
        std::iter::once(head).chain(lines).collect()
    }

    /// The names a complete backup directory holds.
    fn backup_names(&self) -> BTreeSet<OsString> {
        let copies = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.before.is_some())
            .map(|(i, _)| copy_name(i));
        copies.chain([OsString::from(MANIFEST)]).collect()
    }
}

/// Where the record for one project lives in the per-user cache directory.
struct Store {
    dir: Dir,
    name: PathBuf,
    shown: PathBuf,
}

impl Store {
    /// The record name is the digest of the project's canonical path, so two projects never
    /// share one. The record itself says which checkout it belongs to.
    fn name(root: &Path) -> Result<PathBuf, Error> {
        let canonical = std::fs::canonicalize(root).map_err(|e| Error::io(root, e))?;
        Ok(Path::new(RECORD_DIR).join(hex(&sha(canonical.as_os_str().as_bytes()))))
    }

    fn create(root: &Path, state: &Path) -> Result<Self, Error> {
        std::fs::create_dir_all(state).map_err(|e| Error::io(state, e))?;
        let name = Self::name(root)?;
        let dir =
            Dir::open_ambient_dir(state, ambient_authority()).map_err(|e| Error::io(state, e))?;
        Ok(Self {
            dir,
            shown: state.join(&name),
            name,
        })
    }

    fn find(root: &Path, state: &Path) -> Result<Option<Self>, Error> {
        let name = Self::name(root)?;
        let Ok(dir) = Dir::open_ambient_dir(state, ambient_authority()) else {
            return Ok(None);
        };
        Ok(Some(Self {
            dir,
            shown: state.join(&name),
            name,
        }))
    }

    fn save(&self, record: &Record) -> Result<(), Error> {
        write_atomic(
            &self.dir,
            &self.name,
            record.render().as_bytes(),
            WriteOptions::new()
                .new_file_mode(BACKUP_FILE_MODE)
                .create_parents(true),
        )
        .map_err(|e| match e {
            slopvac_fs::Error::Io { source, .. } => Error::io(&self.shown, source),
            other => Error::io(&self.shown, std::io::Error::other(other.to_string())),
        })
    }

    /// The bytes of the saved record, if one exists.
    fn read(&self) -> Result<Option<Vec<u8>>, Error> {
        Ok(look(&self.dir, &self.name)?.map(|seen| seen.bytes))
    }

    fn load(&self) -> Result<Option<Record>, Error> {
        self.read()?
            .map(|bytes| Record::from_bytes(bytes).ok_or_else(unverified))
            .transpose()
    }

    fn remove(&self) -> Result<(), Error> {
        match self.dir.remove_file(&self.name) {
            Err(e) if e.kind() != ErrorKind::NotFound => Err(Error::io(&self.shown, e)),
            _ => Ok(()),
        }
    }
}
fn open_root(root: &Path) -> Result<(Dir, Id), Error> {
    let dir = Dir::open_ambient_dir(root, ambient_authority()).map_err(|e| Error::io(root, e))?;
    let id = id_of(&dir.dir_metadata().map_err(|e| Error::io(root, e))?);
    Ok((dir, id))
}

/// The backup directory this run created, the directories above it, and the files it put
/// in it.
struct Owned {
    project: Dir,
    backups: Dir,
    location: Location,
    dir: Dir,
    files: Vec<(OsString, Id)>,
}

/// Remove what this run put in its backup directory, then the directory, then the
/// directories above it that this run created. Used when apply fails before it changed any
/// target.
fn discard(root: &Dir, owned: &Owned) {
    // Best effort throughout: the error that led here is the one to report, and the record
    // goes next, so nothing trusts what is left. A file that is not the one this run wrote
    // stays, and so do the directories then.
    let at = owned.location.shown();
    let _ = remove_checked(&owned.dir, &owned.files, &at, &mut |_| Ok(()));
    let loc = &owned.location;
    if same_dir(&owned.backups, &loc.name, loc.id) {
        // Removes only an empty directory.
        let _ = owned.backups.remove_dir(&loc.name);
    }
    remove_made(&owned.project, BACKUPS_NAME, loc.backups);
    remove_made(root, PROJECT_DIR, loc.project);
}

/// Create a new directory in `backups` named `now`. A name that is taken gets a numbered
/// suffix, so a backup directory is never reused.
fn run_dir(backups: &Dir, now: &str) -> Result<(Dir, Id, String), Error> {
    for n in 0..NAME_ATTEMPTS {
        let name = if n == 0 {
            now.to_owned()
        } else {
            format!("{now}-{n}")
        };
        match private_dir(backups, OsStr::new(&name)) {
            Ok((dir, id)) => return Ok((dir, id, name)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(Error::io(&Path::new(BACKUP_DIR).join(&name), e)),
        }
    }
    Err(Error::io(
        Path::new(BACKUP_DIR),
        std::io::Error::other(MigratePhrase::IoNoFreeBackupName.as_static()),
    ))
}

/// Create the backup directory, and `.slopvac` and `migrate-backup` above it where they are
/// missing, and write the copies and the manifest into it. On failure, whatever it created
/// is gone again. `now` names the backup directory.
fn make_backup(
    root: &Dir,
    record: &Record,
    plan: &Plan,
    now: &str,
    hook: Hook<'_>,
) -> Result<Owned, Error> {
    let (project, project_level) = ensure_level(root, PROJECT_DIR, Path::new(PROJECT_DIR))?;
    let made = ensure_level(&project, BACKUPS_NAME, Path::new(BACKUP_DIR)).and_then(
        |(backups, backups_level)| match run_dir(&backups, now) {
            Ok(run) => Ok((backups, backups_level, run)),
            Err(e) => {
                remove_made(&project, BACKUPS_NAME, backups_level);
                Err(e)
            }
        },
    );
    let (backups, backups_level, (dir, id, name)) = match made {
        Ok(made) => made,
        Err(e) => {
            remove_made(root, PROJECT_DIR, project_level);
            return Err(e);
        }
    };
    let mut owned = Owned {
        project,
        backups,
        location: Location {
            project: project_level,
            backups: backups_level,
            name,
            id,
        },
        dir,
        files: Vec::new(),
    };
    let shown_dir = owned.location.shown();
    let manifest = record.manifest_text();
    let files = plan
        .changes
        .iter()
        .enumerate()
        .filter_map(|(i, c)| Some((copy_name(i), c.old.as_deref()?.as_bytes())))
        .chain([(OsString::from(MANIFEST), manifest.as_bytes())]);
    for (name, bytes) in files {
        let rel = shown_dir.join(&name);
        let written = write_new(
            &owned.dir,
            &name,
            bytes,
            BACKUP_FILE_MODE,
            &mut owned.files,
            &rel,
            hook,
        )
        .and_then(|_| sync_dir(&owned.dir));
        if let Err(e) = written {
            discard(root, &owned);
            return Err(Error::io(&rel, e));
        }
    }
    Ok(owned)
}

/// Refuse `.slopvac` or `migrate-backup` when it is a link or not a directory, and refuse
/// while the backup that `previous` names exists, so a second write never comes before the
/// restore of the first. Other backup directories do not count.
fn guard(dir: &Dir, previous: Option<&Record>) -> Result<(), Error> {
    let Some(project) = open_level(dir, PROJECT_DIR, Path::new(PROJECT_DIR))? else {
        return Ok(());
    };
    let Some(backups) = open_level(&project, BACKUPS_NAME, Path::new(BACKUP_DIR))? else {
        return Ok(());
    };
    let Some(loc) = previous.and_then(|r| r.backup.as_ref()) else {
        return Ok(());
    };
    match backups.symlink_metadata(&loc.name) {
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(exists(&loc.shown())),
        Err(e) => Err(Error::io(&loc.shown(), e)),
    }
}

/// Check the plan's paths, the backup directories and what each target holds now, before
/// anything is written. `previous` is the record of an earlier run in this checkout.
fn preflight(
    dir: &Dir,
    plan: &Plan,
    previous: Option<&Record>,
) -> Result<(Vec<Want>, Vec<Entry>), Error> {
    let mut paths = BTreeSet::new();
    for c in &plan.changes {
        let key: PathBuf = plain(&c.path)?.into_iter().collect();
        if !paths.insert(key) {
            return Err(refused(&c.path, MigratePhrase::WhyNamedTwice));
        }
    }
    guard(dir, previous)?;
    let mut wants = Vec::with_capacity(plan.changes.len());
    let mut entries = Vec::with_capacity(plan.changes.len());
    for c in &plan.changes {
        let seen = look(dir, &c.path)?;
        let planned = c.old.as_deref().map(|t| sha(t.as_bytes()));
        if seen.as_ref().map(|s| s.digest) != planned {
            return Err(changed(&c.path, MigratePhrase::WhyChangedAfterPlan));
        }
        let after = match c.kind {
            ChangeKind::Write => c.new.as_deref().map(|t| sha(t.as_bytes())),
            ChangeKind::Delete => None,
        };
        wants.push(Want::of(seen.as_ref(), true));
        entries.push(Entry {
            path: c
                .path
                .components()
                .collect::<PathBuf>()
                .display()
                .to_string(),
            before: planned,
            after,
            mode: seen.as_ref().map_or(NEW_FILE_MODE, |s| s.mode),
            done: false,
            installed: None,
        });
    }
    Ok((wants, entries))
}

/// Write the plan to disk. Replaced and deleted files are copied first to a new directory
/// below [`BACKUP_DIR`], named for the UTC time, and the run is recorded in `state`, the
/// per-user cache directory, which [`restore`] trusts. A second write is refused while the
/// backup that the record names exists, so that backup is never lost. Other directories
/// below [`BACKUP_DIR`] are left alone.
///
/// Every target must still hold what the plan read, with one link and no symbolic link on
/// the way, and `.slopvac` and `migrate-backup` must be directories, not links; otherwise
/// nothing is written. When a change fails before any target changed, the backup and the
/// record are removed again, and so are the directories above the backup that this run
/// created.
pub fn apply(root: &Path, plan: &Plan, state: &Path) -> Result<(), Error> {
    apply_hooked(root, plan, state, &mut |_| Ok(()))
}

/// [`apply`] with a callback that runs at each [`Point`]. An error from it fails that step.
/// Tests use it to swap an object or fail a write at the worst moment.
#[doc(hidden)]
pub fn apply_hooked(
    root: &Path,
    plan: &Plan,
    state: &Path,
    hook: &mut dyn FnMut(Point<'_>) -> std::io::Result<()>,
) -> Result<(), Error> {
    let (dir, root_id) = open_root(root)?;
    // A record that does not parse may name a backup that was never restored, so apply
    // stops. A record of another checkout at this path names no backup that restore could
    // use here.
    let previous = match Store::find(root, state)? {
        Some(store) => match store.read()? {
            Some(bytes) => Some(Record::from_bytes(bytes).ok_or_else(|| unreadable(&store.name))?),
            None => None,
        },
        None => None,
    }
    .filter(|r| r.root == root_id);
    let (wants, entries) = preflight(&dir, plan, previous.as_ref())?;
    let now = name_at(SystemTime::now())?;
    let store = Store::create(root, state)?;
    let mut record = Record {
        instance: nonce()?,
        root: root_id,
        backup: None,
        manifest: None,
        entries,
    };
    store.save(&record)?;
    let owned = match make_backup(&dir, &record, plan, &now, hook) {
        Ok(owned) => owned,
        Err(e) => {
            let _ = store.remove();
            return Err(e);
        }
    };
    record.backup = Some(owned.location.clone());
    record.manifest = Some(sha(record.manifest_text().as_bytes()));
    if let Err(e) = store.save(&record) {
        discard(&dir, &owned);
        let _ = store.remove();
        return Err(e);
    }
    for (i, (c, want)) in plan.changes.iter().zip(wants).enumerate() {
        let mode = record.entries.get(i).map_or(NEW_FILE_MODE, |e| e.mode);
        let new = match c.kind {
            ChangeKind::Write => c.new.as_deref().map(|t| (t.as_bytes(), mode)),
            ChangeKind::Delete => None,
        };
        // The staged file's identity is saved before it takes the name, so restore can tell
        // the migration's own file from one with the same bytes.
        let mut staged = |id| {
            if let Some(entry) = record.entries.get_mut(i) {
                entry.installed = Some(id);
            }
            store.save(&record)
        };
        if let Err(failed) = mutate(&dir, &c.path, want, new, hook, &mut staged) {
            if failed.committed {
                // The target changed, so restore must undo it. Best effort: restore finds
                // the migration's own file by its saved identity in any case.
                if let Some(entry) = record.entries.get_mut(i) {
                    entry.done = true;
                }
                let _ = store.save(&record);
            } else if i == 0 {
                // Nothing changed yet, so nothing needs the backup or the record.
                discard(&dir, &owned);
                let _ = store.remove();
            }
            return Err(failed.error);
        }
        if let Some(entry) = record.entries.get_mut(i) {
            entry.done = true;
        }
        store.save(&record)?;
    }
    Ok(())
}

/// The outcome of [`restore`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Restored {
    /// Files now back in their state before the migration.
    pub restored: usize,
    /// Files left as they are. With `--force`, a target that is not the file the migration
    /// installed, or a changed file where the migration created one. In any run, a target
    /// that a stopped apply never changed and that differs now.
    pub kept: Vec<PathBuf>,
}

/// What restoring one entry does.
enum Step {
    /// Already as before the migration.
    Done,
    /// Put the backup back, or delete a created file.
    Undo(Want),
    /// Leave the file as it is.
    Keep,
}

/// The checked record and backup directory.
struct Verified {
    record: Record,
    /// Where the record says the backup is, with a handle on each directory on the way.
    location: Location,
    project: Dir,
    backups: Dir,
    backup: Dir,
    /// Each name in the backup directory with the file it held when it was checked.
    files: Vec<(OsString, Id)>,
    /// The bytes of each entry's backup copy.
    copies: Vec<Option<Vec<u8>>>,
}

/// Check the record against the checkout and the backup directory, before anything changes.
fn verify(root: &Path, dir: &Dir, root_id: Id, state: &Path) -> Result<(Verified, Store), Error> {
    let store = Store::find(root, state)?.ok_or_else(unverified)?;
    let record = store.load()?.ok_or_else(unverified)?;
    if record.root != root_id {
        return Err(unverified());
    }
    let (Some(location), Some(manifest)) = (record.backup.clone(), record.manifest) else {
        return Err(unverified());
    };
    let shown_dir = location.shown();
    let open = |parent: &Dir, name: &str, id: Id| -> Result<Dir, Error> {
        let next = parent.open_dir_nofollow(name).map_err(|_| unverified())?;
        let meta = next.dir_metadata().map_err(|e| Error::io(&shown_dir, e))?;
        if id_of(&meta) == id {
            Ok(next)
        } else {
            Err(unverified())
        }
    };
    let project = open(dir, PROJECT_DIR, location.project.id)?;
    let backups = open(&project, BACKUPS_NAME, location.backups.id)?;
    let backup = open(&backups, &location.name, location.id)?;
    let io = |e| Error::io(&shown_dir, e);
    let expected = record.backup_names();
    let mut found = BTreeSet::new();
    for entry in backup.entries().map_err(io)? {
        found.insert(entry.map_err(io)?.file_name());
    }
    if found != expected {
        return Err(Error::Unsafe(
            MigrateText::BackupForeignFiles {
                directory: shown(&shown_dir),
            }
            .to_string(),
        ));
    }
    let read =
        |name: &OsStr| read_entry(&backup, name, &shown_dir.join(name))?.ok_or_else(unverified);
    let mut files = Vec::with_capacity(expected.len());
    let m = read(OsStr::new(MANIFEST))?;
    if m.digest != manifest {
        return Err(unverified());
    }
    files.push((OsString::from(MANIFEST), m.id));
    let mut copies = Vec::with_capacity(record.entries.len());
    for (i, e) in record.entries.iter().enumerate() {
        let Some(before) = e.before else {
            copies.push(None);
            continue;
        };
        let name = copy_name(i);
        let copy = read(&name)?;
        if copy.digest != before {
            return Err(Error::Unsafe(
                MigrateText::BackupCopyMismatch {
                    path: shown(Path::new(&e.path)),
                }
                .to_string(),
            ));
        }
        files.push((name, copy.id));
        copies.push(Some(copy.bytes));
    }
    let verified = Verified {
        record,
        location,
        project,
        backups,
        backup,
        files,
        copies,
    };
    Ok((verified, store))
}

/// Decide each entry's step from what the target holds now. Fails before anything changes
/// on a target that differs from both sides, unless `force` is set.
fn decide(dir: &Dir, record: &Record, force: bool) -> Result<Vec<Step>, Error> {
    let mut steps = Vec::with_capacity(record.entries.len());
    for e in &record.entries {
        let rel = Path::new(&e.path);
        let now = look(dir, rel)?;
        let digest = now.as_ref().map(|s| s.digest);
        // The migration's own result: the file it staged, by identity, with the bytes it
        // wrote, or no file where it deleted one. Same bytes alone prove nothing.
        let installed = now.as_ref().is_some_and(|s| Some(s.id) == e.installed);
        let own = match e.after {
            Some(after) => installed && digest == Some(after),
            None => now.is_none(),
        };
        let step = if digest == e.before {
            Step::Done
        } else if own {
            Step::Undo(Want::of(now.as_ref(), true))
        } else if !e.done {
            Step::Keep
        } else if !force {
            return Err(changed(rel, MigratePhrase::WhyChangedAfterWrite));
        } else if e.before.is_some() && (installed || now.is_none()) {
            // `force` relaxes only the content check: the object is still the file the
            // migration installed, edited in place, or the name is empty.
            Step::Undo(Want::of(now.as_ref(), false))
        } else {
            // Not the migration's object, or a created file that changed: left as it is.
            Step::Keep
        };
        steps.push(step);
    }
    Ok(steps)
}

/// Undo the last [`apply`]: put backed-up files back and remove files it created.
///
/// Nothing changes unless every check passes first:
///
/// - The record that [`apply`] wrote to `state` exists and names this checkout: the same
///   root directory, by device and inode.
/// - The backup directory is the one the record names, the one that run created, by name
///   and by device and inode, and so are `.slopvac` and `migrate-backup` above it. Any other
///   directory below [`BACKUP_DIR`], a newer one included, is never read. The backup holds
///   exactly the files that run wrote, each with its recorded digest.
/// - Each target is the migration's own result, or already holds its original bytes. The
///   migration's own file is the one it staged, by device and inode, with the bytes it
///   wrote; a file with the same bytes is not.
///
/// With `force`, a target whose content changed is put back anyway, but only when it is
/// still the file the migration installed, or the name is empty. Any other object at the
/// path is kept, and so is a changed file where the migration created one. `force` never
/// skips the other checks or the link checks.
///
/// A target that is not the migration's own and whose change never completed is left as it
/// is. A file is deleted only if it is the migration's own.
///
/// Afterwards the backup directory is removed, then `migrate-backup` and `.slopvac` if that
/// run created them and they are empty now. A `.slopvac` that existed before stays.
pub fn restore(root: &Path, state: &Path, force: bool) -> Result<Restored, Error> {
    restore_hooked(root, state, force, &mut |_| Ok(()))
}

/// [`restore`] with a callback that runs at each [`Point`], as for [`apply_hooked`].
#[doc(hidden)]
pub fn restore_hooked(
    root: &Path,
    state: &Path,
    force: bool,
    hook: &mut dyn FnMut(Point<'_>) -> std::io::Result<()>,
) -> Result<Restored, Error> {
    let (dir, root_id) = open_root(root)?;
    let (verified, store) = verify(root, &dir, root_id, state)?;
    let steps = decide(&dir, &verified.record, force)?;
    let mut out = Restored::default();
    let work = verified
        .record
        .entries
        .iter()
        .zip(steps)
        .zip(&verified.copies);
    for ((e, step), copy) in work {
        let rel = Path::new(&e.path);
        match step {
            Step::Done => out.restored += 1,
            Step::Keep => out.kept.push(rel.to_owned()),
            Step::Undo(want) => {
                let new = copy.as_deref().map(|bytes| (bytes, e.mode));
                mutate(&dir, rel, want, new, hook, &mut |_| Ok(())).map_err(|f| f.error)?;
                out.restored += 1;
            }
        }
    }
    clean_up(&dir, &verified, hook)?;
    store.remove()?;
    Ok(out)
}

/// Move each checked backup file into a holding directory inside the backup directory,
/// check it there and remove it, then remove the backup directory, and then the directories
/// above it that the migration created, when they are empty. A file that is no longer the
/// one that was checked stays, and so does the directory.
fn clean_up(dir: &Dir, verified: &Verified, hook: Hook<'_>) -> Result<(), Error> {
    let loc = &verified.location;
    let shown_dir = loc.shown();
    remove_checked(&verified.backup, &verified.files, &shown_dir, hook)?;
    if !same_dir(&verified.backups, &loc.name, loc.id) {
        return Err(changed(&shown_dir, MigratePhrase::WhyChangedDuringRestore));
    }
    // Removes only an empty directory.
    verified.backups.remove_dir(&loc.name).map_err(|e| {
        if e.kind() == ErrorKind::DirectoryNotEmpty {
            changed(&shown_dir, MigratePhrase::WhyGainedFiles)
        } else {
            Error::io(&shown_dir, e)
        }
    })?;
    remove_made(&verified.project, BACKUPS_NAME, loc.backups);
    remove_made(dir, PROJECT_DIR, loc.project);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use proptest::prelude::*;
    use rstest::rstest;

    use cap_std::ambient_authority;
    use cap_std::fs::Dir;

    use super::{name_at, run_dir, timestamp};

    // Expected values from `date -u -r <secs> +%Y%m%dT%H%M%SZ`.
    #[rstest]
    #[case::epoch(0, "19700101T000000Z")]
    #[case::leap_day_of_a_400_year(951_782_400, "20000229T000000Z")]
    #[case::leap_day_last_second(1_709_251_199, "20240229T235959Z")]
    #[case::autumn(1_795_156_800, "20261120T064000Z")]
    #[case::last_second_of_2099(4_102_444_799, "20991231T235959Z")]
    #[case::first_second_of_2100(4_102_444_800, "21000101T000000Z")]
    #[case::feb_28_2100(4_107_456_000, "21000228T000000Z")]
    #[case::last_second_of_feb_2100(4_107_542_399, "21000228T235959Z")]
    #[case::no_leap_day_in_2100(4_107_542_400, "21000301T000000Z")]
    fn timestamp_is_the_utc_time(#[case] secs: u64, #[case] expected: &str) {
        assert_eq!(timestamp(secs), expected);
    }

    #[test]
    fn the_name_drops_the_fraction_of_a_second() {
        let now = UNIX_EPOCH + Duration::new(1_795_156_800, 999_999_999);
        assert_eq!(name_at(now).unwrap(), "20261120T064000Z");
    }

    #[test]
    fn a_clock_before_1970_is_refused() {
        let now = UNIX_EPOCH - Duration::from_secs(1);
        let e = name_at(now).unwrap_err();
        assert_eq!(e.code(), "migrate-io-failed");
        assert!(
            e.to_string().starts_with(".slopvac/migrate-backup: "),
            "{e}"
        );
    }

    fn is_leap(year: u64) -> bool {
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
    }

    fn month_days(year: u64, month: u64) -> u64 {
        match month {
            2 if is_leap(year) => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        }
    }

    /// The Unix time of a `YYYYMMDDTHHMMSSZ` name, counted year by year and month by month,
    /// or `None` when a field is out of range.
    fn epoch_of(name: &str) -> Option<u64> {
        let b = name.as_bytes();
        if b.len() != 16 || b[8] != b'T' || b[15] != b'Z' {
            return None;
        }
        let num = |range: std::ops::Range<usize>| name.get(range)?.parse::<u64>().ok();
        let (year, month, day) = (num(0..4)?, num(4..6)?, num(6..8)?);
        let (hour, minute, second) = (num(9..11)?, num(11..13)?, num(13..15)?);
        let valid = (1970..=9999).contains(&year)
            && (1..=12).contains(&month)
            && (1..=month_days(year, month)).contains(&day)
            && hour < 24
            && minute < 60
            && second < 60;
        if !valid {
            return None;
        }
        let years: u64 = (1970..year)
            .map(|y| if is_leap(y) { 366 } else { 365 })
            .sum();
        let months: u64 = (1..month).map(|m| month_days(year, m)).sum();
        let days = years + months + day - 1;
        Some(days * 86_400 + hour * 3600 + minute * 60 + second)
    }

    #[rstest]
    #[case::one_taken(&["20261120T064000Z"], "20261120T064000Z-1")]
    #[case::two_taken(&["20261120T064000Z", "20261120T064000Z-1"], "20261120T064000Z-2")]
    fn a_taken_backup_name_gets_a_suffix_and_the_taken_one_stays(
        #[case] taken: &[&str],
        #[case] expected: &str,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        for name in taken {
            std::fs::create_dir(tmp.path().join(name)).unwrap();
            std::fs::write(tmp.path().join(name).join("0"), name).unwrap();
        }
        let backups = Dir::open_ambient_dir(tmp.path(), ambient_authority()).unwrap();

        let (_, _, name) = run_dir(&backups, "20261120T064000Z").unwrap();

        assert_eq!(name, expected);
        assert!(tmp.path().join(expected).is_dir());
        for name in taken {
            let kept = std::fs::read_to_string(tmp.path().join(name).join("0")).unwrap();
            assert_eq!(kept, *name);
        }
    }

    proptest! {
        #[test]
        fn timestamp_round_trips_through_a_reference_calendar(
            secs in 0u64..=253_402_300_799,
        ) {
            prop_assert_eq!(epoch_of(&timestamp(secs)), Some(secs));
        }
    }
}
