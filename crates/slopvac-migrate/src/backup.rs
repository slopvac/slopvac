//! Applying a plan and undoing it, safe against a hostile checkout.
//!
//! The author of a pull request controls every file in the tree, the backup directory
//! included, so neither step trusts what it finds on disk:
//!
//! - Every write goes through [`slopvac_fs::write_atomic`], which renames a new file over
//!   the name. An external file that shares the inode keeps its bytes, and a symlink is never
//!   followed. A target that is a symlink, that sits below a symlinked directory or that has
//!   more than one link is refused.
//! - Right before each write or delete, the target is read again through a handle that
//!   follows no link, and it must still hold the bytes the step expects.
//! - The manifest records the SHA-256 of every file before and after the migration. Its own
//!   digest goes to a record in the per-user cache directory, outside the project, so a
//!   manifest that a checkout ships or edits does not match and is refused. A random nonce in
//!   the manifest keeps a checkout from rebuilding a recorded manifest out of known hashes.

use std::ffi::OsStr;
use std::fs;
use std::io::{ErrorKind, Read};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, MetadataExt, OpenOptions};
use sha2::{Digest, Sha256};
use slopvac_fs::{WriteOptions, write_atomic};

use crate::Error;
use crate::plan::{BACKUP_DIR, ChangeKind, Plan};

const MANIFEST: &str = "manifest.txt";
const HEADER: &str = "slopvac-migrate-manifest 1";
/// The directory below the per-user cache that holds one record per project.
const RECORD_DIR: &str = "migrate-backups";
/// Permission bits for a file that did not exist before.
const NEW_FILE_MODE: u32 = 0o644;

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

/// One file the migration touched: its digest before and after, `None` where it did not
/// exist.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    path: PathBuf,
    before: Option<Sha>,
    after: Option<Sha>,
}

fn digest_field(d: Option<&Sha>) -> String {
    d.map_or_else(|| "-".to_owned(), hex)
}

/// 16 random bytes from the kernel, as hex.
fn nonce() -> Result<String, Error> {
    const SOURCE: &str = "/dev/urandom";
    let mut bytes = [0u8; 16];
    fs::File::open(SOURCE)
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| Error::io(Path::new(SOURCE), e))?;
    Ok(format!("{:032x}", u128::from_be_bytes(bytes)))
}

fn render(nonce: &str, entries: &[Entry]) -> String {
    let lines = entries.iter().map(|e| {
        format!(
            "{} {} {}\n",
            digest_field(e.before.as_ref()),
            digest_field(e.after.as_ref()),
            e.path.display()
        )
    });
    std::iter::once(format!("{HEADER} {nonce}\n"))
        .chain(lines)
        .collect()
}

fn parse(text: &str) -> Result<Vec<Entry>, Error> {
    let bad = |line: &str| Error::Unsafe(format!("malformed manifest line `{line}`"));
    let mut lines = text.lines();
    let known = lines
        .next()
        .and_then(|l| l.strip_prefix(HEADER))
        .and_then(|rest| rest.strip_prefix(' '))
        .is_some_and(|n| n.len() == 32 && n.bytes().all(|b| b.is_ascii_hexdigit()));
    if !known {
        return Err(Error::Unsafe(format!(
            "{BACKUP_DIR}/{MANIFEST} has an unknown format"
        )));
    }
    let field = |f: &str| -> Option<Option<Sha>> {
        if f == "-" {
            Some(None)
        } else {
            unhex(f).map(Some)
        }
    };
    lines
        .map(|line| {
            let mut parts = line.splitn(3, ' ');
            let (Some(b), Some(a), Some(name)) = (parts.next(), parts.next(), parts.next()) else {
                return Err(bad(line));
            };
            let (Some(before), Some(after)) = (field(b), field(a)) else {
                return Err(bad(line));
            };
            if before.is_none() && after.is_none() {
                return Err(bad(line));
            }
            Ok(Entry {
                path: PathBuf::from(name),
                before,
                after,
            })
        })
        .collect()
}

/// Reject a project-relative path that is not a plain descent: absolute paths, `..`,
/// newlines and an empty path. Symlinks are refused when the path is opened.
fn ensure_plain(name: &Path) -> Result<(), Error> {
    let mut any = false;
    for comp in name.components() {
        let Component::Normal(part) = comp else {
            return Err(Error::Unsafe(format!(
                "{}: path leaves the project",
                name.display()
            )));
        };
        if part.as_bytes().contains(&b'\n') {
            return Err(Error::Unsafe(format!(
                "{}: newline in path",
                name.display()
            )));
        }
        any = true;
    }
    if any {
        Ok(())
    } else {
        Err(Error::Unsafe("empty path".into()))
    }
}

