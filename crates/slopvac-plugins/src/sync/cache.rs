use std::fs::{self, File};
use std::io::{self, Read, Seek as _, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;

use super::error::{CacheCause, SyncError};
use super::verified::cause_of;
use crate::extract::{
    Evicted, ExtractError, TreeHash, ensure_private_child, ensure_private_dir, evict, open_regular,
    verify_tree_capturing,
};
use crate::lock::{Digest as LockDigest, PackId};
use crate::messages::sync as messages;
use crate::messages::transport::TransportText;
use crate::oci::Digest;

/// A tree that a hashing pass accepted: its hash, and the manifest at its root as that same pass
/// read it. What is later derived from the tree comes from these, never from a second read.
#[derive(Debug, Clone)]
pub(super) struct Accepted {
    pub(super) hash: TreeHash,
    pub(super) manifest: Option<Vec<u8>>,
}

/// What the store holds for one locked digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeState {
    /// Nothing recorded, or the recorded tree is gone.
    Absent,
    /// The recorded tree exists and hashes to its record. This is "intact against its record",
    /// and the record is only a hint: it does not mean the tree is the one the lock pins.
    /// Trust and loading use [`Store::verified_tree`] instead.
    Intact(PathBuf),
    /// The recorded tree exists and differs from its record.
    Tampered(PathBuf),
}

/// A verified private copy of a cached archive, in the staging area of the store. Only the store
/// builds one, from bytes it hashed while it wrote them. Dropping it removes the file, so a copy
/// never outlives the sync that made it, on success or on error.
#[derive(Debug)]
pub(super) struct StagedArchive(NamedTempFile);

impl StagedArchive {
    /// The copy, positioned at its first byte, for extraction. It is read from its own handle,
    /// and no path is opened.
    pub(super) fn reader(&self) -> &File {
        self.0.as_file()
    }
}

/// What copying a cached archive found.
enum Copied {
    Staged(StagedArchive),
    Absent,
    Wrong(Verdict),
}

/// Why a cached archive is not the archive that the lock names.
enum Verdict {
    /// It is over the size cap.
    Over,
    /// It hashes to another digest.
    Digest(Digest),
}

/// The exclusive store lock. Dropping it releases the lock.
#[derive(Debug)]
pub struct StoreLock {
    _file: File,
}

/// The per-user data directory of packs: extracted trees, the downloaded
/// artifacts they came from and the records that link a locked digest to a tree.
///
/// ```text
/// <data>/trees/tree-<hex>   extracted, read-only, named by the hash of the tree
/// <data>/blobs/<hex>        downloaded content, named by its sha256
/// <data>/refs/<hex>         lock digest -> tree hash (a hint, never an authority)
/// ```
///
/// The CLI picks the location; it must be private to the user (a per-user
/// directory such as `$XDG_DATA_HOME/slopvac`, never a shared or temporary
/// directory), as `SLOPVAC_CACHE_DIR` content is untrusted (threat model E8).
/// Everything read back is checked again: a blob against its name, a tree
/// against its record, and sync re-derives the tree from verified blobs instead
/// of trusting a record.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
    #[cfg(feature = "test-support")]
    observer: Option<ReadObserver>,
    #[cfg(feature = "test-support")]
    archive_bytes: Option<std::sync::Arc<std::sync::atomic::AtomicU64>>,
    /// Called each time the store lock is asked for, before it can block.
    #[cfg(feature = "test-support")]
    lock_hook: Option<LockHook>,
    /// An OS error code that every write to a staged copy fails with.
    #[cfg(feature = "test-support")]
    staging_fault: Option<i32>,
    /// Plugins whose cached files sync removed and fetched again in this run.
    repaired: std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<PackId>>>,
}

/// What a test hangs on the lock of the store.
#[cfg(feature = "test-support")]
#[derive(Clone)]
struct LockHook(std::sync::Arc<dyn Fn() + Send + Sync>);

#[cfg(feature = "test-support")]
impl std::fmt::Debug for LockHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LockHook")
    }
}

/// What a test hangs on the read boundary of the store.
#[cfg(feature = "test-support")]
#[derive(Clone)]
struct ReadObserver(std::sync::Arc<dyn Fn(&Path) + Send + Sync>);

#[cfg(feature = "test-support")]
impl std::fmt::Debug for ReadObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReadObserver")
    }
}

