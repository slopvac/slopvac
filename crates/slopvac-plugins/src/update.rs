//! `slopvac plugins update`: move the lock to the newest versions the config
//! ranges allow (contract: `docs/contract/packs.md` section 7; threat model
//! sections 5 and 6.5; policies.md exit codes).
//!
//! This module is a library. The CLI (`slopvac-8l4.12`) reads the config
//! ranges, builds an [`Updater`] and prints the [`UpdateReport`].
//!
//! # What update does
//!
//! Update needs the network. Offline it fails with [`UpdateError::Offline`]
//! (exit 2) before it reads anything, because it has nothing to resolve from.
//! It never prompts and never touches the trust store: a new version is
//! content the user has not reviewed, so `plugins trust` stays a separate step
//! (a pack that needs re-trust is refused at spawn, not here).
//!
//! Online, it gets the revocation list ([`Revocations::refresh`]), then for each
//! OCI entry in scope:
//!
//! 1. lists the repository's semver tags ([`available_versions`]) and picks the
//!    highest version the config range allows ([`outdated`]);
//! 2. fetches that version's manifest and takes its digest from the bytes;
//! 3. verifies the signed statement under the [`KeyPolicy`], with the lock's
//!    version and `notBefore` as the rollback floor (threat model K1-K3), so a
//!    statement older than the lock is refused. An entry that has no signature
//!    yet has no floor, which lets `update` sign it after the user pins a key;
//! 4. refuses an artifact digest or a signing key on the revocation list (K5).
//!
//! A refusal is per pack: the other packs still update and the report names the
//! reason ([`Outcome::Refused`]), and the exit code is 2. Entries that are not
//! OCI sources, or have no range, are left as they are. A newer version the
//! range excludes is never taken; the report names it
//! ([`PackUpdate::outside_range`]).
//!
//! The new lock is written in canonical form through a temporary file and a
//! rename, only when it differs from the file, so a second run changes nothing.
//! Update does not extract anything; `plugins sync` installs the new lock.
//!
//! `slopvac_api` and `requires` are carried over from the old entry.
// STUB(slopvac-8l4.9.36): the new version's values come from its pack manifest,
// and no manifest reader exists yet. Until the swap lands, an update that
// changes them needs a hand edit of the lock.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::str::FromStr;

use thiserror::Error;

use crate::lock::{
    LockError, LockedPack, Lockfile, MAX_LOCK_BYTES, PackId, Signature, Source, SourceKind,
    Version, VersionReq, outdated,
};
use crate::oci::{Client, MEDIA_MANIFEST, Reference, Target};
use crate::revoke::{RevocationError, RevocationList, RevocationNotice, Revocations};
use crate::sign::{Expectation, KeyPolicy, MEDIA_MINISIG, MEDIA_STATEMENT, fetch_and_verify};
use crate::sync::{ARTIFACT_TYPE_PACK, MEDIA_PACK_LAYER, Network, SyncError, available_versions};

/// Why update could not run at all. Every variant makes the command exit 2.
/// A refusal of one pack is not an error; see [`Outcome::Refused`].
#[derive(Debug, Error)]
pub enum UpdateError {
    /// Offline mode. Update has no cache to resolve from.
    #[error("offline: `plugins update` needs the network; run it online or use `plugins sync`")]
    Offline,
    /// A named pack is not in the lock.
    #[error("pack `{0}` is not in the lock")]
    UnknownPack(PackId),
    /// The lock file is unreadable, too large or invalid.
    #[error("lock file {path}: {message}")]
    Lock {
        /// The file.
        path: String,
        /// What is wrong.
        message: String,
    },
    /// The revocation list could not be fetched or verified.
    #[error(transparent)]
    Revocation(#[from] RevocationError),
    /// The new lock could not be written.
    #[error("writing {path}: {source}")]
    Write {
        /// The file.
        path: String,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
}

impl UpdateError {
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
            Self::Offline => "offline-update",
            Self::Revocation(e) => e.code(),
            _ => "update-failed",
        }
    }
}

/// What update did with one pack.
#[derive(Debug)]
pub enum Outcome {
    /// The lock entry moved to a newer version.
    Updated {
        /// The new version.
        to: Version,
    },
    /// Nothing newer in range (or the entry is not in scope).
    Unchanged,
    /// A newer in-range version exists but failed a check. The old entry stays.
    Refused(SyncError),
}

/// The result for one pack.
#[derive(Debug)]
pub struct PackUpdate {
    /// Pack id.
    pub id: PackId,
    /// The version the lock held before.
    pub locked: Version,
    /// What happened.
    pub outcome: Outcome,
    /// The newest stable version above the lock that the range excludes, when
    /// it is newer than what update chose. Changing the range is up to the user.
    pub outside_range: Option<Version>,
}