/// What a name below a directory holds now.
enum Found {
    Absent,
    File(Vec<u8>),
}

impl Found {
    fn digest(&self) -> Option<Sha> {
        match self {
            Self::Absent => None,
            Self::File(bytes) => Some(sha(bytes)),
        }
    }
}

fn open_dir(path: &Path) -> Result<Dir, Error> {
    Dir::open_ambient_dir(path, ambient_authority()).map_err(|e| Error::io(path, e))
}

/// Read `rel` below `dir` without following a link at any step. A symlink, a non-regular
/// file and a file with more than one link are refused; a missing file or directory is
/// [`Found::Absent`]. `shown` is the full path, for errors.
// TODO(8l4.12.37): replace with the compare-and-swap check in slopvac-fs when it lands.
fn inspect(dir: &Dir, rel: &Path, shown: &Path) -> Result<Found, Error> {
    let refuse = |why: &str| Error::Unsafe(format!("{}: {why}", rel.display()));
    let io = |e: std::io::Error| Error::io(shown, e);
    let mut ancestors: Vec<&OsStr> = rel.iter().collect();
    let Some(name) = ancestors.pop() else {
        return Err(refuse("path names no file"));
    };
    let mut parent = dir.try_clone().map_err(io)?;
    for part in ancestors {
        parent = match parent.open_dir_nofollow(part) {
            Ok(next) => next,
            Err(e) => {
                return match parent.symlink_metadata(part) {
                    Err(m) if m.kind() == ErrorKind::NotFound => Ok(Found::Absent),
                    Ok(m) if m.is_symlink() => Err(refuse("a path component is a symbolic link")),
                    Ok(m) if !m.is_dir() => Err(refuse("a path component is not a directory")),
                    _ => Err(io(e)),
                };
            }
        };
    }
    let entry = match parent.symlink_metadata(name) {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Found::Absent),
        Err(e) => return Err(io(e)),
    };
    if entry.is_symlink() {
        return Err(refuse("is a symbolic link"));
    }
    if !entry.is_file() {
        return Err(refuse("is not a regular file"));
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    nonblocking(&mut options).map_err(io)?;
    let file = parent.open_with(name, &options).map_err(io)?;
    let held = file.metadata().map_err(io)?;
    if !held.is_file() || (held.dev(), held.ino()) != (entry.dev(), entry.ino()) {
        return Err(refuse("changed while it was read"));
    }
    if held.nlink() > 1 {
        return Err(refuse("has more than one hard link"));
    }
    let bytes =
        slopvac_fs::read_capped(file, slopvac_engine::MAX_FILE_BYTES).map_err(|e| match e {
            slopvac_fs::Error::CapExceeded { .. } => refuse("is too large to migrate"),
            slopvac_fs::Error::Read(source) => io(source),
            other => io(std::io::Error::other(other.to_string())),
        })?;
    Ok(Found::File(bytes))
}

/// Open without blocking, so a FIFO swapped in after the check cannot hang the run.
fn nonblocking(options: &mut OpenOptions) -> std::io::Result<()> {
    use cap_std::fs::OpenOptionsExt;
    let flag = i32::try_from(rustix::fs::OFlags::NONBLOCK.bits())
        .map_err(|_| std::io::Error::other("O_NONBLOCK does not fit"))?;
    options.custom_flags(flag);
    Ok(())
}

fn fs_error(root: &Path, e: slopvac_fs::Error) -> Error {
    match e {
        slopvac_fs::Error::UnsafeTarget { reason, path } => {
            Error::Unsafe(format!("{}: {reason}", path.display()))
        }
        slopvac_fs::Error::Io { path, source, .. } => Error::io(&root.join(path), source),
        other => Error::io(root, std::io::Error::other(other.to_string())),
    }
}

/// What a target must hold right before a step changes it.
#[derive(Debug, Clone, Copy)]
enum Expect {
    /// Anything, as long as it is a plain file or absent.
    Any,
    /// This digest, or absent for `None`.
    Digest(Option<Sha>),
}

/// The project's files, opened once through a directory handle.
struct Tree<'a> {
    root: &'a Path,
    dir: Dir,
}