impl Store {
    /// A store at `root`. Nothing is created until the first write.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            #[cfg(feature = "test-support")]
            observer: None,
            #[cfg(feature = "test-support")]
            archive_bytes: None,
            #[cfg(feature = "test-support")]
            lock_hook: None,
            #[cfg(feature = "test-support")]
            staging_fault: None,
            repaired: std::sync::Arc::default(),
        }
    }

    /// Call `observer` with the path of each cached file right after the store has checked its
    /// bytes against its name. A test uses it to change the file at the one moment that a
    /// second read of the same path would see other bytes.
    #[doc(hidden)]
    #[must_use]
    #[cfg(feature = "test-support")]
    pub fn with_read_observer(
        mut self,
        observer: std::sync::Arc<dyn Fn(&Path) + Send + Sync>,
    ) -> Self {
        self.observer = Some(ReadObserver(observer));
        self
    }

    /// Add the bytes that each hash of a cached archive consumes, drain included, to `counter`.
    #[doc(hidden)]
    #[must_use]
    #[cfg(feature = "test-support")]
    pub fn with_archive_byte_counter(
        mut self,
        counter: std::sync::Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        self.archive_bytes = Some(counter);
        self
    }

    /// Call `hook` each time the store lock is asked for, before the call can block.
    #[doc(hidden)]
    #[must_use]
    #[cfg(feature = "test-support")]
    pub fn with_lock_hook(mut self, hook: std::sync::Arc<dyn Fn() + Send + Sync>) -> Self {
        self.lock_hook = Some(LockHook(hook));
        self
    }

    /// Make every write to a staged copy fail with the OS error `code`.
    #[doc(hidden)]
    #[must_use]
    #[cfg(feature = "test-support")]
    pub fn with_staging_write_fault(mut self, code: i32) -> Self {
        self.staging_fault = Some(code);
        self
    }

    /// Write `chunk` to the staged copy, or fail as a test asked.
    fn write_staged(&self, staged: &mut NamedTempFile, chunk: &[u8]) -> io::Result<()> {
        #[cfg(feature = "test-support")]
        if let Some(code) = self.staging_fault {
            return Err(io::Error::from_raw_os_error(code));
        }
        staged.write_all(chunk)
    }

    /// Count `n` bytes of a cached archive, if a test set a counter.
    #[cfg(feature = "test-support")]
    pub(super) fn count_archive_bytes(&self, n: u64) {
        if let Some(counter) = &self.archive_bytes {
            counter.fetch_add(n, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Tell the observer, if a test set one, that `path` was just checked.
    #[cfg(feature = "test-support")]
    pub(super) fn observe(&self, path: &Path) {
        if let Some(observer) = &self.observer {
            (observer.0)(path);
        }
    }

    /// The data directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(super) fn trees(&self) -> PathBuf {
        self.root.join("trees")
    }

    fn blobs(&self) -> PathBuf {
        self.root.join("blobs")
    }

    /// Where the cached blob `digest` lives. The file is not read or checked.
    pub(super) fn blob_path(&self, digest: &Digest) -> PathBuf {
        self.blobs().join(digest.hex())
    }

    fn refs(&self) -> PathBuf {
        self.root.join("refs")
    }

    pub(super) fn prepare(&self) -> Result<(), SyncError> {
        let store = |e: ExtractError| SyncError::Store(e.unescaped());
        ensure_private_dir(&self.root).map_err(store)?;
        // Every child is checked as an owned, non-link directory before use: extraction
        // chmods and renames inside `trees`, so a planted link must not redirect it.
        for dir in [self.trees(), self.blobs(), self.refs()] {
            ensure_private_child(&dir).map_err(store)?;
        }
        ensure_private_child(&self.blobs().join(".tmp")).map_err(store)
    }

    /// Take the exclusive store lock, waiting until another process releases it.
    ///
    /// `sync` holds it from the first inspection to the last record, so two
    /// syncs cannot evict and restore the same tree under each other (threat
    /// model E9). The OS drops the lock if the holder dies. Read-only calls such
    /// as [`status`](super::status) do not take it.
    ///
    /// # Errors
    /// [`SyncError::Store`] when the data directory or lock file cannot be used.
    pub fn lock(&self) -> Result<StoreLock, SyncError> {
        #[cfg(feature = "test-support")]
        if let Some(hook) = &self.lock_hook {
            (hook.0)();
        }
        self.prepare()?;
        let path = self.root.join(".lock");
        if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
            return Err(SyncError::Store(messages::not_plain_file(&path)));
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| SyncError::Store(messages::store_lock_open(&e)))?;
        file.lock()
            .map_err(|e| SyncError::Store(messages::store_lock_take(&e)))?;
        Ok(StoreLock { _file: file })
    }

    /// The directory of a tree.
    #[must_use]
    pub fn tree_dir(&self, hash: &TreeHash) -> PathBuf {
        self.trees().join(format!("tree-{}", hash.hex()))
    }

    /// A private copy of the cached archive `digest`, verified while it was copied, or `None`
    /// when the archive is absent or sync removed it because it was over `max` or hashed to
    /// something else (the repair is recorded). Only sync calls it.
    ///
    /// The cached file is opened once, without following a link. The bytes are hashed as they are
    /// written to a new private file in the staging area, never more than `max` plus one, so the
    /// copy is exactly what was checked and nothing opens the cached file by path again.
    ///
    /// # Errors
    /// [`SyncError::CacheUnreadable`] when the file exists and cannot be read, is not a regular
    /// file, or cannot be removed. Nothing is removed in the first two cases.
    pub(super) fn blob(
        &self,
        id: &PackId,
        digest: &Digest,
        max: u64,
    ) -> Result<Option<StagedArchive>, SyncError> {
        match self.copy_verified(id, digest, max)? {
            Copied::Staged(staged) => Ok(Some(staged)),
            Copied::Absent => Ok(None),
            Copied::Wrong(_) => {
                self.evict_blob(id, digest)?;
                Ok(None)
            }
        }
    }

    /// A private copy of the archive `digest` that the store has just written. A cached file that
    /// is not what was written is an error: the run stops.
    ///
    /// # Errors
    /// [`SyncError::CacheUnreadable`] for a missing, unreadable or oversize file, and
    /// [`SyncError::DigestMismatch`] for other bytes.
    pub(super) fn stage_written(
        &self,
        id: &PackId,
        digest: &Digest,
        max: u64,
    ) -> Result<StagedArchive, SyncError> {
        match self.copy_verified(id, digest, max)? {
            Copied::Staged(staged) => Ok(staged),
            Copied::Absent => Err(self.unreadable(
                id,
                digest,
                CacheCause::Io {
                    kind: io::ErrorKind::NotFound,
                    os_code: None,
                },
            )),
            Copied::Wrong(Verdict::Over) => {
                Err(self.unreadable(id, digest, CacheCause::TooLarge { cap: max }))
            }
            Copied::Wrong(Verdict::Digest(actual)) => Err(SyncError::DigestMismatch {
                id: id.clone(),
                expected: digest.to_string(),
                actual: actual.to_string(),
            }),
        }
    }

    fn copy_verified(&self, id: &PackId, digest: &Digest, max: u64) -> Result<Copied, SyncError> {
        let path = self.blob_path(digest);
        let file = match open_regular(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Copied::Absent),
            Err(e) => return Err(self.unreadable(id, digest, cause_of(&e))),
        };
        let mut staged = self.blob_tempfile()?;
        let mut reader = file.take(max.saturating_add(1));
        let mut hasher = Sha256::new();
        let mut read = 0u64;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| self.unreadable(id, digest, CacheCause::io(&e)))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            self.write_staged(&mut staged, &buf[..n])
                .map_err(|e| self.unreadable(id, digest, CacheCause::io(&e)))?;
            read += n as u64;
        }
        if read > max {
            return Ok(Copied::Wrong(Verdict::Over));
        }
        let actual = Digest::from_hasher(hasher);
        if actual != *digest {
            return Ok(Copied::Wrong(Verdict::Digest(actual)));
        }
        staged
            .as_file_mut()
            .rewind()
            .map_err(|e| self.unreadable(id, digest, CacheCause::io(&e)))?;
        #[cfg(feature = "test-support")]
        self.observe(&path);
        Ok(Copied::Staged(StagedArchive(staged)))
    }

    /// Remove the cached blob `digest` that sync found to be wrong, and record the repair. Only
    /// sync calls it: a read-only lookup never deletes. A blob that is already gone is fine; any
    /// other failure to remove it stops the run.
    ///
    /// # Errors
    /// [`SyncError::CacheUnreadable`] when the file cannot be removed.
    pub(super) fn evict_blob(&self, id: &PackId, digest: &Digest) -> Result<(), SyncError> {
        match fs::remove_file(self.blob_path(digest)) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(self.unreadable(id, digest, CacheCause::io(&e))),
        }
        if let Ok(mut repaired) = self.repaired.lock() {
            repaired.insert(id.clone());
        }
        Ok(())
    }

    /// Forget the repairs recorded so far. Each sync starts from none.
    pub(super) fn clear_repaired(&self) {
        if let Ok(mut repaired) = self.repaired.lock() {
            repaired.clear();
        }
    }

    /// Whether sync repaired a cached file of `id` since the last call, and clear the record.
    pub(super) fn take_repaired(&self, id: &PackId) -> bool {
        self.repaired
            .lock()
            .is_ok_and(|mut repaired| repaired.remove(id))
    }

    /// The error of a cached blob that exists but cannot be read.
    pub(super) fn unreadable(&self, id: &PackId, digest: &Digest, cause: CacheCause) -> SyncError {
        Self::unreadable_at(id, &self.blob_path(digest), cause)
    }

    /// The error of a cached path that cannot be read. The path is redacted here, so no text of
    /// the error, `Debug` included, holds an absolute path.
    pub(super) fn unreadable_at(id: &PackId, path: &Path, cause: CacheCause) -> SyncError {
        SyncError::CacheUnreadable {
            id: id.clone(),
            path: PathBuf::from(slopvac_fs::display_under_root(path, Path::new("."))),
            cause,
        }
    }

    pub(super) fn blob_tempfile(&self) -> Result<NamedTempFile, SyncError> {
        NamedTempFile::new_in(self.blobs().join(".tmp"))
            .map_err(|e| SyncError::Store(messages::download_file(&e)))
    }

    /// A private scratch directory for work that must not outlive the sync
    /// (a git fetch). It sits beside the download files, in the checked store.
    pub(super) fn scratch_dir(&self) -> Result<tempfile::TempDir, SyncError> {
        tempfile::Builder::new()
            .prefix("work-")
            .tempdir_in(self.blobs().join(".tmp"))
            .map_err(|e| SyncError::Store(messages::scratch_dir(&e)))
    }

    /// Move a finished download into place, after hashing it once more.
    ///
    /// # Errors
    /// [`SyncError::Store`] when the content is not `digest` or the move fails.
    pub(super) fn commit_blob(
        &self,
        tmp: NamedTempFile,
        digest: &Digest,
    ) -> Result<PathBuf, SyncError> {
        let actual = hash_file(tmp.path()).map_err(|e| SyncError::Store(e.to_string()))?;
        if actual != *digest {
            return Err(SyncError::Store(messages::download_hash(&actual, digest)));
        }
        let path = self.blobs().join(digest.hex());
        tmp.persist(&path)
            .map_err(|e| SyncError::Store(messages::download_store(&e.error)))?;
        Ok(path)
    }

    pub(super) fn put_blob(&self, digest: &Digest, bytes: &[u8]) -> Result<PathBuf, SyncError> {
        let mut tmp = self.blob_tempfile()?;
        tmp.write_all(bytes)
            .map_err(|e| SyncError::Store(messages::download_write(&e)))?;
        self.commit_blob(tmp, digest)
    }

    pub(super) fn record(&self, locked: &LockDigest, tree: &TreeHash) -> Result<(), SyncError> {
        let path = self.refs().join(&locked.as_str()[7..]);
        let mut tmp = NamedTempFile::new_in(self.refs())
            .map_err(|e| SyncError::Store(messages::record_install(&e)))?;
        tmp.write_all(tree.hex().as_bytes())
            .map_err(|e| SyncError::Store(messages::record_install(&e)))?;
        tmp.persist(path)
            .map_err(|e| SyncError::Store(messages::record_install(&e.error)))?;
        Ok(())
    }

    /// The tree hash that a record names for a locked digest. `None` means the record is missing,
    /// unreadable, not a regular file or malformed, which this call does not tell apart.
    ///
    /// A record is a hint for sync and status, never an authority (threat model E8): whoever
    /// can write the store can write one. Nothing that grants trust or loads a plugin may take
    /// a hash from it; [`Store::verified_tree`] derives the hash from the lock.
    fn recorded(&self, locked: &LockDigest) -> Option<TreeHash> {
        let path = self.refs().join(&locked.as_str()[7..]);
        // The handle is checked, not the name: a ref swapped for a FIFO cannot hang the read.
        let mut text = String::new();
        crate::extract::open_regular(&path)
            .ok()?
            .take(65)
            .read_to_string(&mut text)
            .ok()?;
        if text.len() > 64 {
            return None;
        }
        TreeHash::from_hex(text.trim())
    }

    /// What the store holds for a locked digest, judged against its own record, which is a hint.
    /// Reads the tree again.
    ///
    /// # Errors
    /// [`SyncError::Store`] when the tree cannot be read.
    pub fn inspect(&self, locked: &LockDigest) -> Result<TreeState, SyncError> {
        match self.inspect_accepted(locked) {
            Ok((state, _)) => Ok(state),
            // Sync repairs a recorded tree that the walk refuses as it repairs one that changed.
            Err(ExtractError::Refused(_)) => {
                let dir = self
                    .recorded(locked)
                    .map(|hash| self.tree_dir(&hash))
                    .unwrap_or_default();
                Ok(TreeState::Tampered(dir))
            }
            Err(e) => Err(SyncError::Store(e.unescaped())),
        }
    }

    /// [`Self::inspect`], and for an intact tree the hash the pass accepted and the manifest as
    /// that same pass read it. A caller that acts on the tree uses these and never reads the
    /// manifest again.
    ///
    /// # Errors
    /// The typed cause of a pass that could not finish. A tree that differs from its record is
    /// [`TreeState::Tampered`], not an error.
    pub(super) fn inspect_accepted(
        &self,
        locked: &LockDigest,
    ) -> Result<(TreeState, Option<Accepted>), ExtractError> {
        let Some(hash) = self.recorded(locked) else {
            return Ok((TreeState::Absent, None));
        };
        let dir = self.tree_dir(&hash);
        match fs::symlink_metadata(&dir) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((TreeState::Absent, None)),
            // Only a tree that is not there is absent: a tree that cannot be inspected is an error.
            Err(source) => {
                return Err(ExtractError::Io {
                    what: TransportText::ExtractInspecting {
                        path: dir.display().to_string(),
                    }
                    .to_string(),
                    source,
                });
            }
        }
        let verified = verify_tree_capturing(&dir, &hash);
        #[cfg(feature = "test-support")]
        self.observe(&dir);
        match verified {
            Ok(manifest) => Ok((TreeState::Intact(dir), Some(Accepted { hash, manifest }))),
            Err(ExtractError::Tampered { .. }) => Ok((TreeState::Tampered(dir), None)),
            Err(e) => Err(e),
        }
    }
}

