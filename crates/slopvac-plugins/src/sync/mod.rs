//! `slopvac plugins sync`, `status` and the CI lock check (contract:
//! `docs/contract/packs.md` section 7, threat model sections 5 and 6.5).
//!
//! This module is a library. The CLI (`slopvac-8l4.12`) picks the data
//! directory and the network mode, builds a [`Syncer`] and prints the result.
//!
//! # What sync does
//!
//! `sync` installs exactly the lock, nothing newer. For each entry it:
//!
//! 1. resolves the source: an OCI reference (fetched by the lock's manifest
//!    digest, never by tag), an `https://` archive, or a path in the project;
//! 2. checks the digest the lock pins before anything is trusted;
//! 3. for OCI, verifies the minisign statement against the keys in
//!    [`KeyPolicy`] and checks that it is the signature the lock records (key
//!    id and `notBefore` floor). A first-party namespace with no compiled key
//!    fails closed until `slopvac-8l4.9.16` lands them (TM-22);
//! 4. extracts the layer into the store by tree hash ([`crate::extract`]).
//!
//! Before any pack, sync gets the revocation list ([`crate::revoke`]): online
//! it fetches and caches it, offline it reads the cache. A locked digest or a
//! signing key on the list fails with [`SyncError::Revoked`] before anything
//! is extracted. A stale or missing cache only adds a notice to the report.
//!
//! Every pack is resolved and verified before the first one is extracted, so a
//! bad pack leaves the store unchanged (all-or-nothing for verification). The
//! lock file is never written here.
//!
//! # Cache and offline mode
//!
//! Content is cached under its own sha256 (see [`Store`]). A cached blob is
//! hashed again on every use, so the network is only needed for content the
//! cache lacks; a repeat sync makes no request for pack content (it still
//! fetches the revocation list). Offline mode
//! ([`Network::Offline`]) uses nothing else and fails with
//! [`SyncError::OfflinePackMissing`] (`offline-pack-missing`), naming the pack
//! and the digest. A cached tree that no longer matches its record is removed
//! (threat model E8): online it is installed again, offline sync exits 2.
//!
//! Sync never trusts a record to say which tree belongs to a digest: it
//! extracts again from the verified blob and records the resulting tree hash.
//! Whatever loads a pack at run time recomputes the tree hash itself.
//!
//! Concurrent syncs into one store are safe for extraction (atomic renames) but
//! may both download a blob. Locking is not implemented.

mod cache;
mod error;

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use semver::{Version, VersionReq};

pub use cache::{Store, StoreLock, TreeState};
pub use error::SyncError;

use crate::extract::{ExtractError, Limits, extract_targz, tree_hash};
use crate::lock::{
    LockedPack, Lockfile, PackId, ResolvedPack, SourceKind, diff, outdated as lock_outdated,
};
use crate::oci::{
    Client, Descriptor, Digest, MAX_MANIFEST_BYTES, MEDIA_MANIFEST, Manifest, OciError, Reference,
    Target,
};
use crate::revoke::{RevocationList, RevocationNotice, Revocations};
use crate::sign::{
    Expectation, KeyPolicy, MAX_STATEMENT_BYTES, MEDIA_MINISIG, MEDIA_STATEMENT, signature_layers,
    verify,
};

/// `artifactType` of a pack artifact. Provisional: the contract names the
/// field but not yet its value (`slopvac-8l4.9.5`).
pub const ARTIFACT_TYPE_PACK: &str = "application/vnd.slopvac.pack.v1";
/// Media type of the layer that holds the pack as a `.tar.gz`.
pub const MEDIA_PACK_LAYER: &str = "application/vnd.slopvac.pack.layer.v1.tar+gzip";
const MAX_MINISIG_BYTES: u64 = 4 * 1024;

/// Whether sync may use the network.
#[derive(Clone, Copy)]
pub enum Network<'a> {
    /// Offline mode: use the cache only.
    Offline,
    /// Fetch what the cache lacks through this client.
    Online(&'a Client),
}

/// What sync did with one pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Extracted for the first time (or after the record was lost).
    Installed,
    /// Already installed; nothing changed.
    Unchanged,
    /// A modified tree was removed and extracted again.
    Reinstalled,
    /// A directory in the project, used where it is. Its tree hash matches the lock.
    InPlace,
}