impl Tree<'_> {
    fn inspect(&self, rel: &Path) -> Result<Found, Error> {
        inspect(&self.dir, rel, &self.root.join(rel))
    }

    /// Replace `rel` with `bytes`, or remove it for `None`, after checking once more that
    /// it holds what `expected` says. [`Expect::Any`] skips the content check (`--force`),
    /// but never the link checks.
    fn replace(&self, rel: &Path, expected: Expect, bytes: Option<&[u8]>) -> Result<(), Error> {
        // TODO(8l4.12.37): a compare-and-swap in slopvac-fs would close the gap between
        // this check and the rename.
        let now = self.inspect(rel)?.digest();
        if let Expect::Digest(want) = expected
            && now != want
        {
            return Err(changed(rel, "changed after it was checked"));
        }
        match bytes {
            Some(bytes) => write_atomic(
                &self.dir,
                rel,
                bytes,
                WriteOptions {
                    new_file_mode: Some(NEW_FILE_MODE),
                    create_parents: true,
                },
            )
            .map_err(|e| fs_error(self.root, e)),
            None if now.is_none() => Ok(()),
            None => self.remove(rel),
        }
    }

    fn remove(&self, rel: &Path) -> Result<(), Error> {
        let shown = self.root.join(rel);
        let io = |e| Error::io(&shown, e);
        let mut parent = self.dir.try_clone().map_err(io)?;
        let mut names: Vec<&OsStr> = rel.iter().collect();
        let name = names
            .pop()
            .ok_or_else(|| Error::Unsafe("empty path".into()))?;
        for part in names {
            parent = parent.open_dir_nofollow(part).map_err(io)?;
        }
        parent.remove_file(name).map_err(io)
    }
}

fn changed(rel: &Path, why: &str) -> Error {
    Error::Changed(format!("{}: {why}", rel.display()))
}

/// Where the record for `root` lives below the per-user directory `state`: the digest of
/// the project's canonical path, so two projects never share a record.
fn record_name(root: &Path) -> Result<PathBuf, Error> {
    let canonical = fs::canonicalize(root).map_err(|e| Error::io(root, e))?;
    Ok(Path::new(RECORD_DIR).join(hex(&sha(canonical.as_os_str().as_bytes()))))
}

/// Write the plan to disk. Replaced and deleted files are copied to [`BACKUP_DIR`]
/// first, with a manifest that [`restore`] reads, and the manifest's digest is recorded in
/// `state`, the per-user cache directory. A second write is refused while a backup exists,
/// so the first backup is never overwritten.
///
/// Every target must still hold what the plan read, with one link and no symbolic link on
/// the way; otherwise nothing is written.
pub fn apply(root: &Path, plan: &Plan, state: &Path) -> Result<(), Error> {
    let tree = Tree {
        root,
        dir: open_dir(root)?,
    };
    let backup_exists = || {
        Error::Unsafe(format!(
            "{BACKUP_DIR} exists; run `slopvac migrate --restore` or delete it first"
        ))
    };
    if tree.dir.symlink_metadata(BACKUP_DIR).is_ok() {
        return Err(backup_exists());
    }
    let mut entries = Vec::new();
    for c in &plan.changes {
        ensure_plain(&c.path)?;
        let planned = c.old.as_deref().map(str::as_bytes);
        if tree.inspect(&c.path)?.digest() != planned.map(sha) {
            return Err(changed(&c.path, "changed after the migration was planned"));
        }
        let after = match c.kind {
            ChangeKind::Write => c.new.as_deref().map(|t| sha(t.as_bytes())),
            ChangeKind::Delete => None,
        };
        entries.push(Entry {
            path: c.path.clone(),
            before: planned.map(sha),
            after,
        });
    }
    match tree.dir.create_dir(BACKUP_DIR) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists => return Err(backup_exists()),
        Err(e) => return Err(Error::io(&root.join(BACKUP_DIR), e)),
    }
    let backup = Path::new(BACKUP_DIR);
    for c in &plan.changes {
        if let Some(old) = &c.old {
            tree.replace(
                &backup.join(&c.path),
                Expect::Digest(None),
                Some(old.as_bytes()),
            )?;
        }
    }
    let manifest = render(&nonce()?, &entries);
    tree.replace(
        &backup.join(MANIFEST),
        Expect::Digest(None),
        Some(manifest.as_bytes()),
    )?;
    write_record(root, state, &sha(manifest.as_bytes()))?;
    for (c, e) in plan.changes.iter().zip(&entries) {
        let new = match c.kind {
            ChangeKind::Write => c.new.as_deref().map(str::as_bytes),
            ChangeKind::Delete => None,
        };
        tree.replace(&c.path, Expect::Digest(e.before), new)?;
    }
    Ok(())
}

