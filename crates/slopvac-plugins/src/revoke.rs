//! The signed revocation list and its cache (threat model section 5.3, K5, K6).
//!
//! A publisher whose key is stolen cannot sign its own revocation, so the
//! first-party root key signs one list for everyone. It names publisher key
//! ids and artifact manifest digests. A key or digest on the list fails `sync`
//! with exit 2 and refuses to spawn; trust records for it are inert, and
//! [`RevocationList::flag`] marks them in `plugins status`.
//!
//! # Format
//!
//! The list is strict canonical JSON (sorted keys, no white space, at most
//! 1 MiB), signed by a root key in minisign's prehashed mode:
//!
//! ```json
//! {"digests":["sha256:..."],"issued":"2026-11-12T00:00:00Z","keys":["8384841ACDC75F77"],"schema_version":1,"type":"revocations"}
//! ```
//!
//! `keys` holds 16-digit uppercase key ids and `digests` holds manifest
//! digests, each sorted and without repeats, so one list has one encoding.
//! `issued` orders lists: a fetched list older than the cached one is a
//! rollback and is refused.
//!
//! # Where it comes from
//!
//! The list is an OCI artifact of type [`ARTIFACT_TYPE_REVOCATIONS`] with two
//! layers, the list ([`MEDIA_REVOCATIONS`]) and its `.minisig`
//! ([`crate::sign::MEDIA_MINISIG`]). [`DEFAULT_SOURCE`] is the provisional
//! location. `sync` fetches it on every online run ([`Revocations::refresh`]),
//! and so must the CLI's `update` and `outdated`.
//!
//! # Cache and offline use
//!
//! A verified list is cached with the time it was fetched, in one private file
//! in the per-user state directory. Every read verifies the signature again,
//! so an edited cache fails closed ([`RevocationError::CacheInvalid`]): spawn
//! and offline sync exit 2 until an online refresh replaces it. Offline runs
//! and every spawn read the cache ([`Revocations::cached`]). A cache fetched
//! more than [`STALE_AFTER_DAYS`] days ago prints a notice and does not block;
//! a machine that never fetched one runs with a notice too (residual risk R3).
//! The fetch time is not signed: a local process of the same user can change
//! it, or put back an older signed list (R6).

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::oci::{Client, Digest, OciError, Reference};
use crate::sign::{
    MEDIA_MINISIG, PublicKey, SignError, is_key_id, parse_canonical, utc_seconds, verify_bytes,
};
use crate::trust::{Revocation, TrustRecord};

/// `artifactType` of the revocation-list artifact.
pub const ARTIFACT_TYPE_REVOCATIONS: &str = "application/vnd.slopvac.revocations.v1";
/// Media type of the layer that holds the list.
pub const MEDIA_REVOCATIONS: &str = "application/vnd.slopvac.revocations.v1+json";
/// The `type` field of a list.
pub const LIST_TYPE: &str = "revocations";
/// Where the list is published. Provisional until the release process
/// publishes it (`slopvac-8l4.9.15` PR asks).
pub const DEFAULT_SOURCE: &str = "ghcr.io/srobroek/slopvac-revocations:latest";
/// Largest list the verifier reads.
pub const MAX_LIST_BYTES: usize = 1 << 20;
/// A cache fetched longer ago than this prints a notice (threat model R3).
pub const STALE_AFTER_DAYS: i64 = 30;

const MAX_MINISIG_BYTES: u64 = 4 * 1024;
const CACHE_FILE: &str = "revocations.json";
const MAX_CACHE_BYTES: u64 = 3 << 20;

/// Why the list could not be used. Every variant makes the command exit 2.
#[derive(Debug, Error)]
pub enum RevocationError {
    /// No root key is configured, so no list can verify. Fails closed.
    #[error("no first-party root key is configured to verify the revocation list")]
    NoRootKey,
    /// The list is not a valid version 1 list.
    #[error("the revocation list is invalid: {0}")]
    Invalid(String),
    /// No root key signed these bytes.
    #[error("the revocation list is not signed by the first-party root key")]
    BadSignature,
    /// The registry or the network failed, or withheld the list.
    #[error("the revocation list could not be fetched: {0}")]
    Fetch(#[from] OciError),
    /// The artifact does not have the shape of a list artifact.
    #[error("the revocation list artifact is malformed: {0}")]
    Layout(String),
    /// The fetched list is older than the cached one.
    #[error(
        "the fetched revocation list (issued {fetched}) is older than the cached one (issued {cached})"
    )]
    Rollback {
        /// `issued` of the cached list.
        cached: String,
        /// `issued` of the fetched list.
        fetched: String,
    },
    /// The cache file does not hold a list the root signed.
    #[error("the revocation cache {path} is damaged: {why}; run an online sync to replace it", path = path.display())]
    CacheInvalid {
        /// The cache file.
        path: PathBuf,
        /// What is wrong.
        why: String,
    },
    /// A file operation on the cache failed.
    #[error("revocation cache: {what}: {source}")]
    Io {
        /// The operation.
        what: String,
        /// The cause.
        #[source]
        source: io::Error,
    },
    /// The caller's clock is not a `YYYY-MM-DDTHH:MM:SSZ` time.
    #[error("the current time `{0}` is not YYYY-MM-DDTHH:MM:SSZ")]
    Clock(String),
}