/// The result for one pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackOutcome {
    /// Pack id.
    pub id: PackId,
    /// Locked version.
    pub version: Version,
    /// What happened.
    pub change: Change,
    /// The tree to load the pack from.
    pub tree: PathBuf,
    /// A modified tree too large to delete was moved here and keeps using storage.
    pub quarantined: Option<PathBuf>,
}

/// What sync did, sorted by pack id.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncReport {
    /// One entry per locked pack.
    pub packs: Vec<PackOutcome>,
    /// Revocation-list notices (stale or missing cache). They do not fail the sync.
    pub notices: Vec<RevocationNotice>,
}

enum Acquired {
    /// A verified archive in the blob cache.
    Archive(PathBuf),
    /// A verified directory in the project.
    InPlace(PathBuf),
}

/// Installs a lock into a [`Store`].
pub struct Syncer<'a> {
    store: &'a Store,
    network: Network<'a>,
    policy: &'a KeyPolicy,
    project_root: &'a Path,
    revocations: &'a Revocations,
    limits: Limits,
    executables: BTreeMap<PackId, HashSet<String>>,
}

impl<'a> Syncer<'a> {
    /// `project_root` anchors `path` sources. `policy` carries the keys: the
    /// compiled first-party keys and the keys the user pinned. `revocations`
    /// says where the revocation list comes from and is cached.
    #[must_use]
    pub fn new(
        store: &'a Store,
        network: Network<'a>,
        policy: &'a KeyPolicy,
        project_root: &'a Path,
        revocations: &'a Revocations,
    ) -> Self {
        Self {
            store,
            network,
            policy,
            project_root,
            revocations,
            limits: Limits::default(),
            executables: BTreeMap::new(),
        }
    }

    /// Use other extraction limits.
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Files that become executable in a pack's tree, by pack id.
    // STUB(slopvac-8l4.9.28): the executable set of a plugin pack comes from
    // its `[plugin]` command in the pack manifest. No manifest reader exists yet,
    // so the CLI passes the set in; a pack without an entry has no executables.
    #[must_use]
    pub fn with_executables(mut self, executables: BTreeMap<PackId, HashSet<String>>) -> Self {
        self.executables = executables;
        self
    }

    /// Install exactly `lock`.
    ///
    /// # Errors
    /// The first [`SyncError`]. Verification failures leave the store without
    /// new trees; see the module documentation.
    pub fn sync(&self, lock: &Lockfile) -> Result<SyncReport, SyncError> {
        let current = match self.network {
            Network::Online(client) => self.revocations.refresh(client)?,
            Network::Offline => self.revocations.cached()?,
        };
        let revoked = &current.list;
        for pack in lock.packs() {
            if revoked.revokes_digest(pack.digest.as_str()) {
                return Err(SyncError::Revoked {
                    id: pack.id.clone(),
                    what: format!("artifact {}", pack.digest),
                });
            }
        }
        let _lock = self.store.lock()?;
        let mut acquired = Vec::with_capacity(lock.packs().len());
        for pack in lock.packs() {
            acquired.push((pack, self.acquire(pack, revoked)?));
        }

        let mut states = Vec::with_capacity(acquired.len());
        for (pack, how) in &acquired {
            if matches!(how, Acquired::Archive(_)) {
                let state = self.store.inspect(&pack.digest)?;
                let mut set_aside = None;
                if let TreeState::Tampered(dir) = &state {
                    let aside = cache::evict_tree(dir)?;
                    set_aside.clone_from(&aside);
                    if matches!(self.network, Network::Offline) {
                        return Err(SyncError::CacheTampered {
                            id: pack.id.clone(),
                            path: aside.unwrap_or_else(|| dir.clone()),
                        });
                    }
                }
                states.push((state, set_aside));
            } else {
                states.push((TreeState::Absent, None));
            }
        }

        let mut report = SyncReport {
            notices: current.notices,
            ..SyncReport::default()
        };
        for ((pack, how), (before, aside)) in acquired.into_iter().zip(states) {
            let mut outcome = self.install(pack, &how, &before)?;
            // A modified tree too large to delete was moved aside; keep reporting it.
            outcome.quarantined = outcome.quarantined.or(aside);
            report.packs.push(outcome);
        }
        Ok(report)
    }

