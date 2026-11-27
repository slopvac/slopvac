//! `slopvac plugins sync`, `status` and the CI lock check (contract:
//! `docs/contract/plugins.md` section 7, threat model sections 5 and 6.5).
//!
//! This module is a library. The CLI (`slopvac-8l4.12`) picks the data
//! directory and the network mode, builds a [`Syncer`] and prints the result.
//!
//! # What sync does
//!
//! `sync` installs exactly the lock, nothing newer. For each entry it:
//!
//! 1. resolves the source: an OCI reference (fetched by the lock's manifest
//!    digest, never by tag), an `https://` archive, a `git+https://` repository
//!    pinned to a full commit, or a path in the project;
//! 2. checks the digest the lock pins before anything is trusted. For git the
//!    digest is the tree hash of the commit's files, computed from an export
//!    without a checkout by a `git` subprocess with a scrubbed environment (no
//!    hooks, filters, links or submodules);
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
//! [`SyncError::OfflinePackMissing`] (`offline-plugin-missing`), naming the pack
//! and the digest. A cached tree that no longer matches its record is removed
//! (threat model E8): online it is installed again, offline sync exits 2.
//!
//! A git pack has no archive to cache: the stored tree is the cache, found by
//! its hash (the digest) and hashed again on every use, so a warm cache needs
//! neither the network nor `git`.
//!
//! Sync never trusts a record to say which tree belongs to a digest: it
//! extracts again from the verified blob and records the resulting tree hash.
//! Whatever loads a pack at run time recomputes the tree hash itself.
//!
//! Concurrent syncs into one store are safe for extraction (atomic renames) but
//! may both download a blob. Locking is not implemented.

mod cache;
mod error;
mod git;
mod verified;

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use semver::{Version, VersionReq};

use cache::Accepted;
pub use cache::{Store, StoreLock, TreeState};
pub use error::{CacheCause, SyncError};
use verified::Cached;
pub use verified::VerifiedTree;

use crate::extract::{
    ExtractError, Limits, StagedTree, TreeHash, extract_targz, tree_hash_capturing,
    verify_tree_capturing,
};
use crate::launch::Launcher;
use crate::lock::{
    LockedPack, Lockfile, PackId, ResolvedPack, SourceKind, diff, outdated as lock_outdated,
};
use crate::messages::sync::{self as messages, SyncPhrase};
use crate::oci::{
    Client, Descriptor, Digest, MAX_MANIFEST_BYTES, MEDIA_MANIFEST, Manifest, OciError, Reference,
    Target,
};
use crate::pyenv::{EnvStore, InstallNetwork, Installer, MANIFEST_FILE, PythonRuntime};
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
/// Longest the git commands of one pack may take together.
const GIT_TIMEOUT: Duration = Duration::from_secs(300);

/// The one layer of a pack manifest that holds the pack, or the sentence that says why it has
/// none. The signature layers do not count.
fn payload_layer<'m>(
    manifest: &'m Manifest,
    limits: &Limits,
) -> Result<&'m Descriptor, &'static str> {
    let mut payload = manifest
        .layers
        .iter()
        .filter(|l| l.media_type != MEDIA_STATEMENT && l.media_type != MEDIA_MINISIG);
    let (Some(layer), None) = (payload.next(), payload.next()) else {
        return Err(SyncPhrase::OneLayer.as_static());
    };
    if layer.media_type != MEDIA_PACK_LAYER {
        return Err(SyncPhrase::LayerNotArchive.as_static());
    }
    if layer.size > limits.compressed_bytes {
        return Err(SyncPhrase::LayerTooLarge.as_static());
    }
    Ok(layer)
}

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
    /// A cached file of this plugin was not what it must be (over its size cap, or hashing to
    /// another digest), was removed and was fetched again. Only sync repairs the cache.
    pub repaired: bool,
    /// Store key of the managed Python environment for this pack, when its
    /// manifest selects `runtime = "python"` and the syncer builds environments
    /// ([`Syncer::with_python`]).
    pub python_env: Option<String>,
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
    /// A verified archive in the blob cache, and the digest its bytes must have. The install
    /// reads the file once more and checks those bytes, and extracts only what it read.
    Archive(PathBuf, Digest),
    /// A tree fetched from a pinned git commit, hashed and equal to the lock,
    /// not yet in the store.
    Staged(Box<StagedTree>, Accepted),
    /// A tree already in the store whose hash is the locked digest, and its manifest as the
    /// pass that checked the hash read it.
    Stored(PathBuf, Option<Vec<u8>>),
    /// A verified directory in the project, and its manifest as the pass that checked the hash
    /// read it.
    InPlace(PathBuf, Option<Vec<u8>>),
}

