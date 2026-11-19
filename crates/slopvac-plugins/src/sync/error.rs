use std::path::PathBuf;

use slopvac_sanitize::line as esc;
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
    #[error("offline: pack `{id}` ({}) is not in the cache: {}", esc(.digest), esc(.what))]
    OfflinePackMissing {
        /// Pack id.
        id: PackId,
        /// The digest the lock pins.
        digest: String,
        /// What is missing.
        what: String,
    },
    /// The lock entry cannot be resolved as written.
    #[error("pack `{id}`: {}", esc(.why))]
    Source {
        /// Pack id.
        id: PackId,
        /// What is wrong.
        why: String,
    },
    /// Fetching or exporting a pinned git commit failed, or the commit holds
    /// something a pack may not (a link or a submodule).
    #[error("pack `{id}`: git source: {}", esc(.why))]
    Git {
        /// Pack id.
        id: PackId,
        /// What went wrong.
        why: String,
    },
    /// A git source passed a limit: the download size, the number of files or
    /// the expanded size.
    #[error("pack `{id}`: git source passes a limit: {}", esc(.why))]
    GitLimit {
        /// Pack id.
        id: PackId,
        /// Which limit.
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
    #[error("pack `{id}`: the signature differs from the lock: {}", esc(.why))]
    SignatureDiffersFromLock {
        /// Pack id.
        id: PackId,
        /// What differs.
        why: String,
    },
    /// Fetched or read content does not hash to the locked digest.
    #[error("pack `{id}`: digest mismatch: the lock pins {}, the content is {}", esc(.expected), esc(.actual))]
    DigestMismatch {
        /// Pack id.
        id: PackId,
        /// Locked digest.
        expected: String,
        /// Computed digest.
        actual: String,
    },
    /// The manifest does not have the shape of a pack artifact.
    #[error("pack `{id}`: unexpected artifact layout: {}", esc(.why))]
    Layout {
        /// Pack id.
        id: PackId,
        /// What is wrong.
        why: String,
    },
    /// An installed tree changed and sync is offline, so it cannot be fetched
    /// again. The tree was removed.
    #[error("pack `{id}`: the cached tree {path} was modified and was removed; offline sync cannot restore it", path = esc(&path.to_string_lossy()))]
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
    #[error("data directory: {}", esc(.0))]
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
            // A credential helper refused while fetching a signature blob is a
            // login problem, not a bad signature.
            Self::Sign {
                source: SignError::Fetch(e),
                ..
            } if e.code().is_some() => e.code().unwrap_or("signature-invalid"),
            Self::Sign { .. } | Self::Unsigned { .. } | Self::SignatureDiffersFromLock { .. } => {
                "signature-invalid"
            }
            Self::Revoked { .. } => "pack-revoked",
            Self::Revocation(e) => e.code(),
            Self::Fetch { source, .. } => source.code().unwrap_or("sync-failed"),
            _ => "sync-failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal() -> OciError {
        OciError::HelperUntrusted {
            program: "docker-credential-x".into(),
            dir: "tools".into(),
        }
    }

    fn sign_fetch(e: OciError) -> SyncError {
        SyncError::Sign {
            id: "demo".parse().unwrap(),
            source: SignError::Fetch(e),
        }
    }

    #[test]
    fn a_helper_refusal_while_fetching_a_signature_keeps_its_code() {
        assert_eq!(sign_fetch(refusal()).code(), "credential-helper-untrusted");
    }

    #[test]
    fn another_failure_while_fetching_a_signature_is_still_a_signature_failure() {
        let err = sign_fetch(OciError::NotFound("sig".into()));
        assert_eq!(err.code(), "signature-invalid");
    }

    #[test]
    fn a_helper_refusal_while_fetching_a_pack_keeps_its_code() {
        let err = SyncError::Fetch {
            id: "demo".parse().unwrap(),
            source: refusal(),
        };
        assert_eq!(err.code(), "credential-helper-untrusted");
    }
}
