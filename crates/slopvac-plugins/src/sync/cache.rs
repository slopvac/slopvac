use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;

use super::error::SyncError;
use crate::extract::{
    Evicted, ExtractError, TreeHash, ensure_private_child, ensure_private_dir, evict, verify_tree_capturing,
};
use crate::lock::Digest as LockDigest;
use crate::messages::sync as messages;
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

    /// A cached blob whose content hashes to `digest`, or `None`. A file that
    /// is not a plain file, is too large or hashes differently is deleted.
    pub(super) fn blob(&self, digest: &Digest, max: u64) -> Option<PathBuf> {
        let path = self.blobs().join(digest.hex());
        let meta = fs::symlink_metadata(&path).ok()?;
        let ok = meta.is_file()
            && meta.len() <= max
            && hash_file(&path).is_ok_and(|actual| actual == *digest);
        if ok {
            #[cfg(feature = "test-support")]
            self.observe(&path);
            Some(path)
        } else {
            let _ = fs::remove_file(&path);
            None
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
        self.inspect_accepted(locked).map(|(state, _)| state)
    }

    /// [`Self::inspect`], and for an intact tree the hash the pass accepted and the manifest as
    /// that same pass read it. A caller that acts on the tree uses these and never reads the
    /// manifest again.
    pub(super) fn inspect_accepted(
        &self,
        locked: &LockDigest,
    ) -> Result<(TreeState, Option<Accepted>), SyncError> {
        let Some(hash) = self.recorded(locked) else {
            return Ok((TreeState::Absent, None));
        };
        let dir = self.tree_dir(&hash);
        if fs::symlink_metadata(&dir).is_err() {
            return Ok((TreeState::Absent, None));
        }
        let verified = verify_tree_capturing(&dir, &hash);
        #[cfg(feature = "test-support")]
        self.observe(&dir);
        match verified {
            Ok(manifest) => Ok((TreeState::Intact(dir), Some(Accepted { hash, manifest }))),
            Err(ExtractError::Tampered { .. }) => Ok((TreeState::Tampered(dir), None)),
            Err(e) => Err(SyncError::Store(e.unescaped())),
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
