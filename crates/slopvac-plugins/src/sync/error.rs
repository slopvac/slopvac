use std::path::PathBuf;

use thiserror::Error;

use crate::extract::ExtractError;
use crate::lock::{LockDiff, PackId};
use crate::oci::OciError;
use crate::revoke::RevocationError;
use crate::sign::SignError;

/// Why a sync, status or check failed. Every variant makes the command exit 2
/// (policies.md, section 3.5): the run could not establish that the installed
/// packs are the locked packs.
#[derive(Debug, Error)]
pub enum SyncError {
    /// Offline mode and the cache lacks content the lock needs.
    #[error("offline: pack `{id}` ({digest}) is not in the cache: {what}")]
    OfflinePackMissing {
        /// Pack id.
        id: PackId,
        /// The digest the lock pins.
        digest: String,
        /// What is missing.
        what: String,
    },
    /// The source kind is not implemented yet.
    #[error("pack `{id}`: {reason}")]
    Unsupported {
        /// Pack id.
        id: PackId,
        /// Why.
        reason: String,
    },
    /// The lock entry cannot be resolved as written.
    #[error("pack `{id}`: {why}")]
    Source {
        /// Pack id.
        id: PackId,
        /// What is wrong.
        why: String,
    },
    /// The registry or the network failed.
    #[error("pack `{id}`: {source}")]
    Fetch {
        /// Pack id.
        id: PackId,
        /// The cause.
        #[source]
        source: OciError,
    },
    /// The signature gate refused the artifact.
    #[error("pack `{id}`: signature check failed: {source}")]
    Sign {
        /// Pack id.
        id: PackId,
        /// The cause.
        #[source]
        source: SignError,
    },
    /// An OCI entry has no `[packs.signature]` table. Data packs must be signed.
    #[error("pack `{id}`: the lock has no signature for an OCI pack; pin its key, then update")]
    Unsigned {
        /// Pack id.
        id: PackId,
    },
    /// The signed statement is valid but is not the one the lock records.
    #[error("pack `{id}`: the signature differs from the lock: {why}")]
    SignatureDiffersFromLock {
        /// Pack id.
        id: PackId,
        /// What differs.
        why: String,
    },
    /// Fetched or read content does not hash to the locked digest.
    #[error("pack `{id}`: digest mismatch: the lock pins {expected}, the content is {actual}")]
    DigestMismatch {
        /// Pack id.
        id: PackId,
        /// Locked digest.
        expected: String,
        /// Computed digest.
        actual: String,
    },
    /// The manifest does not have the shape of a pack artifact.
    #[error("pack `{id}`: unexpected artifact layout: {why}")]
    Layout {
        /// Pack id.
        id: PackId,
        /// What is wrong.
        why: String,
    },
    /// An installed tree changed and sync is offline, so it cannot be fetched
    /// again. The tree was removed.
    #[error("pack `{id}`: the cached tree {path} was modified and was removed; offline sync cannot restore it", path = path.display())]
    CacheTampered {
        /// Pack id.
        id: PackId,
        /// The tree.
        path: PathBuf,
    },
    /// Safe extraction refused the archive, or a file operation failed.
    #[error("pack `{id}`: {source}")]
    Extract {
        /// Pack id.
        id: PackId,
        /// The cause.
        #[source]
        source: ExtractError,
    },
    /// The data directory could not be prepared or written.
    #[error("data directory: {0}")]
    Store(String),
    /// The revocation list names the pack's artifact or its signing key
    /// (threat model K5).
    #[error("pack `{id}`: {what} is on the revocation list")]
    Revoked {
        /// Pack id.
        id: PackId,
        /// What is revoked.
        what: String,
    },
    /// The revocation list could not be fetched, verified or read from the cache.
    #[error(transparent)]
    Revocation(#[from] RevocationError),
    /// The resolved packs differ from the lock.
    #[error("the lock does not match the resolved packs:\n{0}")]
    LockMismatch(LockDiff),
}

impl SyncError {
    /// The exit code. Slopvac uses no other failing code for an incomplete run.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        2
    }

    /// The stable error code of the contract (policies.md, section 1.3), where
    /// one exists.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::OfflinePackMissing { .. } => "offline-pack-missing",
            Self::LockMismatch(_) => "lock-mismatch",
            Self::CacheTampered { .. } => "cache-tampered",
            Self::DigestMismatch { .. } => "digest-mismatch",
            Self::Sign { .. } | Self::Unsigned { .. } | Self::SignatureDiffersFromLock { .. } => {
                "signature-invalid"
            }
            Self::Revoked { .. } => "pack-revoked",
            Self::Revocation(e) => e.code(),
            _ => "sync-failed",
        }
    }
}