impl RevocationError {
    /// The stable error code (policies.md section 3.5).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoRootKey | Self::Invalid(_) | Self::BadSignature | Self::Layout(_) => {
                "revocation-list-invalid"
            }
            Self::Fetch(_) => "revocation-list-unavailable",
            Self::Rollback { .. } => "revocation-list-rollback",
            Self::CacheInvalid { .. } | Self::Io { .. } => "revocation-cache-invalid",
            Self::Clock(_) => "revocation-clock-invalid",
        }
    }
}

impl From<SignError> for RevocationError {
    fn from(e: SignError) -> Self {
        match e {
            SignError::BadSignature => Self::BadSignature,
            other => Self::Invalid(other.to_string()),
        }
    }
}

/// What a revoked trust record is revoked by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Revoked {
    /// The publisher key with this key id.
    Key(String),
    /// The artifact with this manifest digest.
    Digest(String),
}

impl std::fmt::Display for Revoked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Key(id) => write!(f, "revoked: publisher key {id}"),
            Self::Digest(d) => write!(f, "revoked: artifact {d}"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListDoc {
    schema_version: u32,
    #[serde(rename = "type")]
    kind: String,
    issued: String,
    keys: Vec<String>,
    digests: Vec<String>,
}

/// A verified revocation list. The default is the empty list of a machine
/// that never fetched one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RevocationList {
    issued: Option<String>,
    keys: Vec<String>,
    digests: Vec<String>,
}

fn strictly_sorted(v: &[String]) -> bool {
    v.windows(2).all(|w| w[0] < w[1])
}

impl RevocationList {
    /// Verify `statement` and its `.minisig` text against the root keys.
    ///
    /// # Errors
    /// [`RevocationError::NoRootKey`] with no roots,
    /// [`RevocationError::BadSignature`] unless a root signed exactly these
    /// bytes (prehashed), [`RevocationError::Invalid`] for anything that is not
    /// a canonical version 1 list.
    pub fn verify(
        statement: &[u8],
        minisig: &str,
        roots: &[PublicKey],
    ) -> Result<Self, RevocationError> {
        if roots.is_empty() {
            return Err(RevocationError::NoRootKey);
        }
        if statement.len() > MAX_LIST_BYTES {
            return Err(RevocationError::Invalid(format!(
                "larger than {MAX_LIST_BYTES} bytes"
            )));
        }
        if !roots
            .iter()
            .any(|k| verify_bytes(k, statement, minisig).is_ok())
        {
            return Err(RevocationError::BadSignature);
        }
        let doc: ListDoc = parse_canonical(statement, MAX_LIST_BYTES)?;
        let invalid = |why: &str| Err(RevocationError::Invalid(why.to_owned()));
        if doc.schema_version != 1 || doc.kind != LIST_TYPE {
            return invalid("not a version 1 revocation list");
        }
        if utc_seconds(&doc.issued).is_none() {
            return invalid("issued is not a valid UTC time");
        }
        if !doc.keys.iter().all(|k| is_key_id(k)) {
            return invalid("a key id is not 16 uppercase hex digits");
        }
        if doc.digests.iter().any(|d| Digest::parse(d).is_err()) {
            return invalid("a digest is not sha256 with 64 lowercase hex digits");
        }
        if !strictly_sorted(&doc.keys) || !strictly_sorted(&doc.digests) {
            return invalid("keys and digests must be sorted and unique");
        }
        Ok(Self {
            issued: Some(doc.issued),
            keys: doc.keys,
            digests: doc.digests,
        })
    }

    /// When the root issued the list; `None` for the empty default.
    #[must_use]
    pub fn issued(&self) -> Option<&str> {
        self.issued.as_deref()
    }

    /// Whether the key with this key id is revoked.
    #[must_use]
    pub fn revokes_key(&self, key_id: &str) -> bool {
        self.keys
            .binary_search_by(|k| k.as_str().cmp(&key_id.to_ascii_uppercase()))
            .is_ok()
    }

    /// Whether the artifact with this manifest digest is revoked.
    #[must_use]
    pub fn revokes_digest(&self, digest: &str) -> bool {
        self.digests
            .binary_search_by(|d| d.as_str().cmp(digest))
            .is_ok()
    }

