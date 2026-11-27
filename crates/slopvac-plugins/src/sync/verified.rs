//! The tree hash that a loader may trust.
//!
//! The store keeps a record from each locked digest to a tree hash, but the cache is not
//! trusted (threat model E8): whoever can write it can write a tree and a record that points at
//! it. A loader that grants trust or loads a plugin therefore derives the hash from the lock:
//! the locked digest for a git pack, and for an archive the hash of the files that the cached
//! archive holds, after the archive is checked against the digest the lock pins. Nothing the
//! store records enters the result.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

use sha2::{Digest as _, Sha256};

use super::cache::Store;
use super::error::{CacheCause, SyncError};
use super::{Syncer, payload_layer};
use crate::extract::{Limits, TreeHash, hash_targz, open_regular};
use crate::lock::{LockedPack, SourceKind};
use crate::messages::sync as messages;
use crate::oci::{Digest, MAX_MANIFEST_BYTES, Manifest};

/// A tree directory and the hash that the lock and the cached archive give it.
///
/// The directory may not hold those files: a caller hashes it and compares the result with
/// `hash` before it reads anything from it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct VerifiedTree {
    /// Where the store keeps the tree of `hash`.
    pub dir: PathBuf,
    /// The hash derived from the lock.
    pub hash: TreeHash,
}

/// A reader that hashes what passes through it, so a file is hashed and used in one read, and
/// counts the bytes that it passed.
struct Hashing<R> {
    inner: R,
    hasher: Sha256,
    read: u64,
}

impl<R: Read> Read for Hashing<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        self.read += n as u64;
        Ok(n)
    }
}

/// What the cache holds for a small blob.
pub(super) enum Cached {
    /// No file of that name: only `NotFound` means this.
    Absent,
    /// The bytes, which hash to the digest they are named by.
    Bytes(Vec<u8>),
    /// A file whose bytes hash to something else.
    Mismatch(Digest),
}

/// The cause for a failed open or read of a cached file.
pub(super) fn cause_of(e: &io::Error) -> CacheCause {
    if e.kind() == io::ErrorKind::InvalidInput {
        CacheCause::NotRegular
    } else {
        CacheCause::io(e)
    }
}