/// Where and how sync builds managed Python environments (threat model
/// section 6.6). Offline mode builds offline too: uv downloads nothing and
/// uses only interpreters that are already installed.
#[derive(Debug, Clone, Copy)]
pub struct PythonBuild<'a> {
    /// The store the environments go into.
    pub envs: &'a EnvStore,
    /// Absolute path of the `uv` binary.
    pub uv: &'a Path,
    /// Where uv keeps managed interpreters (`UV_PYTHON_INSTALL_DIR`).
    pub python_install_dir: &'a Path,
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
    git: GitSettings,
    python: Option<PythonBuild<'a>>,
}

/// How git sources are fetched. Production uses the defaults; tests may allow
/// a local repository in place of the locked HTTPS URL.
struct GitSettings {
    timeout: Duration,
    memory: u64,
    launcher: Launcher,
    /// `(locked URL prefix, replacement prefix)`, tests only.
    rewrite: Option<(String, String)>,
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
            git: GitSettings {
                timeout: GIT_TIMEOUT,
                memory: git::GIT_MEMORY_BYTES,
                launcher: Launcher::CurrentExe,
                rewrite: None,
            },
            python: None,
        }
    }

    /// Test seam: fetch a git source whose locked URL starts with `from` from
    /// the URL that starts with `to` instead, and allow `file://` there.
    ///
    /// Production code never calls this: locks accept only `git+https://`
    /// sources, and nothing in a lock or in configuration can switch a local
    /// transport on.
    #[doc(hidden)]
    #[must_use]
    pub fn with_git_rewrite_for_tests(mut self, from: &str, to: &str) -> Self {
        self.git.rewrite = Some((from.to_owned(), to.to_owned()));
        self
    }

    /// The program that runs `git` under resource limits. The default is the
    /// running executable, which must call [`crate::launch::intercept`] first
    /// thing in `main`; a test points this at the package's launcher binary.
    #[must_use]
    pub fn with_launcher(mut self, launcher: Launcher) -> Self {
        self.git.launcher = launcher;
        self
    }

    /// Test seam: a lower memory limit for `git`.
    #[doc(hidden)]
    #[must_use]
    pub fn with_git_memory_for_tests(mut self, bytes: u64) -> Self {
        self.git.memory = bytes;
        self
    }

    /// Test seam: a shorter time limit for the git commands of one pack.
    #[doc(hidden)]
    #[must_use]
    pub fn with_git_timeout_for_tests(mut self, timeout: Duration) -> Self {
        self.git.timeout = timeout;
        self
    }

    /// Build the managed Python environment of every pack whose manifest
    /// selects `runtime = "python"`, after the pack is extracted and
    /// verified. An environment that still passes its quick check is kept.
    /// A failed build fails the sync ([`SyncError::PythonEnv`]).
    #[must_use]
    pub fn with_python(mut self, python: PythonBuild<'a>) -> Self {
        self.python = Some(python);
        self
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
        self.check_git_launcher(lock)?;
        // A repair is reported by the run that made it: marks of an earlier run are dropped.
        self.store.clear_repaired();
        let current = match self.network {
            Network::Online(client) => self.revocations.refresh(client)?,
            Network::Offline => self.revocations.cached()?,
        };
        let revoked = &current.list;
        for pack in lock.packs() {
            if revoked.revokes_digest(pack.digest.as_str()) {
                return Err(SyncError::Revoked {
                    id: pack.id.clone(),
                    what: messages::revoked_artifact(&pack.digest),
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
            if matches!(how, Acquired::Archive(..) | Acquired::Staged(..)) {
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
            let (mut outcome, accepted) = self.install(pack, how, &before)?;
            outcome.python_env = self.python_env(pack, &outcome.tree, &accepted)?;
            outcome.repaired = self.store.take_repaired(&pack.id);
            // A modified tree too large to delete was moved aside; keep reporting it.
            outcome.quarantined = outcome.quarantined.or(aside);
            report.packs.push(outcome);
        }
        Ok(report)
    }

    fn python_env(
        &self,
        pack: &LockedPack,
        tree: &Path,
        accepted: &Accepted,
    ) -> Result<Option<String>, SyncError> {
        let Some(build) = self.python else {
            return Ok(None);
        };
        let fail = |source| SyncError::PythonEnv {
            id: pack.id.clone(),
            source,
        };
        // The hash and the manifest come from the pass that accepted the tree: the runtime is
        // selected from the bytes that were accepted, and no second pass reads the tree.
        let Accepted { hash, manifest } = accepted;
        let Some(manifest) = manifest else {
            return Ok(None);
        };
        let Some(runtime) = PythonRuntime::from_manifest(tree, manifest).map_err(fail)? else {
            return Ok(None);
        };
        let installer = Installer {
            uv: build.uv.to_path_buf(),
            python_install_dir: build.python_install_dir.to_path_buf(),
            python: runtime.python,
            network: match self.network {
                Network::Offline => InstallNetwork::Offline,
                Network::Online(_) => InstallNetwork::Online,
            },
        };
        build.envs.ensure(&installer, tree, hash).map_err(fail)?;
        Ok(Some(EnvStore::key(hash, &installer.python)))
    }

    fn install(
        &self,
        pack: &LockedPack,
        how: Acquired,
        before: &TreeState,
    ) -> Result<(PackOutcome, Accepted), SyncError> {
        let outcome = |change, tree, quarantined| PackOutcome {
            id: pack.id.clone(),
            version: pack.version.clone(),
            change,
            tree,
            quarantined,
            repaired: false,
            python_env: None,
        };
        let extract = |source| SyncError::Extract {
            id: pack.id.clone(),
            source,
        };
        let blob = match how {
            Acquired::InPlace(dir, manifest) => {
                let hash = Self::locked_tree_hash(pack)?;
                return Ok((
                    outcome(Change::InPlace, dir, None),
                    Accepted { hash, manifest },
                ));
            }
            Acquired::Stored(dir, manifest) => {
                // The tree hash is the digest: link the digest to it again in case
                // the record was lost.
                let hash = Self::locked_tree_hash(pack)?;
                self.store.record(&pack.digest, &hash)?;
                return Ok((
                    outcome(Change::Unchanged, dir, None),
                    Accepted { hash, manifest },
                ));
            }
            Acquired::Staged(staged, accepted) => {
                // The tree is hashed again to seal it. That hash must be the accepted one, and
                // the manifest is the one the accepting pass read.
                let done = staged
                    .install(&self.store.trees(), &accepted.hash)
                    .map_err(extract)?;
                self.store.record(&pack.digest, &done.hash)?;
                let change = match before {
                    TreeState::Tampered(_) => Change::Reinstalled,
                    _ => Change::Installed,
                };
                return Ok((outcome(change, done.root, done.quarantined), accepted));
            }
            Acquired::Archive(_, archive) => archive,
        };
        // The archive is read once, here, and extracted from the bytes that were checked: a file
        // swapped after the earlier check cannot reach the tree.
        let bytes = self.store.read_archive(&pack.id, &blob, self.limits.compressed_bytes)?;
        let exec = self.executables.get(&pack.id).cloned().unwrap_or_default();
        let done = extract_targz(bytes.as_slice(), &self.store.trees(), &exec, self.limits)
            .map_err(extract)?;
        #[cfg(feature = "test-support")]
        self.store.observe(&done.root);
        self.store.record(&pack.digest, &done.hash)?;
        let change = match before {
            TreeState::Intact(dir) if *dir == done.root => Change::Unchanged,
            TreeState::Tampered(_) => Change::Reinstalled,
            _ => Change::Installed,
        };
        Ok((
            outcome(change, done.root, done.quarantined),
            Accepted {
                hash: done.hash,
                manifest: done.manifest,
            },
        ))
    }

    fn acquire(&self, pack: &LockedPack, revoked: &RevocationList) -> Result<Acquired, SyncError> {
        match pack.source.kind() {
            SourceKind::Oci(text) => self.acquire_oci(pack, text, revoked),
            SourceKind::Archive(url) => self.acquire_archive(pack, url),
            SourceKind::Path(rel) => self.acquire_path(pack, rel),
            SourceKind::Git { url, commit } => self.acquire_git(pack, url, commit),
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

    /// The reference, the manifest and the namespace of an OCI entry: the manifest comes by the
    /// locked digest, from the cache when it holds it. An unsigned entry costs no request.
    fn oci_manifest(
        &self,
        pack: &LockedPack,
        text: &str,
        may_fetch: bool,
    ) -> Result<(Reference, Manifest, String), SyncError> {
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

        let body = self.manifest_bytes(pack, &reference, &locked, may_fetch)?;
        let manifest: Manifest =
            serde_json::from_slice(&body).map_err(|e| layout(&messages::manifest_parse(&e)))?;
        if manifest
            .media_type
            .as_deref()
            .is_some_and(|m| m != MEDIA_MANIFEST)
        {
            return Err(layout(SyncPhrase::ManifestNotImage.as_static()));
        }

        Ok((reference, manifest, namespace))
    }

    /// Check the signature of a cached OCI entry again. The cache is not trusted (threat model
    /// E8), so `plugins trust` calls this before it records trust: a lock edited to name a
    /// planted artifact has no signature that sync ever checked.
    ///
    /// It reads the cache and nothing else, whatever the network mode of this syncer: no
    /// request, and no write into the cache. Each cached file is read once, checked against its
    /// digest in memory, and the check uses those bytes. Returns the key id of the signature
    /// that verified, or `None` for an entry from a source that has no signature (a directory,
    /// an archive or a git commit).
    ///
    /// # Errors
    /// [`SyncError::Unsigned`] when the lock records no signature for an OCI entry,
    /// [`SyncError::OfflinePackMissing`] when the cache lacks the manifest or a signature file,
    /// and [`SyncError::Sign`] or [`SyncError::SignatureDiffersFromLock`] when the signature
    /// does not verify.
    pub fn verify_cached_signature(&self, pack: &LockedPack) -> Result<Option<String>, SyncError> {
        let SourceKind::Oci(text) = pack.source.kind() else {
            return Ok(None);
        };
        let (reference, manifest, namespace) = self.oci_manifest(pack, text, false)?;
        self.verify_signature(pack, &reference, &manifest, &namespace, false)
            .map(Some)
    }

    /// The failure for content that only the network could supply.
    fn missing(pack: &LockedPack, what: &str) -> SyncError {
        SyncError::OfflinePackMissing {
            id: pack.id.clone(),
            digest: pack.digest.to_string(),
            what: what.to_owned(),
        }
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
        let (reference, manifest, namespace) = self.oci_manifest(pack, text, true)?;

        let key_id = self.verify_signature(pack, &reference, &manifest, &namespace, true)?;
        if revoked.revokes_key(&key_id) {
            return Err(SyncError::Revoked {
                id: id.clone(),
                what: messages::revoked_key(&key_id),
            });
        }

        let layer = payload_layer(&manifest, &self.limits).map_err(layout)?;
        if let Some(path) = self.store.blob(&pack.id, &layer.digest, layer.size)? {
            return Ok(Acquired::Archive(path, layer.digest.clone()));
        }
        let client = self.client(pack, SyncPhrase::WhatArchive.as_static())?;
        let mut tmp = self.store.blob_tempfile()?;
        client
            .pull_blob(&reference, layer, tmp.as_file_mut())
            .map_err(fetch)?;
        Ok(Acquired::Archive(
            self.store.commit_blob(tmp, &layer.digest)?,
            layer.digest.clone(),
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
        fetch: bool,
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
            SyncPhrase::WhatStatement.as_static(),
            fetch,
        )?;
        let minisig_bytes = self.small_blob(
            pack,
            reference,
            minisig,
            MAX_MINISIG_BYTES,
            SyncPhrase::WhatSignature.as_static(),
            fetch,
        )?;
        let minisig_text = String::from_utf8(minisig_bytes)
            .map_err(|_| layout(SyncPhrase::SignatureNotText.as_static()))?;
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
            return Err(differs(messages::signed_key(
                &verified.key().key_id(),
                &lock_signature.key_id,
            )));
        }
        if verified.statement().not_before != lock_signature.not_before.as_str() {
            return Err(differs(messages::signed_not_before(
                &verified.statement().not_before,
                &lock_signature.not_before,
            )));
        }

        Ok(verified.key().key_id())
    }

    /// The cached bytes of a small blob, or `None` when it is absent or, under `repair`, was
    /// removed because it was wrong. Only sync repairs (`repair`): a blob that is over `max` or
    /// not what its name says is removed, so the caller fetches it again, and the repair is
    /// recorded. Inspection (`repair` false) deletes nothing: a wrong digest is
    /// [`SyncError::DigestMismatch`], and any other failure is [`SyncError::CacheUnreadable`].
    fn cached_small(
        &self,
        pack: &LockedPack,
        digest: &Digest,
        max: u64,
        repair: bool,
    ) -> Result<Option<Vec<u8>>, SyncError> {
        match self.store.read_cached(digest, max) {
            Ok(Cached::Bytes(bytes)) => Ok(Some(bytes)),
            Ok(Cached::Absent) => Ok(None),
            Ok(Cached::Mismatch(_)) if repair => {
                self.store.evict_blob(&pack.id, digest).map(|()| None)
            }
            Ok(Cached::Mismatch(actual)) => Err(SyncError::DigestMismatch {
                id: pack.id.clone(),
                expected: digest.to_string(),
                actual: actual.to_string(),
            }),
            // Slopvac never writes a cached file over its cap, so an oversize file is as wrong
            // as one with the wrong digest.
            Err(CacheCause::TooLarge { .. }) if repair => {
                self.store.evict_blob(&pack.id, digest).map(|()| None)
            }
            Err(cause) => Err(self.store.unreadable(&pack.id, digest, cause)),
        }
    }

    fn manifest_bytes(
        &self,
        pack: &LockedPack,
        reference: &Reference,
        locked: &Digest,
        fetch: bool,
    ) -> Result<Vec<u8>, SyncError> {
        if let Some(bytes) = self.cached_small(pack, locked, MAX_MANIFEST_BYTES as u64, fetch)? {
            return Ok(bytes);
        }
        if !fetch {
            return Err(Self::missing(pack, SyncPhrase::WhatManifest.as_static()));
        }
        let client = self.client(pack, SyncPhrase::WhatManifest.as_static())?;
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
        fetch: bool,
    ) -> Result<Vec<u8>, SyncError> {
        if let Some(bytes) = self.cached_small(pack, &desc.digest, max, fetch)? {
            return Ok(bytes);
        }
        if !fetch {
            return Err(Self::missing(pack, what));
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

    fn locked_tree_hash(pack: &LockedPack) -> Result<TreeHash, SyncError> {
        TreeHash::parse(pack.digest.as_str()).map_err(|e| SyncError::Source {
            id: pack.id.clone(),
            why: e.to_string(),
        })
    }

    /// The launcher path for a git fetch of plugin `id`.
    fn launcher_for(&self, id: &PackId) -> Result<PathBuf, SyncError> {
        self.git.launcher.path().map_err(|e| match e {
            crate::launch::LaunchError::NotInstalled => SyncError::Launcher {
                id: id.clone(),
                source: crate::PluginError::LauncherNotInstalled,
            },
            e @ crate::launch::LaunchError::CurrentExe(_) => SyncError::Git {
                id: id.clone(),
                why: messages::launcher_missing(&e),
            },
        })
    }

    /// Refuse before any side effect when a git pack must be fetched and the
    /// launcher cannot start.
    fn check_git_launcher(&self, lock: &Lockfile) -> Result<(), SyncError> {
        for pack in lock.packs() {
            if let SourceKind::Git { .. } = pack.source.kind() {
                let expected = Self::locked_tree_hash(pack)?;
                if fs::symlink_metadata(self.store.tree_dir(&expected)).is_err() {
                    self.launcher_for(&pack.id)?;
                }
            }
        }
        Ok(())
    }

    /// A git source: the digest is the hash of the tree at the pinned commit.
    /// A stored tree with that hash is used as is (offline too); otherwise the
    /// commit is fetched, exported without a checkout and hashed. The tree
    /// reaches the store only in the install step, after every pack verified.
    fn acquire_git(
        &self,
        pack: &LockedPack,
        url: &str,
        commit: &str,
    ) -> Result<Acquired, SyncError> {
        let id = &pack.id;
        let extract = |source| SyncError::Extract {
            id: id.clone(),
            source,
        };
        let expected = Self::locked_tree_hash(pack)?;
        let dir = self.store.tree_dir(&expected);
        if fs::symlink_metadata(&dir).is_ok() {
            let verified = verify_tree_capturing(&dir, &expected);
            #[cfg(feature = "test-support")]
            self.store.observe(&dir);
            match verified {
                Ok(manifest) => return Ok(Acquired::Stored(dir, manifest)),
                // A stored tree the walk refuses is repaired like one that changed.
                Err(ExtractError::Tampered { .. } | ExtractError::Refused(_)) => {
                    if matches!(self.network, Network::Offline) {
                        let aside = cache::evict_tree(&dir)?;
                        return Err(SyncError::CacheTampered {
                            id: id.clone(),
                            path: aside.unwrap_or(dir),
                        });
                    }
                }
                Err(source) => return Err(extract(source)),
            }
        }
        self.client(pack, SyncPhrase::WhatGitTree.as_static())?;

        let (url, allow_local) = match &self.git.rewrite {
            Some((from, to)) if url.starts_with(from.as_str()) => {
                (format!("{to}{}", &url[from.len()..]), true)
            }
            _ => (url.to_owned(), false),
        };
        let git_failed = |why: String| SyncError::Git {
            id: id.clone(),
            why,
        };
        let launcher = self.launcher_for(id)?;
        let program = git::find_git(self.project_root)
            .ok_or_else(|| git_failed(SyncPhrase::GitNotFound.as_static().into()))?;
        let scratch = self.store.scratch_dir()?;
        let mut staged = StagedTree::new(&self.store.trees(), self.limits).map_err(extract)?;
        let request = git::Request {
            git: &program,
            url: &url,
            commit,
            allow_local,
            timeout: self.git.timeout,
            limits: self.limits,
            confinement: Some(git::Confinement {
                launcher: &launcher,
                memory: self.git.memory,
            }),
        };
        git::export_commit(&request, scratch.path(), &mut staged).map_err(|e| match e {
            git::GitError::Failed(msg) => git_failed(msg),
            git::GitError::Limit(why) => SyncError::GitLimit {
                id: id.clone(),
                why,
            },
            git::GitError::Extract(source) => extract(source),
        })?;
        let (actual, manifest) = staged.hash_capturing().map_err(extract)?;
        if actual != expected {
            return Err(SyncError::DigestMismatch {
                id: id.clone(),
                expected: expected.to_string(),
                actual: actual.to_string(),
            });
        }
        #[cfg(feature = "test-support")]
        self.store.observe(staged.root());
        Ok(Acquired::Staged(
            Box::new(staged),
            Accepted {
                hash: actual,
                manifest,
            },
        ))
    }

    fn acquire_archive(&self, pack: &LockedPack, url: &str) -> Result<Acquired, SyncError> {
        let locked = Self::locked_digest(pack)?;
        if let Some(path) = self
            .store
            .blob(&pack.id, &locked, self.limits.compressed_bytes)?
        {
            return Ok(Acquired::Archive(path, locked));
        }
        let client = self.client(pack, SyncPhrase::WhatRelease.as_static())?;
        let mut tmp = self.store.blob_tempfile()?;
        client
            .download(url, self.limits.compressed_bytes, tmp.as_file_mut())
            .map_err(|source| SyncError::Fetch {
                id: pack.id.clone(),
                source,
            })?;
        let actual = cache_hash(tmp.path()).map_err(|e| SyncError::Store(messages::hashing(&e)))?;
        if actual != locked {
            return Err(SyncError::DigestMismatch {
                id: pack.id.clone(),
                expected: locked.to_string(),
                actual: actual.to_string(),
            });
        }
        Ok(Acquired::Archive(
            self.store.commit_blob(tmp, &locked)?,
            locked,
        ))
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
            let meta =
                fs::symlink_metadata(&path).map_err(|e| source(messages::path_io(rel, &e)))?;
            if meta.file_type().is_symlink() {
                return Err(source(messages::path_link(rel)));
            }
        }
        // Defence in depth after the segment walk: the resolved path must stay inside
        // the project root, whatever the platform makes of the segments.
        let inside = fs::canonicalize(self.project_root)
            .and_then(|root| fs::canonicalize(&path).map(|p| p.starts_with(root)))
            .map_err(|e| source(messages::path_io(rel, &e)))?;
        if !inside {
            return Err(source(messages::path_outside(rel)));
        }
        let mismatch = |actual: &str| SyncError::DigestMismatch {
            id: id.clone(),
            expected: pack.digest.to_string(),
            actual: actual.to_owned(),
        };
        if fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
            let (hash, manifest) =
                tree_hash_capturing(&path, MANIFEST_FILE).map_err(|source| SyncError::Extract {
                    id: id.clone(),
                    source,
                })?;
            if hash.as_str() != pack.digest.as_str() {
                return Err(mismatch(hash.as_str()));
            }
            #[cfg(feature = "test-support")]
            self.store.observe(&path);
            return Ok(Acquired::InPlace(path, manifest));
        }
        let locked = Self::locked_digest(pack)?;
        if let Some(cached) = self
            .store
            .blob(&pack.id, &locked, self.limits.compressed_bytes)?
        {
            // Still check the file in the project: it is what the lock names.
            let actual = cache_hash(&path).map_err(|e| source(messages::path_reading(rel, &e)))?;
            if actual != locked {
                return Err(mismatch(actual.as_str()));
            }
            return Ok(Acquired::Archive(cached, locked));
        }
        let mut tmp = self.store.blob_tempfile()?;
        let mut file = File::open(&path).map_err(|e| source(messages::path_reading(rel, &e)))?;
        let mut limited = (&mut file).take(self.limits.compressed_bytes + 1);
        let copied = io::copy(&mut limited, tmp.as_file_mut())
            .map_err(|e| source(messages::path_reading(rel, &e)))?;
        if copied > self.limits.compressed_bytes {
            return Err(source(messages::path_too_large(rel)));
        }
        let actual = cache_hash(tmp.path()).map_err(|e| SyncError::Store(messages::hashing(&e)))?;
        if actual != locked {
            return Err(mismatch(actual.as_str()));
        }
        Ok(Acquired::Archive(
            self.store.commit_blob(tmp, &locked)?,
            locked,
        ))
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

/// How the managed Python environment of a pack stands. `status` leaves it at
/// [`PythonState::NotChecked`]; [`status_with_python`] fills it in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PythonState {
    /// Not looked at: the caller passed no environment store, the pack is not
    /// a Python plugin, or its tree is not intact.
    NotChecked,
    /// The environment exists and passes the full check, which hashes every
    /// file and the interpreter (threat model Y5). Carries its store key.
    Verified(String),
    /// The pack needs an environment and none is built. `sync` builds it.
    Missing,
    /// The environment fails the full check, or the manifest cannot select
    /// one. Spawning is refused until it is rebuilt and trusted again.
    Failed(String),
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
    /// State of its managed Python environment.
    pub python: PythonState,
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
/// [`SyncError::Extract`] with the typed cause when a tree cannot be read or hashed: a walk that
/// is refused, a manifest over the capture limit or a failed read. A tree whose finished hash
/// differs from the lock or its record is reported in the state, not as an error.
pub fn status(
    store: &Store,
    project_root: &Path,
    lock: &Lockfile,
) -> Result<Vec<PackStatus>, SyncError> {
    status_in(store, None, project_root, lock)
}

/// [`status`], plus the full check of each managed Python environment
/// (`slopvac plugins status` and `doctor`, threat model section 6.6). For an
/// intact pack whose manifest selects `runtime = "python"`, this loads the
/// environment from `envs` and hashes every file in it and the interpreter.
/// The hashing is the cost that the spawn gate avoids (Y5). Environments
/// that no pack uses are listed by [`EnvStore::list`].
///
/// # Errors
/// As [`status`]: [`SyncError::Extract`] with the typed cause when a tree cannot be read or
/// hashed.
pub fn status_with_python(
    store: &Store,
    envs: &EnvStore,
    project_root: &Path,
    lock: &Lockfile,
) -> Result<Vec<PackStatus>, SyncError> {
    status_in(store, Some(envs), project_root, lock)
}

/// The state of the environment of an accepted tree. The hash and the manifest are those of the
/// pass that accepted the tree; nothing here reads the tree again.
fn python_state(envs: &EnvStore, tree: &Path, accepted: &Accepted) -> PythonState {
    let Accepted { hash, manifest } = accepted;
    let Some(manifest) = manifest else {
        return PythonState::NotChecked;
    };
    let runtime = match PythonRuntime::from_manifest(tree, manifest) {
        Ok(None) => return PythonState::NotChecked,
        Ok(Some(r)) => r,
        Err(e) => return PythonState::Failed(e.to_string()),
    };
    match envs.load(hash, &runtime.python) {
        Ok(None) => PythonState::Missing,
        Ok(Some(env)) => match env.verify_full() {
            Ok(()) => PythonState::Verified(EnvStore::key(hash, &runtime.python)),
            Err(e) => PythonState::Failed(e.to_string()),
        },
        Err(e) => PythonState::Failed(e.to_string()),
    }
}

fn status_in(
    store: &Store,
    envs: Option<&EnvStore>,
    project_root: &Path,
    lock: &Lockfile,
) -> Result<Vec<PackStatus>, SyncError> {
    lock.packs()
        .iter()
        .map(|pack| {
            let mut accepted = None;
            let state = match pack.source.kind() {
                SourceKind::Path(rel) if project_root.join(rel).is_dir() => {
                    let dir = project_root.join(rel);
                    let passed = tree_hash_capturing(&dir, MANIFEST_FILE);
                    #[cfg(feature = "test-support")]
                    store.observe(&dir);
                    // Only a hash that differs from the lock is a change. A pass that could not
                    // finish keeps its typed cause.
                    let (hash, manifest) = passed.map_err(|source| SyncError::Extract {
                        id: pack.id.clone(),
                        source,
                    })?;
                    if hash.as_str() == pack.digest.as_str() {
                        accepted = Some(Accepted { hash, manifest });
                        PackState::InPlace(dir)
                    } else {
                        PackState::InPlaceChanged(dir)
                    }
                }
                _ => match store.inspect_accepted(&pack.digest).map_err(|source| {
                    SyncError::Extract {
                        id: pack.id.clone(),
                        source,
                    }
                })? {
                    (TreeState::Absent, _) => PackState::NotInstalled,
                    (TreeState::Intact(dir), seen) => {
                        accepted = seen;
                        PackState::Recorded(dir)
                    }
                    (TreeState::Tampered(dir), _) => PackState::Tampered(dir),
                },
            };
            let python = match (&state, envs, &accepted) {
                (PackState::Recorded(dir) | PackState::InPlace(dir), Some(envs), Some(seen)) => {
                    python_state(envs, dir, seen)
                }
                _ => PythonState::NotChecked,
            };
            Ok(PackStatus {
                id: pack.id.clone(),
                version: pack.version.clone(),
                state,
                python,
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
                lines.push(messages::notice_update(&o.id, v, &o.locked));
            }
            if let Some(v) = o.newest.as_ref().filter(|v| Some(*v) != o.update.as_ref()) {
                lines.push(messages::notice_outside(&o.id, v));
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