    fn install(
        &self,
        pack: &LockedPack,
        how: &Acquired,
        before: &TreeState,
    ) -> Result<PackOutcome, SyncError> {
        let outcome = |change, tree, quarantined| PackOutcome {
            id: pack.id.clone(),
            version: pack.version.clone(),
            change,
            tree,
            quarantined,
        };
        let blob = match how {
            Acquired::InPlace(dir) => return Ok(outcome(Change::InPlace, dir.clone(), None)),
            Acquired::Archive(blob) => blob,
        };
        let extract = |source| SyncError::Extract {
            id: pack.id.clone(),
            source,
        };
        let file = File::open(blob).map_err(|e| {
            extract(ExtractError::Io {
                what: format!("opening {}", blob.display()),
                source: e,
            })
        })?;
        let exec = self.executables.get(&pack.id).cloned().unwrap_or_default();
        let done = extract_targz(file, &self.store.trees(), &exec, self.limits).map_err(extract)?;
        self.store.record(&pack.digest, &done.hash)?;
        let change = match before {
            TreeState::Intact(dir) if *dir == done.root => Change::Unchanged,
            TreeState::Tampered(_) => Change::Reinstalled,
            _ => Change::Installed,
        };
        Ok(outcome(change, done.root, done.quarantined))
    }

    fn acquire(&self, pack: &LockedPack, revoked: &RevocationList) -> Result<Acquired, SyncError> {
        match pack.source.kind() {
            SourceKind::Oci(text) => self.acquire_oci(pack, text, revoked),
            SourceKind::Archive(url) => self.acquire_archive(pack, url),
            SourceKind::Path(rel) => self.acquire_path(pack, rel),
            // STUB(slopvac-8l4.9.27): a pinned-commit clone needs a git subprocess or a
            // new dependency (gitoxide); the brief asks for approval first.
            SourceKind::Git { .. } => Err(SyncError::Unsupported {
                id: pack.id.clone(),
                reason: "git sources are not supported yet".into(),
            }),
        }
    }