    /// The entries in the form [`crate::trust::TrustGate::with_revoked`] takes.
    #[must_use]
    pub fn revocations(&self) -> Vec<Revocation> {
        let keys = self.keys.iter().cloned().map(Revocation::KeyId);
        let digests = self
            .digests
            .iter()
            .filter_map(|d| Digest::parse(d).ok())
            .map(Revocation::ManifestDigest);
        keys.chain(digests).collect()
    }

    /// Why a trust record is revoked, for `plugins status`. `None` when it is not.
    #[must_use]
    pub fn flag(&self, record: &TrustRecord) -> Option<Revoked> {
        if let Some(d) = record
            .manifest_digest
            .as_deref()
            .filter(|d| self.revokes_digest(d))
        {
            return Some(Revoked::Digest(d.to_owned()));
        }
        record
            .key_id
            .as_deref()
            .filter(|k| self.revokes_key(k))
            .map(|k| Revoked::Key(k.to_ascii_uppercase()))
    }
}

/// A notice about the list. It never blocks the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevocationNotice {
    /// No list was ever fetched on this machine.
    Missing,
    /// The cached list was fetched more than [`STALE_AFTER_DAYS`] days ago.
    Stale {
        /// When it was fetched.
        fetched_at: String,
        /// Whole days since then.
        days: i64,
    },
}

impl RevocationNotice {
    /// The stable notice code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Missing => "revocation-list-missing",
            Self::Stale { .. } => "revocation-list-stale",
        }
    }
}

impl std::fmt::Display for RevocationNotice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => f.write_str(
                "no revocation list has been fetched; run `slopvac plugins sync` online",
            ),
            Self::Stale { fetched_at, days } => write!(
                f,
                "the revocation list was fetched {days} days ago ({fetched_at}); run `slopvac plugins sync` online"
            ),
        }
    }
}

/// The list in force for this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Current {
    /// The verified list, or the empty list when none was ever fetched.
    pub list: RevocationList,
    /// When the list was fetched, if ever.
    pub fetched_at: Option<String>,
    /// Notices to print. They do not block.
    pub notices: Vec<RevocationNotice>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheDoc {
    schema_version: u32,
    fetched_at: String,
    statement: String,
    minisig: String,
}

/// Where the list comes from, where it is cached, which keys verify it and
/// what time it is.
#[derive(Debug, Clone)]
pub struct Revocations {
    dir: PathBuf,
    roots: Vec<PublicKey>,
    source: Reference,
    now: String,
    now_at: i64,
}

impl Revocations {
    // STUB(slopvac-8l4.9.16): the CLI passes the compiled first-party root key
    // (key id D9914BDB4570AC6D, `keys/slopvac-release.pub`) as `roots` once it exists.
    /// `dir` is the per-user state directory. `roots` are the first-party root
    /// keys; with none, every list fails closed. `now` is the caller's clock as
    /// `YYYY-MM-DDTHH:MM:SSZ`.
    ///
    /// # Errors
    /// [`RevocationError::Clock`] for a malformed `now`.
    pub fn new(
        dir: impl Into<PathBuf>,
        roots: Vec<PublicKey>,
        source: Reference,
        now: &str,
    ) -> Result<Self, RevocationError> {
        let now_at = utc_seconds(now).ok_or_else(|| RevocationError::Clock(now.to_owned()))?;
        Ok(Self {
            dir: dir.into(),
            roots,
            source,
            now: now.to_owned(),
            now_at,
        })
    }

    /// The cache file.
    #[must_use]
    pub fn cache_file(&self) -> PathBuf {
        self.dir.join(CACHE_FILE)
    }

    /// The cached list, for offline runs and every spawn. Never uses the network.
    ///
    /// # Errors
    /// [`RevocationError::CacheInvalid`] when the cache exists but does not
    /// hold a list a root key signed.
    pub fn cached(&self) -> Result<Current, RevocationError> {
        let Some((list, fetched_at)) = self.read_cache()? else {
            return Ok(Current {
                list: RevocationList::default(),
                fetched_at: None,
                notices: vec![RevocationNotice::Missing],
            });
        };
        let age = utc_seconds(&fetched_at).map_or(0, |at| self.now_at - at);
        let notices = if age > STALE_AFTER_DAYS * 86_400 {
            vec![RevocationNotice::Stale {
                fetched_at: fetched_at.clone(),
                days: age / 86_400,
            }]
        } else {
            vec![]
        };
        Ok(Current {
            list,
            fetched_at: Some(fetched_at),
            notices,
        })
    }

