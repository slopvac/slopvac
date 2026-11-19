//! Applying a plan and undoing it, safe against a hostile checkout.
//!
//! The author of a pull request controls every file in the tree, the backup directory
//! included, so neither step trusts what it finds on disk:
//!
//! - Every path is walked from one handle on the project root, without following a link at
//!   any component. A symbolic link, a non-regular file and a file with more than one link
//!   are refused.
//! - Each change is conditional. The new bytes are staged under a random name first. The
//!   target is then renamed aside, and the object that moved is checked: the same file the
//!   preflight saw, with the expected bytes. Only then does the staged file take the name,
//!   with a rename that fails if the name was taken again. An unexpected object goes back
//!   under its name untouched, and the step fails.
//! - The trusted state is a record in the per-user cache directory, outside the project. It
//!   names the root and backup directories by device and inode, and it holds a random
//!   instance id, the digest of every file before and after, each file's mode, the digest of
//!   the backup manifest, and which changes completed. Restore acts only on what the record
//!   says this run did, and it refuses a backup directory with any other content.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::hash::{BuildHasher, Hasher};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
use cap_std::fs::{
    Dir, DirBuilder, DirBuilderExt, File, Metadata, MetadataExt, OpenOptions, OpenOptionsExt,
    Permissions, PermissionsExt,
};
use rustix::fs::RenameFlags;
use sha2::{Digest, Sha256};
use slopvac_fs::{WriteOptions, write_atomic};

use crate::Error;
use crate::plan::{BACKUP_DIR, ChangeKind, Plan};

/// The list of backup copies, for people. Restore checks its digest and never reads it.
const MANIFEST: &str = "manifest.txt";
const MANIFEST_HEADER: &str = "slopvac-migrate-backup";
const RECORD_HEADER: &str = "slopvac-migrate-record 2";
/// The directory below the per-user cache that holds one record per project.
const RECORD_DIR: &str = "migrate-backups";
/// Permission bits for a file that did not exist before.
const NEW_FILE_MODE: u32 = 0o644;
/// Permission bits for the backup directory, and for the files in it and the record.
const BACKUP_DIR_MODE: u32 = 0o700;
const BACKUP_FILE_MODE: u32 = 0o600;
/// How many random names to try before giving up.
const NAME_ATTEMPTS: usize = 16;

/// Runs before the final check of each change, with the project-relative path. Tests swap
/// the target here.
type Hook<'a> = &'a mut dyn FnMut(&Path) -> std::io::Result<()>;

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

/// A path as messages show it: control characters escaped, on one line.
fn shown(path: &Path) -> String {
    slopvac_sanitize::line(&path.display().to_string())
}

fn refused(path: &Path, why: &str) -> Error {
    Error::Unsafe(format!("{}: {why}", shown(path)))
}

fn changed(path: &Path, why: &str) -> Error {
    Error::Changed(format!("{}: {why}", shown(path)))
}

fn unverified() -> Error {
    Error::Unsafe(format!(
        "{BACKUP_DIR} was not written by `slopvac migrate --write` in this checkout, or it \
         changed since; nothing restored"
    ))
}

fn exists() -> Error {
    Error::Unsafe(format!(
        "{BACKUP_DIR} exists; run `slopvac migrate --restore` or delete it first"
    ))
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
        .map_err(|_| std::io::Error::other("O_NONBLOCK does not fit"))?;
    options.custom_flags(flag);
    Ok(())
}

/// Rename within `dir`, failing with `AlreadyExists` instead of replacing `to`.
fn rename_noreplace(dir: &Dir, from: &OsStr, to: &OsStr) -> std::io::Result<()> {
    rustix::fs::renameat_with(dir, from, dir, to, RenameFlags::NOREPLACE).map_err(Into::into)
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
            return Err(refused(path, "path leaves the project"));
        };
        if part.to_str().is_none() {
            return Err(refused(path, "path is not valid UTF-8"));
        }
        if part.as_bytes().contains(&b'\n') {
            return Err(refused(path, "newline in path"));
        }
        names.push(part);
    }
    if names.is_empty() {
        return Err(Error::Unsafe("empty path".into()));
    }
    Ok(names)
}