/// Remove a modified tree. Returns where it was moved when it was too large to delete.
pub(super) fn evict_tree(dir: &Path) -> Result<Option<PathBuf>, SyncError> {
    match evict(dir).map_err(|e| SyncError::Store(e.unescaped()))? {
        Evicted::Removed => Ok(None),
        Evicted::Quarantined(aside) => Ok(Some(aside)),
    }
}

/// `sha256` of a file, streamed.
pub(super) fn hash_file(path: &Path) -> io::Result<Digest> {
    let mut f = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(Digest::from_hasher(hasher))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
mod tests {
    use super::*;

    #[test]
    fn evicting_a_blob_that_cannot_be_removed_is_an_error_and_not_a_repair() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let digest = Digest::of(b"x");
        let id: PackId = "demo".parse().unwrap();
        // A directory with content cannot be unlinked as a file.
        let path = store.blob_path(&digest);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("inner"), b"y").unwrap();

        let err = store.evict_blob(&id, &digest).unwrap_err();

        assert!(
            matches!(
                err,
                SyncError::CacheUnreadable {
                    cause: CacheCause::Io { .. },
                    ..
                }
            ),
            "{err:?}"
        );
        assert!(!store.take_repaired(&id), "a failed removal is no repair");
    }

    #[test]
    fn evicting_a_blob_that_is_already_gone_is_a_repair() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let digest = Digest::of(b"x");
        let id: PackId = "demo".parse().unwrap();

        store.evict_blob(&id, &digest).unwrap();

        assert!(store.take_repaired(&id));
    }
}