/// What update did, sorted by pack id.
#[derive(Debug, Default)]
pub struct UpdateReport {
    /// One entry per pack in scope.
    pub packs: Vec<PackUpdate>,
    /// Revocation-list notices. They do not fail the run.
    pub notices: Vec<RevocationNotice>,
    /// Whether the lock file was rewritten.
    pub wrote_lock: bool,
}

impl UpdateReport {
    /// 2 when any pack was refused, otherwise 0.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        if self
            .packs
            .iter()
            .any(|p| matches!(p.outcome, Outcome::Refused(_)))
        {
            2
        } else {
            0
        }
    }
}

/// Resolves newer pack versions and rewrites the lock.
pub struct Updater<'a> {
    network: Network<'a>,
    policy: &'a KeyPolicy,
    revocations: &'a Revocations,
}

impl<'a> Updater<'a> {
    /// `policy` carries the keys; `revocations` says where the revocation list
    /// comes from and is cached.
    #[must_use]
    pub fn new(network: Network<'a>, policy: &'a KeyPolicy, revocations: &'a Revocations) -> Self {
        Self {
            network,
            policy,
            revocations,
        }
    }

    /// Updates the lock file at `lock_path`.
    ///
    /// `ranges` are the config version requirements by pack id; a pack with no
    /// range stays as it is. `only` limits the run to the named packs; an empty
    /// slice means every pack.
    ///
    /// # Errors
    /// An [`UpdateError`] when the run cannot start or finish; a pack that
    /// fails a check is reported in the [`UpdateReport`] instead.
    pub fn update(
        &self,
        lock_path: &Path,
        ranges: &BTreeMap<PackId, VersionReq>,
        only: &[PackId],
    ) -> Result<UpdateReport, UpdateError> {
        let Network::Online(client) = self.network else {
            return Err(UpdateError::Offline);
        };
        let shown = lock_path.display().to_string();
        let lock_error = |message: String| UpdateError::Lock {
            path: shown.clone(),
            message,
        };
        let text = read_lock(lock_path).map_err(lock_error)?;
        let lock = Lockfile::parse(&text).map_err(|e: LockError| lock_error(e.to_string()))?;
        if let Some(missing) = only.iter().find(|id| lock.get(id).is_none()) {
            return Err(UpdateError::UnknownPack(missing.clone()));
        }
        let current = self.revocations.refresh(client)?;

        let in_scope = |pack: &&LockedPack| only.is_empty() || only.contains(&pack.id);
        let mut results: BTreeMap<PackId, SyncError> = BTreeMap::new();
        let mut repos: BTreeMap<PackId, Reference> = BTreeMap::new();
        let mut available: BTreeMap<PackId, Vec<Version>> = BTreeMap::new();
        for pack in lock.packs().iter().filter(in_scope) {
            let SourceKind::Oci(text) = pack.source.kind() else {
                continue;
            };
            if !ranges.contains_key(&pack.id) {
                continue;
            }
            let listed = reference_of(pack, text).and_then(|r| {
                available_versions(client, &r)
                    .map(|v| (r, v))
                    .map_err(|source| SyncError::Fetch {
                        id: pack.id.clone(),
                        source,
                    })
            });
            match listed {
                Ok((reference, versions)) => {
                    repos.insert(pack.id.clone(), reference);
                    available.insert(pack.id.clone(), versions);
                }
                Err(e) => {
                    results.insert(pack.id.clone(), e);
                }
            }
        }
        let newer = outdated(&lock, ranges, &available);

        let mut report = UpdateReport {
            notices: current.notices,
            ..UpdateReport::default()
        };
        let mut next: Vec<LockedPack> = lock.packs().to_vec();
        for pack in lock.packs().iter().filter(in_scope) {
            let found = newer.iter().find(|o| o.id == pack.id);
            let target = found.and_then(|o| o.update.clone());
            let outside_range = found.and_then(|o| {
                o.newest
                    .clone()
                    .filter(|n| *n > o.update.clone().unwrap_or_else(|| pack.version.clone()))
            });
            let outcome = if let Some(e) = results.remove(&pack.id) {
                Outcome::Refused(e)
            } else if let (Some(version), Some(repo)) = (target, repos.get(&pack.id)) {
                match resolve(client, self.policy, &current.list, pack, repo, &version) {
                    Ok(entry) => {
                        if let Some(slot) = next.iter_mut().find(|p| p.id == pack.id) {
                            *slot = entry;
                        }
                        Outcome::Updated { to: version }
                    }
                    Err(e) => Outcome::Refused(e),
                }
            } else {
                Outcome::Unchanged
            };
            report.packs.push(PackUpdate {
                id: pack.id.clone(),
                locked: pack.version.clone(),
                outcome,
                outside_range,
            });
        }

        let updated = Lockfile::new(next).map_err(|e| lock_error(e.to_string()))?;
        let canonical = updated.to_toml();
        if canonical != text {
            write_atomic(lock_path, canonical.as_bytes()).map_err(|source| UpdateError::Write {
                path: shown,
                source,
            })?;
            report.wrote_lock = true;
        }
        Ok(report)
    }
}