    fn client(&self, pack: &LockedPack, what: &str) -> Result<&'a Client, SyncError> {
        match self.network {
            Network::Online(client) => Ok(client),
            Network::Offline => Err(SyncError::OfflinePackMissing {
                id: pack.id.clone(),
                digest: pack.digest.to_string(),
                what: what.to_owned(),
            }),
        }
    }

    fn locked_digest(pack: &LockedPack) -> Result<Digest, SyncError> {
        Digest::parse_raw(pack.digest.as_str()).map_err(|why| SyncError::Source {
            id: pack.id.clone(),
            why,
        })
    }

    fn acquire_oci(
        &self,
        pack: &LockedPack,
        text: &str,
        revoked: &RevocationList,
    ) -> Result<Acquired, SyncError> {
        let id = &pack.id;
        let layout = |why: &str| SyncError::Layout {
            id: id.clone(),
            why: why.to_owned(),
        };
        let fetch = |source| SyncError::Fetch {
            id: id.clone(),
            source,
        };
        // Checked first so an unsigned entry costs no request.
        if pack.signature.is_none() {
            return Err(SyncError::Unsigned { id: id.clone() });
        }
        let locked = Self::locked_digest(pack)?;
        let parsed =
            Reference::parse(text.strip_prefix("oci://").unwrap_or(text)).map_err(fetch)?;
        if let Target::Digest(named) = &parsed.target
            && *named != locked
        {
            return Err(SyncError::DigestMismatch {
                id: id.clone(),
                expected: locked.to_string(),
                actual: named.to_string(),
            });
        }
        let reference = parsed.with_target(Target::Digest(locked.clone()));
        let namespace = reference
            .repository
            .split('/')
            .next()
            .unwrap_or_default()
            .to_owned();

        let body = self.manifest_bytes(pack, &reference, &locked)?;
        let manifest: Manifest = serde_json::from_slice(&body)
            .map_err(|e| layout(&format!("the manifest does not parse: {e}")))?;
        if manifest
            .media_type
            .as_deref()
            .is_some_and(|m| m != MEDIA_MANIFEST)
        {
            return Err(layout("the manifest is not an image manifest"));
        }

        let key_id = self.verify_signature(pack, &reference, &manifest, &namespace)?;
        if revoked.revokes_key(&key_id) {
            return Err(SyncError::Revoked {
                id: id.clone(),
                what: format!("publisher key {key_id}"),
            });
        }

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
        if layer.size > self.limits.compressed_bytes {
            return Err(layout("the content layer exceeds the archive size limit"));
        }
        if let Some(path) = self.store.blob(&layer.digest, layer.size) {
            return Ok(Acquired::Archive(path));
        }
        let client = self.client(pack, "pack archive")?;
        let mut tmp = self.store.blob_tempfile()?;
        client
            .pull_blob(&reference, layer, tmp.as_file_mut())
            .map_err(fetch)?;
        Ok(Acquired::Archive(
            self.store.commit_blob(tmp, &layer.digest)?,
        ))
    }

    /// The signature gate: the statement is signed by a policy key, names this
    /// artifact and is the one the lock records. Returns the signing key id.
    fn verify_signature(
        &self,
        pack: &LockedPack,
        reference: &Reference,
        manifest: &Manifest,
        namespace: &str,
    ) -> Result<String, SyncError> {
        let id = &pack.id;
        let layout = |why: &str| SyncError::Layout {
            id: id.clone(),
            why: why.to_owned(),
        };
        let lock_signature = pack
            .signature
            .as_ref()
            .ok_or_else(|| SyncError::Unsigned { id: id.clone() })?;
        let (statement, minisig) =
            signature_layers(manifest).map_err(|source| SyncError::Sign {
                id: id.clone(),
                source,
            })?;
        let statement_bytes = self.small_blob(
            pack,
            reference,
            statement,
            MAX_STATEMENT_BYTES as u64,
            "signed statement",
        )?;
        let minisig_bytes =
            self.small_blob(pack, reference, minisig, MAX_MINISIG_BYTES, "signature")?;
        let minisig_text =
            String::from_utf8(minisig_bytes).map_err(|_| layout("the signature is not text"))?;
        let version = pack.version.to_string();
        let expect = Expectation {
            id: id.as_str(),
            namespace,
            version: &version,
            artifact_type: ARTIFACT_TYPE_PACK,
            floor: Some((&version, lock_signature.not_before.as_str())),
            allow_downgrade: false,
        };
        let verified = verify(
            manifest,
            &statement_bytes,
            &minisig_text,
            self.policy,
            &expect,
        )
        .map_err(|source| SyncError::Sign {
            id: id.clone(),
            source,
        })?;
        let differs = |why: String| SyncError::SignatureDiffersFromLock {
            id: id.clone(),
            why,
        };
        if verified.key().key_id() != lock_signature.key_id.as_str() {
            return Err(differs(format!(
                "signed by key {}, the lock records {}",
                verified.key().key_id(),
                lock_signature.key_id
            )));
        }
        if verified.statement().not_before != lock_signature.not_before.as_str() {
            return Err(differs(format!(
                "notBefore is {}, the lock records {}",
                verified.statement().not_before,
                lock_signature.not_before
            )));
        }

        Ok(verified.key().key_id())
    }

    fn manifest_bytes(
        &self,
        pack: &LockedPack,
        reference: &Reference,
        locked: &Digest,
    ) -> Result<Vec<u8>, SyncError> {
        if let Some(path) = self.store.blob(locked, MAX_MANIFEST_BYTES as u64)
            && let Ok(bytes) = fs::read(path)
        {
            return Ok(bytes);
        }
        let client = self.client(pack, "manifest")?;
        let fetched = client
            .fetch_manifest(reference)
            .map_err(|source| SyncError::Fetch {
                id: pack.id.clone(),
                source,
            })?;
        self.store.put_blob(locked, &fetched.body)?;
        Ok(fetched.body)
    }

    fn small_blob(
        &self,
        pack: &LockedPack,
        reference: &Reference,
        desc: &Descriptor,
        max: u64,
        what: &str,
    ) -> Result<Vec<u8>, SyncError> {
        if let Some(path) = self.store.blob(&desc.digest, max)
            && let Ok(bytes) = fs::read(path)
        {
            return Ok(bytes);
        }
        let client = self.client(pack, what)?;
        let bytes = client
            .fetch_blob_bytes(reference, desc, max)
            .map_err(|source| SyncError::Fetch {
                id: pack.id.clone(),
                source,
            })?;
        self.store.put_blob(&desc.digest, &bytes)?;
        Ok(bytes)
    }

    fn acquire_archive(&self, pack: &LockedPack, url: &str) -> Result<Acquired, SyncError> {
        let locked = Self::locked_digest(pack)?;
        if let Some(path) = self.store.blob(&locked, self.limits.compressed_bytes) {
            return Ok(Acquired::Archive(path));
        }
        let client = self.client(pack, "release archive")?;
        let mut tmp = self.store.blob_tempfile()?;
        client
            .download(url, self.limits.compressed_bytes, tmp.as_file_mut())
            .map_err(|source| SyncError::Fetch {
                id: pack.id.clone(),
                source,
            })?;
        let actual =
            cache_hash(tmp.path()).map_err(|e| SyncError::Store(format!("hashing: {e}")))?;
        if actual != locked {
            return Err(SyncError::DigestMismatch {
                id: pack.id.clone(),
                expected: locked.to_string(),
                actual: actual.to_string(),
            });
        }
        Ok(Acquired::Archive(self.store.commit_blob(tmp, &locked)?))
    }

    fn acquire_path(&self, pack: &LockedPack, rel: &str) -> Result<Acquired, SyncError> {
        let id = &pack.id;
        let source = |why: String| SyncError::Source {
            id: id.clone(),
            why,
        };
        let mut path = self.project_root.to_path_buf();
        for part in rel.split('/') {
            path.push(part);
            let meta = fs::symlink_metadata(&path)
                .map_err(|e| source(format!("path source `{rel}`: {e}")))?;
            if meta.file_type().is_symlink() {
                return Err(source(format!("path source `{rel}` goes through a link")));
            }
        }
        // Defence in depth after the segment walk: the resolved path must stay inside
        // the project root, whatever the platform makes of the segments.
        let inside = fs::canonicalize(self.project_root)
            .and_then(|root| fs::canonicalize(&path).map(|p| p.starts_with(root)))
            .map_err(|e| source(format!("path source `{rel}`: {e}")))?;
        if !inside {
            return Err(source(format!("path source `{rel}` leaves the project")));
        }
        let mismatch = |actual: &str| SyncError::DigestMismatch {
            id: id.clone(),
            expected: pack.digest.to_string(),
            actual: actual.to_owned(),
        };
        if fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
            let hash = tree_hash(&path).map_err(|source| SyncError::Extract {
                id: id.clone(),
                source,
            })?;
            if hash.as_str() != pack.digest.as_str() {
                return Err(mismatch(hash.as_str()));
            }
            return Ok(Acquired::InPlace(path));
        }
        let locked = Self::locked_digest(pack)?;
        if let Some(cached) = self.store.blob(&locked, self.limits.compressed_bytes) {
            // Still check the file in the project: it is what the lock names.
            let actual = cache_hash(&path).map_err(|e| source(format!("reading `{rel}`: {e}")))?;
            if actual != locked {
                return Err(mismatch(actual.as_str()));
            }
            return Ok(Acquired::Archive(cached));
        }
        let mut tmp = self.store.blob_tempfile()?;
        let mut file = File::open(&path).map_err(|e| source(format!("reading `{rel}`: {e}")))?;
        let mut limited = (&mut file).take(self.limits.compressed_bytes + 1);
        let copied = io::copy(&mut limited, tmp.as_file_mut())
            .map_err(|e| source(format!("reading `{rel}`: {e}")))?;
        if copied > self.limits.compressed_bytes {
            return Err(source(format!("`{rel}` exceeds the archive size limit")));
        }
        let actual =
            cache_hash(tmp.path()).map_err(|e| SyncError::Store(format!("hashing: {e}")))?;
        if actual != locked {
            return Err(mismatch(actual.as_str()));
        }
        Ok(Acquired::Archive(self.store.commit_blob(tmp, &locked)?))
    }
}