    /// Fetch, verify and cache the list. A damaged cache is replaced.
    ///
    /// # Errors
    /// [`RevocationError::Fetch`] when the registry fails or withholds the
    /// list, the verification errors of [`RevocationList::verify`],
    /// [`RevocationError::Rollback`] for a list older than the cache, and
    /// cache write failures. The cache is unchanged on any error.
    pub fn refresh(&self, client: &Client) -> Result<Current, RevocationError> {
        let (statement, minisig) = self.fetch(client)?;
        let list = RevocationList::verify(&statement, &minisig, &self.roots)?;
        if let Ok(Some((cached, _))) = self.read_cache()
            && let (Some(old), Some(new)) = (cached.issued(), list.issued())
            && utc_seconds(new) < utc_seconds(old)
        {
            return Err(RevocationError::Rollback {
                cached: old.to_owned(),
                fetched: new.to_owned(),
            });
        }
        let statement = String::from_utf8(statement)
            .map_err(|_| RevocationError::Invalid("the list is not UTF-8".into()))?;
        self.write_cache(&CacheDoc {
            schema_version: 1,
            fetched_at: self.now.clone(),
            statement,
            minisig,
        })?;
        Ok(Current {
            list,
            fetched_at: Some(self.now.clone()),
            notices: vec![],
        })
    }

    fn fetch(&self, client: &Client) -> Result<(Vec<u8>, String), RevocationError> {
        let layout = |why: &str| RevocationError::Layout(why.to_owned());
        let manifest = client.fetch_manifest(&self.source)?.manifest()?;
        if manifest.artifact_type.as_deref() != Some(ARTIFACT_TYPE_REVOCATIONS) {
            return Err(layout("the artifact type is not a revocation list"));
        }
        let [a, b] = manifest.layers.as_slice() else {
            return Err(layout("a list artifact has exactly two layers"));
        };
        let (list, sig) = match (a.media_type.as_str(), b.media_type.as_str()) {
            (MEDIA_REVOCATIONS, MEDIA_MINISIG) => (a, b),
            (MEDIA_MINISIG, MEDIA_REVOCATIONS) => (b, a),
            _ => return Err(layout("the layers are not a list and its signature")),
        };
        let statement = client.fetch_blob_bytes(&self.source, list, MAX_LIST_BYTES as u64)?;
        let sig = client.fetch_blob_bytes(&self.source, sig, MAX_MINISIG_BYTES)?;
        let sig = String::from_utf8(sig).map_err(|_| RevocationError::BadSignature)?;
        Ok((statement, sig))
    }

    fn read_cache(&self) -> Result<Option<(RevocationList, String)>, RevocationError> {
        let path = self.cache_file();
        let invalid = |why: String| RevocationError::CacheInvalid {
            path: path.clone(),
            why,
        };
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(invalid(e.to_string())),
        };
        let mut raw = Vec::new();
        file.take(MAX_CACHE_BYTES + 1)
            .read_to_end(&mut raw)
            .map_err(|e| invalid(e.to_string()))?;
        if raw.len() as u64 > MAX_CACHE_BYTES {
            return Err(invalid("too large".into()));
        }
        let doc: CacheDoc = serde_json::from_slice(&raw).map_err(|e| invalid(e.to_string()))?;
        if doc.schema_version != 1 || utc_seconds(&doc.fetched_at).is_none() {
            return Err(invalid("unsupported schema_version or fetch time".into()));
        }
        let list = RevocationList::verify(doc.statement.as_bytes(), &doc.minisig, &self.roots)
            .map_err(|e| invalid(e.to_string()))?;
        Ok(Some((list, doc.fetched_at)))
    }

    fn write_cache(&self, doc: &CacheDoc) -> Result<(), RevocationError> {
        let io = |what: &str| {
            let what = what.to_owned();
            move |source| RevocationError::Io { what, source }
        };
        create_private_dir(&self.dir).map_err(io("creating the state directory"))?;
        let bytes = serde_json::to_vec(doc).map_err(|e| RevocationError::Io {
            what: "encoding the cache".into(),
            source: io::Error::other(e),
        })?;
        let mut tmp = tempfile::Builder::new()
            .prefix(".revocations-")
            .tempfile_in(&self.dir)
            .map_err(io("creating a temporary cache file"))?;
        set_private_file(tmp.path()).map_err(io("securing the cache file"))?;
        tmp.write_all(&bytes).map_err(io("writing the cache"))?;
        tmp.as_file().sync_all().map_err(io("syncing the cache"))?;
        tmp.persist(self.cache_file())
            .map_err(|e| RevocationError::Io {
                what: "replacing the cache".into(),
                source: e.error,
            })?;
        Ok(())
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)
}

#[cfg(unix)]
fn set_private_file(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn set_private_file(_: &Path) -> io::Result<()> {
    Ok(())
}