fn read_lock(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_LOCK_BYTES {
        return Err(format!("larger than {MAX_LOCK_BYTES} bytes"));
    }
    String::from_utf8(bytes).map_err(|_| "not UTF-8".to_owned())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let mut tmp = tempfile::Builder::new()
        .prefix(".packs.lock-")
        .tempfile_in(dir.unwrap_or_else(|| Path::new(".")))?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

fn reference_of(pack: &LockedPack, text: &str) -> Result<Reference, SyncError> {
    Reference::parse(text.strip_prefix("oci://").unwrap_or(text)).map_err(|source| {
        SyncError::Fetch {
            id: pack.id.clone(),
            source,
        }
    })
}

/// Fetches, verifies and checks `version` of `pack`; returns the new entry.
fn resolve(
    client: &Client,
    policy: &KeyPolicy,
    revoked: &RevocationList,
    pack: &LockedPack,
    repository: &Reference,
    version: &Version,
) -> Result<LockedPack, SyncError> {
    let id = &pack.id;
    let layout = |why: &str| SyncError::Layout {
        id: id.clone(),
        why: why.to_owned(),
    };
    let fetch = |source| SyncError::Fetch {
        id: id.clone(),
        source,
    };
    let tag = version.to_string();
    let fetched = client
        .fetch_manifest(&repository.with_target(Target::Tag(tag.clone())))
        .map_err(fetch)?;
    if fetched.media_type != MEDIA_MANIFEST {
        return Err(layout("the manifest is not an image manifest"));
    }
    if revoked.revokes_digest(fetched.digest.as_str()) {
        return Err(SyncError::Revoked {
            id: id.clone(),
            what: format!("artifact {}", fetched.digest),
        });
    }
    let manifest = fetched
        .manifest()
        .map_err(|e| layout(&format!("the manifest does not parse: {e}")))?;
    let mut payload = manifest
        .layers
        .iter()
        .filter(|l| l.media_type != MEDIA_STATEMENT && l.media_type != MEDIA_MINISIG);
    let (Some(layer), None) = (payload.next(), payload.next()) else {
        return Err(layout("a pack has exactly one content layer"));
    };
    if layer.media_type != MEDIA_PACK_LAYER {
        return Err(layout("the content layer is not a pack archive"));
    }

    let namespace = repository.repository.split('/').next().unwrap_or_default();
    let by_digest = repository.with_target(Target::Digest(fetched.digest.clone()));
    let locked_version = pack.version.to_string();
    let expect = Expectation {
        id: id.as_str(),
        namespace,
        version: &tag,
        artifact_type: ARTIFACT_TYPE_PACK,
        floor: pack
            .signature
            .as_ref()
            .map(|s| (locked_version.as_str(), s.not_before.as_str())),
        allow_downgrade: false,
    };
    let verified =
        fetch_and_verify(client, &by_digest, &manifest, policy, &expect).map_err(|source| {
            SyncError::Sign {
                id: id.clone(),
                source,
            }
        })?;
    let key_id = verified.key().key_id();
    if revoked.revokes_key(&key_id) {
        return Err(SyncError::Revoked {
            id: id.clone(),
            what: format!("publisher key {key_id}"),
        });
    }

    let source_text = match &repository.target {
        Target::Digest(_) => format!(
            "oci://{}/{}@{}",
            repository.registry, repository.repository, fetched.digest
        ),
        Target::Tag(_) => format!(
            "oci://{}/{}:{tag}",
            repository.registry, repository.repository
        ),
    };
    let invalid = |why: String| SyncError::Source {
        id: id.clone(),
        why,
    };
    Ok(LockedPack {
        version: version.clone(),
        source: Source::from_str(&source_text).map_err(|e| invalid(e.to_string()))?,
        digest: fetched
            .digest
            .as_str()
            .parse()
            .map_err(|e: crate::lock::ValueError| invalid(e.to_string()))?,
        signature: Some(Signature {
            key_id: key_id
                .parse()
                .map_err(|e: crate::lock::ValueError| invalid(e.to_string()))?,
            not_before: verified
                .statement()
                .not_before
                .parse()
                .map_err(|e: crate::lock::ValueError| invalid(e.to_string()))?,
        }),
        ..pack.clone()
    })
}