fn write_record(root: &Path, state: &Path, digest: &Sha) -> Result<(), Error> {
    fs::create_dir_all(state).map_err(|e| Error::io(state, e))?;
    let dir = open_dir(state)?;
    write_atomic(
        &dir,
        &record_name(root)?,
        hex(digest).as_bytes(),
        WriteOptions {
            new_file_mode: None,
            create_parents: true,
        },
    )
    .map_err(|e| fs_error(state, e))
}

/// The outcome of [`restore`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Restored {
    /// Files now back in their state before the migration.
    pub restored: usize,
    /// Files the migration created and that changed since, so `--force` kept them.
    pub kept: Vec<PathBuf>,
}

/// What restoring one entry does.
enum Step {
    /// Already as before the migration.
    Done,
    /// Put the backup back, or delete a created file.
    Undo(Expect),
    /// A created file that changed: keep it.
    Keep,
}

/// Undo the last [`apply`]: put backed-up files back and remove files it created.
///
/// Nothing changes unless every check passes first:
///
/// - The manifest's digest matches the record that [`apply`] wrote to `state`, so a backup
///   this tool did not create here is refused.
/// - Each backup copy hashes to the digest the manifest recorded.
/// - Each target still holds the bytes the migration wrote, or already its original
///   bytes. With `force`, a changed target is put back anyway, and a changed file that the
///   migration created is kept. `force` never skips the first two checks or the link checks.
pub fn restore(root: &Path, state: &Path, force: bool) -> Result<Restored, Error> {
    let tree = Tree {
        root,
        dir: open_dir(root)?,
    };
    let backup = Path::new(BACKUP_DIR);
    let unverified = || {
        Error::Unsafe(format!(
            "{BACKUP_DIR} was not written by `slopvac migrate --write` for this project as \
             this user, or it was changed; nothing restored"
        ))
    };
    let Found::File(manifest) = tree.inspect(&backup.join(MANIFEST))? else {
        return Err(Error::Unsafe(format!("{BACKUP_DIR}/{MANIFEST} is missing")));
    };
    let Ok(state_dir) = Dir::open_ambient_dir(state, ambient_authority()) else {
        return Err(unverified());
    };
    let Found::File(record) = inspect(&state_dir, &record_name(root)?, state)? else {
        return Err(unverified());
    };
    if record != hex(&sha(&manifest)).as_bytes() {
        return Err(unverified());
    }
    let text = String::from_utf8(manifest).map_err(|_| unverified())?;
    let entries = parse(&text)?;
    let mut plan = Vec::with_capacity(entries.len());
    let mut originals = Vec::with_capacity(entries.len());
    for e in &entries {
        ensure_plain(&e.path)?;
        let original = match e.before {
            Some(want) => match tree.inspect(&backup.join(&e.path))? {
                Found::File(bytes) if sha(&bytes) == want => Some(bytes),
                _ => {
                    return Err(Error::Unsafe(format!(
                        "{}: the backup copy does not match the manifest",
                        e.path.display()
                    )));
                }
            },
            None => None,
        };
        let now = tree.inspect(&e.path)?.digest();
        let step = if now == e.before {
            Step::Done
        } else if now == e.after {
            Step::Undo(Expect::Digest(e.after))
        } else if !force {
            return Err(changed(
                &e.path,
                "changed after the migration wrote it; rerun with --force to restore anyway",
            ));
        } else if e.before.is_none() {
            Step::Keep
        } else {
            Step::Undo(Expect::Any)
        };
        plan.push(step);
        originals.push(original);
    }
    let mut out = Restored::default();
    for ((e, step), original) in entries.iter().zip(plan).zip(originals) {
        match step {
            Step::Done => out.restored += 1,
            Step::Keep => out.kept.push(e.path.clone()),
            Step::Undo(expected) => {
                tree.replace(&e.path, expected, original.as_deref())?;
                out.restored += 1;
            }
        }
    }
    let full = root.join(BACKUP_DIR);
    fs::remove_dir_all(&full).map_err(|e| Error::io(&full, e))?;
    let record = state.join(record_name(root)?);
    fs::remove_file(&record).map_err(|e| Error::io(&record, e))?;
    Ok(out)
}
