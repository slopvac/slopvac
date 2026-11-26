//! The tree hash that a loader may trust.
//!
//! The store keeps a record from each locked digest to a tree hash, but the cache is not
//! trusted (threat model E8): whoever can write it can write a tree and a record that points at
//! it. A loader that grants trust or loads a plugin therefore derives the hash from the lock:
//! the locked digest for a git pack, and for an archive the hash of the files that the cached
//! archive holds, after the archive is checked against the digest the lock pins. Nothing the
//! store records enters the result.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::PathBuf;

use sha2::{Digest as _, Sha256};

use super::cache::Store;
use super::error::SyncError;
use super::{payload_layer, Syncer};
use crate::extract::{hash_targz, open_regular, ExtractError, Limits, TreeHash};
use crate::lock::{LockedPack, SourceKind};
use crate::messages::sync as messages;
use crate::oci::{Digest, Manifest, MAX_MANIFEST_BYTES};

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

/// A reader that hashes what passes through it, so a file is hashed and used in one read.
struct Hashing<R> {
    inner: R,
    hasher: Sha256,
}

impl<R: Read> Read for Hashing<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
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
        Ok(fs::symlink_metadata(&dir)
            .is_ok()
            .then_some(VerifiedTree { dir, hash }))
    }

    /// The digest of the pack layer of the cached manifest that the lock pins. The manifest is
    /// read once and checked in memory, so what is parsed is what the lock digest names.
    fn cached_layer(&self, pack: &LockedPack, limits: Limits) -> Result<Option<Digest>, SyncError> {
        let locked = Syncer::locked_digest(pack)?;
        let Some(bytes) = self.read_cached(&locked, MAX_MANIFEST_BYTES as u64) else {
            return Ok(None);
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

    /// The bytes of a cached small blob if they hash to `digest`. A blob that does not is
    /// removed, as [`Store::blob`] does.
    fn read_cached(&self, digest: &Digest, max: u64) -> Option<Vec<u8>> {
        let path = self.blob_path(digest);
        let mut bytes = Vec::new();
        open_regular(&path)
            .ok()?
            .take(max + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 <= max && Digest::of(&bytes) == *digest {
            Some(bytes)
        } else {
            let _ = fs::remove_file(&path);
            None
        }
    }

    /// Hash the cached archive `digest` as extraction would, in one read that also checks the
    /// archive against its name.
    fn hash_cached_archive(
        &self,
        pack: &LockedPack,
        digest: &Digest,
        executables: &HashSet<String>,
        limits: Limits,
    ) -> Result<Option<TreeHash>, SyncError> {
        let path = self.blob_path(digest);
        let Ok(file) = open_regular(&path) else {
            return Ok(None);
        };
        let mut reader = Hashing {
            inner: file,
            hasher: Sha256::new(),
        };
        let hashed = hash_targz(&mut reader, executables, limits);
        // The archive must be read to its end, so a trailing swap cannot go unhashed.
        io::copy(&mut reader, &mut io::sink()).map_err(|e| SyncError::Extract {
            id: pack.id.clone(),
            source: ExtractError::Io {
                what: messages::opening(&path),
                source: e,
            },
        })?;
        match Digest::parse_raw(&format!("sha256:{:x}", reader.hasher.finalize())) {
            Ok(actual) if actual == *digest => {}
            _ => {
                let _ = fs::remove_file(&path);
                return Ok(None);
            }
        }
        hashed.map(Some).map_err(|source| SyncError::Extract {
            id: pack.id.clone(),
            source,
        })
    }
}