/// Open the directory that holds `rel` below `dir`, one component at a time without
/// following links. `None` when a directory on the way does not exist.
fn open_parent(dir: &Dir, rel: &Path) -> Result<Option<(Dir, OsString)>, Error> {
    let mut names = plain(rel)?;
    let Some(name) = names.pop() else {
        return Err(Error::Unsafe("empty path".into()));
    };
    let mut parent = dir.try_clone().map_err(|e| Error::io(rel, e))?;
    for part in names {
        parent = match parent.open_dir_nofollow(part) {
            Ok(next) => next,
            Err(e) => {
                return match parent.symlink_metadata(part) {
                    Err(m) if m.kind() == ErrorKind::NotFound => Ok(None),
                    Ok(m) if m.is_symlink() => {
                        Err(refused(rel, "a path component is a symbolic link"))
                    }
                    Ok(m) if !m.is_dir() => {
                        Err(refused(rel, "a path component is not a directory"))
                    }
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
        return Err(refused(rel, "is a symbolic link"));
    }
    if !entry.is_file() {
        return Err(refused(rel, "is not a regular file"));
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    nonblocking(&mut options).map_err(io)?;
    let file = parent.open_with(name, &options).map_err(io)?;
    let held = file.metadata().map_err(io)?;
    if !held.is_file() || id_of(&held) != id_of(&entry) {
        return Err(changed(rel, "changed while it was read"));
    }
    if held.nlink() > 1 {
        return Err(refused(rel, "has more than one hard link"));
    }
    let bytes =
        slopvac_fs::read_capped(file, slopvac_fs::MAX_DOCUMENT_BYTES).map_err(|e| match e {
            slopvac_fs::Error::CapExceeded { .. } => refused(rel, "is too large to migrate"),
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

/// A file under a temporary name, removed on drop unless it was handed over.
struct Staged<'a> {
    dir: &'a Dir,
    name: OsString,
    armed: bool,
}

impl Drop for Staged<'_> {
    fn drop(&mut self) {
        if self.armed {
            // Best effort: the error that led here is the one to report.
            let _ = self.dir.remove_file(&self.name);
        }
    }
}

fn temp_name(name: &OsStr, role: &str) -> OsString {
    let mut out = OsString::from(".");
    if name.len() <= 200 {
        out.push(name);
    }
    out.push(format!(".slopvac-{role}-{:016x}", random()));
    out
}

/// Create a new owner-only file in `dir` with `bytes`, without following a link.
fn create_file(dir: &Dir, name: &OsStr, bytes: &[u8]) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No)
        .mode(BACKUP_FILE_MODE);
    let mut file = dir.open_with(name, &options)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(file)
}

/// Stage `bytes` with `mode` under a fresh name next to `name`.
fn stage<'a>(dir: &'a Dir, name: &OsStr, bytes: &[u8], mode: u32) -> std::io::Result<Staged<'a>> {
    for _ in 0..NAME_ATTEMPTS {
        let temp = temp_name(name, "new");
        match create_file(dir, &temp, bytes) {
            Ok(file) => {
                let staged = Staged {
                    dir,
                    name: temp,
                    armed: true,
                };
                file.set_permissions(Permissions::from_mode(mode))?;
                file.sync_all()?;
                return Ok(staged);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("no free temporary name"))
}

/// Move `name` to a fresh name in `dir`, and return that name.
fn move_aside(dir: &Dir, name: &OsStr) -> std::io::Result<OsString> {
    for _ in 0..NAME_ATTEMPTS {
        let aside = temp_name(name, "old");
        match rename_noreplace(dir, name, &aside) {
            Ok(()) => return Ok(aside),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("no free temporary name"))
}

/// Whether the object moved to `aside` is the file the preflight saw, with `digest`.
fn check_aside(
    dir: &Dir,
    aside: &OsStr,
    rel: &Path,
    id: Id,
    digest: Option<Sha>,
) -> Result<(), Error> {
    let Some(seen) = read_entry(dir, aside, rel)? else {
        return Err(changed(rel, "disappeared while it was replaced"));
    };
    if seen.id != id {
        return Err(changed(rel, "was replaced after it was checked"));
    }
    if digest.is_some_and(|d| d != seen.digest) {
        return Err(changed(rel, "changed after it was checked"));
    }
    Ok(())
}

/// Rename `aside` back to `name`. Fails, and leaves `aside`, when the name is taken again.
fn put_back(dir: &Dir, aside: &OsStr, name: &OsStr, rel: &Path) -> Result<(), Error> {
    match rename_noreplace(dir, aside, name) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(changed(
            rel,
            &format!(
                "was replaced twice while it was checked; the first replacement is kept as {}",
                shown(Path::new(aside))
            ),
        )),
        Err(e) => Err(Error::io(rel, e)),
    }
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

/// Replace `rel` below `root` with `new`, or remove it for `None`, if and only if it is what
/// `want` says at that moment.
///
/// The target is renamed aside and checked there, so the check and the change act on the
/// same object. An unexpected object goes back under its name. Every rename refuses to
/// replace an existing name, so nothing that was not checked is overwritten or removed.
// TODO(8l4.12.37): move to the compare-and-swap write in slopvac-fs when it lands.
fn mutate(
    root: &Dir,
    rel: &Path,
    want: Want,
    new: Option<(&[u8], u32)>,
    hook: Hook<'_>,
) -> Result<(), Failed> {
    let (parent, aside) = swap_in(root, rel, want, new, hook)?;
    // The target has its new state now; what is left only tidies up.
    aside
        .map_or(Ok(()), |aside| parent.remove_file(aside))
        .and_then(|()| sync_dir(&parent))
        .map_err(|e| Failed {
            error: Error::io(rel, e),
            committed: true,
        })
}

/// The part of [`mutate`] up to the moment the target changes. Returns the parent directory
/// and the name the checked original was moved to, if any.
fn swap_in(
    root: &Dir,
    rel: &Path,
    want: Want,
    new: Option<(&[u8], u32)>,
    hook: Hook<'_>,
) -> Result<(Dir, Option<OsString>), Error> {
    let io = |e| Error::io(rel, e);
    let Some((parent, name)) = open_parent(root, rel)? else {
        return Err(changed(rel, "its directory disappeared"));
    };
    let mut staged = match new {
        Some((bytes, mode)) => Some(stage(&parent, &name, bytes, mode).map_err(io)?),
        None => None,
    };
    hook(rel).map_err(io)?;
    let aside = match want {
        Want::Absent => None,
        Want::File { id, digest } => {
            let aside = match move_aside(&parent, &name) {
                Ok(aside) => aside,
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    return Err(changed(rel, "disappeared after it was checked"));
                }
                Err(e) => return Err(io(e)),
            };
            if let Err(e) = check_aside(&parent, &aside, rel, id, digest) {
                return Err(put_back(&parent, &aside, &name, rel).err().unwrap_or(e));
            }
            Some(aside)
        }
    };
    if let Some(staged) = staged.as_mut() {
        match rename_noreplace(&parent, &staged.name, &name) {
            Ok(()) => staged.armed = false,
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                let kept = aside.as_ref().map_or(String::new(), |a| {
                    format!("; the checked file is kept as {}", shown(Path::new(a)))
                });
                return Err(changed(
                    rel,
                    &format!("appeared after it was checked{kept}"),
                ));
            }
            Err(e) => {
                if let Some(aside) = &aside {
                    put_back(&parent, aside, &name, rel)?;
                }
                return Err(io(e));
            }
        }
    } else if aside.is_none() && parent.symlink_metadata(&name).is_ok() {
        return Err(changed(rel, "appeared after it was checked"));
    }
    drop(staged);
    Ok((parent, aside))
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
}

/// The name of the backup copy of entry `index`.
fn copy_name(index: usize) -> OsString {
    OsString::from(index.to_string())
}

/// The trusted state of one migration.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Record {
    instance: String,
    root: Id,
    backup: Option<Id>,
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
        let head = format!(
            "{RECORD_HEADER}\ninstance {}\nroot {}\nbackup {}\nmanifest {}\n",
            self.instance,
            opt_id(Some(self.root)),
            opt_id(self.backup),
            opt_hex(self.manifest.as_ref()),
        );
        let entries = self.entries.iter().map(|e| {
            format!(
                "entry {} {} {} {:o} {}\n",
                u8::from(e.done),
                opt_hex(e.before.as_ref()),
                opt_hex(e.after.as_ref()),
                e.mode,
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
        let backup = match field("backup")? {
            "-" => None,
            text => Some(parse_id(text)?),
        };
        let manifest = parse_digest(field("manifest")?).ok()?;
        let mut entries = Vec::new();
        let mut paths = BTreeSet::new();
        for line in lines {
            let mut parts = line.strip_prefix("entry ")?.splitn(5, ' ');
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

    fn load(&self) -> Result<Option<Record>, Error> {
        let Some(seen) = look(&self.dir, &self.name)? else {
            return Ok(None);
        };
        let text = String::from_utf8(seen.bytes).map_err(|_| unverified())?;
        Record::parse(&text).map(Some).ok_or_else(unverified)
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

/// A backup directory this run created, and the names it put in it.
struct Owned {
    dir: Dir,
    id: Id,
    names: Vec<OsString>,
}

/// Remove what this run put in its backup directory, then the directory. Used when apply
/// fails before it changed any target.
fn discard(root: &Dir, owned: &Owned) {
    // Best effort throughout: the error that led here is the one to report, and the record
    // goes next, so nothing trusts what is left.
    for name in &owned.names {
        let _ = owned.dir.remove_file(name);
    }
    if root
        .symlink_metadata(BACKUP_DIR)
        .is_ok_and(|m| id_of(&m) == owned.id)
    {
        let _ = root.remove_dir(BACKUP_DIR);
    }
}

/// Create the backup directory and write the copies and the manifest into it. On failure,
/// whatever it created is gone again.
fn make_backup(root: &Dir, record: &Record, plan: &Plan) -> Result<Owned, Error> {
    let shown_dir = Path::new(BACKUP_DIR);
    let mut builder = DirBuilder::new();
    builder.mode(BACKUP_DIR_MODE);
    match root.create_dir_with(BACKUP_DIR, &builder) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists => return Err(exists()),
        Err(e) => return Err(Error::io(shown_dir, e)),
    }
    let opened = root
        .open_dir_nofollow(BACKUP_DIR)
        .and_then(|dir| Ok((id_of(&dir.dir_metadata()?), dir)));
    let (id, dir) = match opened {
        Ok(pair) => pair,
        Err(e) => {
            let _ = root.remove_dir(BACKUP_DIR);
            return Err(Error::io(shown_dir, e));
        }
    };
    let mut owned = Owned {
        dir,
        id,
        names: Vec::new(),
    };
    let manifest = record.manifest_text();
    let files = plan
        .changes
        .iter()
        .enumerate()
        .filter_map(|(i, c)| Some((copy_name(i), c.old.as_deref()?.as_bytes())))
        .chain([(OsString::from(MANIFEST), manifest.as_bytes())]);
    for (name, bytes) in files {
        let written = create_file(&owned.dir, &name, bytes);
        if written.is_ok() {
            owned.names.push(name.clone());
        }
        if let Err(e) = written.and_then(|_| sync_dir(&owned.dir)) {
            discard(root, &owned);
            return Err(Error::io(&shown_dir.join(&name), e));
        }
    }
    Ok(owned)
}

/// Check the plan's paths and what each target holds now, before anything is written.
fn preflight(dir: &Dir, plan: &Plan) -> Result<(Vec<Want>, Vec<Entry>), Error> {
    let mut paths = BTreeSet::new();
    for c in &plan.changes {
        let key: PathBuf = plain(&c.path)?.into_iter().collect();
        if !paths.insert(key) {
            return Err(refused(&c.path, "the plan names this file twice"));
        }
    }
    if dir.symlink_metadata(BACKUP_DIR).is_ok() {
        return Err(exists());
    }
    let mut wants = Vec::with_capacity(plan.changes.len());
    let mut entries = Vec::with_capacity(plan.changes.len());
    for c in &plan.changes {
        let seen = look(dir, &c.path)?;
        let planned = c.old.as_deref().map(|t| sha(t.as_bytes()));
        if seen.as_ref().map(|s| s.digest) != planned {
            return Err(changed(&c.path, "changed after the migration was planned"));
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
        });
    }
    Ok((wants, entries))
}

/// Write the plan to disk. Replaced and deleted files are copied to [`BACKUP_DIR`] first,
/// and the run is recorded in `state`, the per-user cache directory, which [`restore`]
/// trusts. A second write is refused while a backup exists, so the first backup is never
/// overwritten.
///
/// Every target must still hold what the plan read, with one link and no symbolic link on
/// the way; otherwise nothing is written. When a change fails before any target changed,
/// the backup and the record are removed again.
pub fn apply(root: &Path, plan: &Plan, state: &Path) -> Result<(), Error> {
    apply_hooked(root, plan, state, &mut |_| Ok(()))
}

/// [`apply`] with a callback that runs before the final check of each change, with the
/// project-relative path. An error from it fails that change. Tests use it to swap a target
/// at the worst moment.
#[doc(hidden)]
pub fn apply_hooked(
    root: &Path,
    plan: &Plan,
    state: &Path,
    hook: &mut dyn FnMut(&Path) -> std::io::Result<()>,
) -> Result<(), Error> {
    let (dir, root_id) = open_root(root)?;
    let (wants, entries) = preflight(&dir, plan)?;
    let store = Store::create(root, state)?;
    let mut record = Record {
        instance: nonce()?,
        root: root_id,
        backup: None,
        manifest: None,
        entries,
    };
    store.save(&record)?;
    let owned = match make_backup(&dir, &record, plan) {
        Ok(owned) => owned,
        Err(e) => {
            let _ = store.remove();
            return Err(e);
        }
    };
    record.backup = Some(owned.id);
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
        if let Err(failed) = mutate(&dir, &c.path, want, new, hook) {
            if failed.committed {
                // The target changed, so restore must undo it. Best effort: restore also
                // undoes a replaced file whose completion is missing.
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
    /// Files left as they are: created by the migration and changed since (kept with
    /// `--force`), or not yet changed when an apply stopped, and different now.
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
    backup: Dir,
    backup_id: Id,
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
    let (Some(backup_id), Some(manifest)) = (record.backup, record.manifest) else {
        return Err(unverified());
    };
    let backup = dir
        .open_dir_nofollow(BACKUP_DIR)
        .map_err(|_| unverified())?;
    let io = |e| Error::io(Path::new(BACKUP_DIR), e);
    if id_of(&backup.dir_metadata().map_err(io)?) != backup_id {
        return Err(unverified());
    }
    let expected = record.backup_names();
    let mut found = BTreeSet::new();
    for entry in backup.entries().map_err(io)? {
        found.insert(entry.map_err(io)?.file_name());
    }
    if found != expected {
        return Err(Error::Unsafe(format!(
            "{BACKUP_DIR} holds files that `slopvac migrate --write` did not write; \
             nothing restored"
        )));
    }
    let read = |name: &OsStr| {
        read_entry(&backup, name, &Path::new(BACKUP_DIR).join(name))?.ok_or_else(unverified)
    };
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
            return Err(Error::Unsafe(format!(
                "{}: the backup copy does not match the record; nothing restored",
                shown(Path::new(&e.path))
            )));
        }
        files.push((name, copy.id));
        copies.push(Some(copy.bytes));
    }
    let verified = Verified {
        record,
        backup,
        backup_id,
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
        let step = if digest == e.before {
            Step::Done
        } else if !e.done {
            // The change may have gone through without its completion being saved. A file
            // with an original and exactly the new bytes goes back; a created file stays,
            // because something else may have put it there.
            if e.before.is_some() && digest == e.after {
                Step::Undo(Want::of(now.as_ref(), true))
            } else {
                Step::Keep
            }
        } else if digest == e.after {
            Step::Undo(Want::of(now.as_ref(), true))
        } else if !force {
            return Err(changed(
                rel,
                "changed after the migration wrote it; rerun with --force to restore anyway",
            ));
        } else if e.before.is_none() {
            Step::Keep
        } else {
            Step::Undo(Want::of(now.as_ref(), false))
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
/// - The backup directory is the one that run created, also by device and inode, and holds
///   exactly the files that run wrote, each with its recorded digest.
/// - Each target still holds the bytes the migration wrote, or already its original
///   bytes. With `force`, a changed target is put back anyway, and a changed file that the
///   migration created is kept. `force` never skips the other checks or the link checks.
///
/// Changes the record marks as done are undone, and so is a replaced or deleted file that
/// holds exactly what the migration left there when its completion was not saved. A file is
/// deleted only if the record says the migration created it and it still holds exactly the
/// bytes the migration wrote.
pub fn restore(root: &Path, state: &Path, force: bool) -> Result<Restored, Error> {
    restore_hooked(root, state, force, &mut |_| Ok(()))
}

/// [`restore`] with a callback that runs before the final check of each change, as for
/// [`apply_hooked`].
#[doc(hidden)]
pub fn restore_hooked(
    root: &Path,
    state: &Path,
    force: bool,
    hook: &mut dyn FnMut(&Path) -> std::io::Result<()>,
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
                mutate(&dir, rel, want, new, hook).map_err(|f| f.error)?;
                out.restored += 1;
            }
        }
    }
    clean_up(&dir, &verified)?;
    store.remove()?;
    Ok(out)
}

/// Remove the checked backup files by name through the backup directory's handle, then the
/// directory itself. A file that is no longer the one that was checked stays, and so does
/// the directory.
fn clean_up(dir: &Dir, verified: &Verified) -> Result<(), Error> {
    let shown_dir = Path::new(BACKUP_DIR);
    for (name, id) in &verified.files {
        let rel = shown_dir.join(name);
        let same = verified
            .backup
            .symlink_metadata(name)
            .is_ok_and(|m| m.is_file() && id_of(&m) == *id);
        if !same {
            return Err(changed(
                &rel,
                "changed during the restore; delete the backup by hand",
            ));
        }
        verified
            .backup
            .remove_file(name)
            .map_err(|e| Error::io(&rel, e))?;
    }
    let same = dir
        .symlink_metadata(BACKUP_DIR)
        .is_ok_and(|m| m.is_dir() && id_of(&m) == verified.backup_id);
    if !same {
        return Err(changed(
            shown_dir,
            "changed during the restore; delete it by hand",
        ));
    }
    dir.remove_dir(BACKUP_DIR).map_err(|e| {
        if e.kind() == ErrorKind::DirectoryNotEmpty {
            changed(
                shown_dir,
                "gained files during the restore; delete it by hand",
            )
        } else {
            Error::io(shown_dir, e)
        }
    })
}