fn cache_hash(path: &Path) -> io::Result<Digest> {
    cache::hash_file(path)
}

/// How one locked pack stands in the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackState {
    /// Not installed (or its record is gone).
    NotInstalled,
    /// The store records this tree for the locked digest and the tree still
    /// hashes to its record. Status does not re-derive the tree from the
    /// artifact, so an edited record would pass; `sync` always re-derives.
    Recorded(PathBuf),
    /// Installed, but the tree differs from its record. `sync` repairs it.
    Tampered(PathBuf),
    /// A project directory whose tree hash matches the lock.
    InPlace(PathBuf),
    /// A project directory whose tree hash differs from the lock.
    InPlaceChanged(PathBuf),
}

/// The state of one locked pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackStatus {
    /// Pack id.
    pub id: PackId,
    /// Locked version.
    pub version: Version,
    /// State in the store.
    pub state: PackState,
}

impl PackStatus {
    /// True when the pack has an intact recorded tree or a matching project
    /// directory. This is a report, not a proof: the CI gate is `sync` or
    /// [`check_lock`].
    #[must_use]
    pub fn is_recorded(&self) -> bool {
        matches!(self.state, PackState::Recorded(_) | PackState::InPlace(_))
    }
}

/// What is installed against `lock`, sorted by id. Reads trees; never writes.
///
/// # Errors
/// [`SyncError::Store`] when a tree cannot be read.
pub fn status(
    store: &Store,
    project_root: &Path,
    lock: &Lockfile,
) -> Result<Vec<PackStatus>, SyncError> {
    lock.packs()
        .iter()
        .map(|pack| {
            let state = match pack.source.kind() {
                SourceKind::Path(rel) if project_root.join(rel).is_dir() => {
                    let dir = project_root.join(rel);
                    match tree_hash(&dir) {
                        Ok(hash) if hash.as_str() == pack.digest.as_str() => {
                            PackState::InPlace(dir)
                        }
                        _ => PackState::InPlaceChanged(dir),
                    }
                }
                _ => match store.inspect(&pack.digest)? {
                    TreeState::Absent => PackState::NotInstalled,
                    TreeState::Intact(dir) => PackState::Recorded(dir),
                    TreeState::Tampered(dir) => PackState::Tampered(dir),
                },
            };
            Ok(PackStatus {
                id: pack.id.clone(),
                version: pack.version.clone(),
                state,
            })
        })
        .collect()
}