impl Store {
    /// The tree that the lock pins for `pack`, with the hash derived from the lock alone.
    ///
    /// - A git pack's digest is its tree hash.
    /// - An archive (an `https://` URL or a file in the project) and an OCI pack have a cached
    ///   archive. It is read once, checked against the digest the lock pins (for OCI, through
    ///   the manifest the lock pins), and hashed with the rules and limits of extraction, so the
    ///   result is what extraction would give. Nothing is written and no network is used.
    ///
    /// The store's records are not read. `executables` is the set that sync used for the pack,
    /// which is part of the hash: a different set gives a different hash, and the load that
    /// follows then fails.
    ///
    /// Returns `None` when the cache lacks the archive or the tree. A project directory has no
    /// tree in the store, so it gives `None` as well; its digest is the hash, and the caller
    /// reads it from the lock.
    ///
    /// # Errors
    /// [`SyncError::Extract`] when the archive breaks an extraction rule, or the lock digest of a
    /// git pack is not a tree hash; [`SyncError::Layout`] for a manifest that is not a pack.
    pub fn verified_tree(
        &self,
        pack: &LockedPack,
        executables: &HashSet<String>,
        limits: Limits,
    ) -> Result<Option<VerifiedTree>, SyncError> {
        let extract = |source| SyncError::Extract {
            id: pack.id.clone(),
            source,
        };
        let hash = match pack.source.kind() {
            SourceKind::Git { .. } => TreeHash::parse(pack.digest.as_str()).map_err(extract)?,
            SourceKind::Path(_) | SourceKind::Archive(_) => {
                let digest = Syncer::locked_digest(pack)?;
                let Some(hash) = self.hash_cached_archive(pack, &digest, executables, limits)?
                else {
                    return Ok(None);
                };
                hash
            }
            SourceKind::Oci(_) => {
                let Some(layer) = self.cached_layer(pack, limits)? else {
                    return Ok(None);
                };
                let Some(hash) = self.hash_cached_archive(pack, &layer, executables, limits)?
                else {
                    return Ok(None);
                };
                hash
            }
        };
        let dir = self.tree_dir(&hash);
        match fs::symlink_metadata(&dir) {
            Ok(_) => Ok(Some(VerifiedTree { dir, hash })),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Store::unreadable_at(&pack.id, &dir, CacheCause::io(&e))),
        }
    }

    /// The digest of the pack layer of the cached manifest that the lock pins. The manifest is
    /// read once and checked in memory, so what is parsed is what the lock digest names.
    fn cached_layer(&self, pack: &LockedPack, limits: Limits) -> Result<Option<Digest>, SyncError> {
        let locked = Syncer::locked_digest(pack)?;
        let bytes = match self
            .read_cached(&locked, MAX_MANIFEST_BYTES as u64)
            .map_err(|cause| self.unreadable(&pack.id, &locked, cause))?
        {
            Cached::Bytes(bytes) => bytes,
            Cached::Absent => return Ok(None),
            Cached::Mismatch(actual) => {
                return Err(SyncError::DigestMismatch {
                    id: pack.id.clone(),
                    expected: locked.to_string(),
                    actual: actual.to_string(),
                });
            }
        };
        let layout = |why: &str| SyncError::Layout {
            id: pack.id.clone(),
            why: why.to_owned(),
        };
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|e| layout(&messages::manifest_parse(&e)))?;
        let layer = payload_layer(&manifest, &limits).map_err(layout)?;
        Ok(Some(layer.digest.clone()))
    }

    /// The bytes of a cached small blob, read once and checked against `digest`. Nothing is
    /// written or removed: sync, which repairs the cache, removes a [`Cached::Mismatch`].
    ///
    /// # Errors
    /// The cause when the file exists and cannot be read, is not a regular file or is over `max`.
    pub(super) fn read_cached(&self, digest: &Digest, max: u64) -> Result<Cached, CacheCause> {
        let path = self.blob_path(digest);
        let file = match open_regular(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Cached::Absent),
            Err(e) => return Err(cause_of(&e)),
        };
        let mut bytes = Vec::new();
        file.take(max.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| CacheCause::io(&e))?;
        if bytes.len() as u64 > max {
            return Err(CacheCause::TooLarge { cap: max });
        }
        let actual = Digest::of(&bytes);
        if actual != *digest {
            return Ok(Cached::Mismatch(actual));
        }
        #[cfg(feature = "test-support")]
        self.observe(&path);
        Ok(Cached::Bytes(bytes))
    }

    /// Hash the cached archive `digest` as extraction would, in one read that also checks the
    /// archive against its name. The read is bounded by the compressed size limit plus one byte,
    /// the drain included, and nothing is removed: sync repairs the cache.
    fn hash_cached_archive(
        &self,
        pack: &LockedPack,
        digest: &Digest,
        executables: &HashSet<String>,
        limits: Limits,
    ) -> Result<Option<TreeHash>, SyncError> {
        let path = self.blob_path(digest);
        let unreadable = |cause| self.unreadable(&pack.id, digest, cause);
        let file = match open_regular(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(unreadable(cause_of(&e))),
        };
        let cap = limits.compressed_bytes;
        let mut reader = Hashing {
            inner: file.take(cap.saturating_add(1)),
            hasher: Sha256::new(),
            read: 0,
        };
        let hashed = hash_targz(&mut reader, executables, limits);
        // The archive must be read to its end, so a trailing swap cannot go unhashed, but never
        // past the limit plus one byte.
        io::copy(&mut reader, &mut io::sink()).map_err(|e| unreadable(CacheCause::io(&e)))?;
        #[cfg(feature = "test-support")]
        self.count_archive_bytes(reader.read);
        if reader.read > cap {
            return Err(unreadable(CacheCause::TooLarge { cap }));
        }
        match Digest::parse_raw(&format!("sha256:{:x}", reader.hasher.finalize())) {
            Ok(actual) if actual == *digest => {}
            Ok(actual) => {
                return Err(SyncError::DigestMismatch {
                    id: pack.id.clone(),
                    expected: digest.to_string(),
                    actual: actual.to_string(),
                });
            }
            Err(_) => {
                return Err(unreadable(CacheCause::Io {
                    kind: io::ErrorKind::InvalidData,
                    os_code: None,
                }));
            }
        }
        hashed.map(Some).map_err(|source| SyncError::Extract {
            id: pack.id.clone(),
            source,
        })
    }
}