/// The CI check: the resolved packs must be exactly the lock (policies.md N7).
/// Pure; the lock file is not touched.
///
/// # Errors
/// [`SyncError::LockMismatch`] with the differences.
pub fn check_lock(lock: &Lockfile, resolved: &[ResolvedPack]) -> Result<(), SyncError> {
    let delta = diff(lock, resolved);
    if delta.is_empty() {
        Ok(())
    } else {
        Err(SyncError::LockMismatch(delta))
    }
}

/// One line per locked pack that has a newer version available, for the
/// plugin-update notice. Pure.
///
/// A version inside the config range is `update`d; a newer one outside it
/// needs a range change first.
#[must_use]
pub fn update_notices(
    lock: &Lockfile,
    ranges: &BTreeMap<PackId, VersionReq>,
    available: &BTreeMap<PackId, Vec<Version>>,
) -> Vec<String> {
    lock_outdated(lock, ranges, available)
        .into_iter()
        .flat_map(|o| {
            let mut lines = Vec::new();
            if let Some(v) = &o.update {
                lines.push(format!(
                    "pack {}: {} is available (locked {}); run `slopvac plugins update`",
                    o.id, v, o.locked
                ));
            }
            if let Some(v) = o.newest.as_ref().filter(|v| Some(*v) != o.update.as_ref()) {
                lines.push(format!(
                    "pack {}: {} is available but outside the configured range; change the range to use it",
                    o.id, v
                ));
            }
            lines
        })
        .collect()
}

/// The semver tags of a repository, ascending. Tags that are not versions are
/// ignored. The caller must be online; this is the input of [`update_notices`].
///
/// # Errors
/// The registry error.
pub fn available_versions(
    client: &Client,
    repository: &Reference,
) -> Result<Vec<Version>, OciError> {
    let mut versions: Vec<Version> = client
        .list_tags(repository)?
        .iter()
        .filter_map(|t| Version::parse(t).ok())
        .filter(|v| v.build.is_empty())
        .collect();
    versions.sort();
    versions.dedup();
    Ok(versions)
}
