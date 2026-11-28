//! The trust store: which plugin contents the user has approved.
//!
//! Trust is a content hash recorded by `slopvac plugins trust` (mise-style, no
//! prompts). Nothing else writes it: [`TrustGate::check`] only reads, and the
//! fetch, sync and update code has no handle to [`TrustStore::record`]. CI can
//! name hashes in `SLOPVAC_TRUST`. The store lives in the user's state
//! directory. A file with the same name in a project is never read.
//!
//! The trust hash covers everything that can execute or change what executes
//! (threat model section 5.4): the verified tree hash, the declaration, the
//! program and its arguments, the environment names and values and the `PATH`
//! passed to the plugin, the manifest digest and the publisher key, plus the
//! interpreter identity and environment listing of a managed Python
//! environment ([`crate::pyenv`]). Any change revokes trust.
//!
//! Environment values enter the hash only as HMAC-SHA256 commitments under a
//! per-user key kept in the store (see [`TrustKey`]), so a credential in a
//! plugin's declared environment cannot be guessed offline from the hash that
//! diagnostics and `SLOPVAC_TRUST` show. A changed value still changes the hash.
//!
//! Lookup is by trust hash only (section 5.4). A record also keeps the name,
//! version, source, key and declaration, but they only feed listings and update
//! diffs; "per project" is met because the declaration, arguments and
//! environment are inside the hash.
//!
//! The program must be an absolute path inside the verified tree, reached
//! without symlinks, and the hash tags it `tree:<relative path>`. A program
//! outside the tree (a managed interpreter) is refused unless the gate was given
//! an explicit allowlist entry ([`TrustGate::with_external_programs`]); the hash
//! then carries `external:<absolute path>`. Variables that load code into the
//! child (see [`is_forbidden_env_name`]) are refused outright.
//!
//! A Python plugin with a managed environment ([`TrustGate::with_managed_python`])
//! must run that environment's interpreter with [`crate::pyenv::PYTHON_FLAGS`]
//! first. The hash then carries `managed-python`, the interpreter identity and
//! the environment digest, and every spawn runs the quick environment check
//! (threat model Y5).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::error::PluginError;
use crate::extract::{ExtractError, TreeHash, tree_hash, verify_tree};
use crate::host::{PluginSpec, SpawnGate};
use crate::messages::trust::{self as messages, TrustPhrase, codes};
use crate::oci::Digest;
use crate::protocol::PluginDeclaration;
use crate::pyenv::{EnvStore, ManagedEnv, PYTHON_FLAGS, PyEnvError, PythonRuntime};
use crate::revoke::{RevocationError, RevocationList, RevocationNotice, Revocations};
use crate::sign::{
    PinScope, PublicKey, RotationProof, SignError, canonical_json, is_reserved_namespace,
    verify_rotation,
};

mod key;

pub use key::TrustKey;

const FILE_NAME: &str = "trust.json";
const LOCK_NAME: &str = "trust.lock";
const MAX_STORE_BYTES: u64 = 8 << 20;
/// The version this build writes. Version 2 added the registry host of each key pin.
const SCHEMA_VERSION: u32 = 2;

/// Errors from the trust store and its inputs.
#[derive(Debug, Error)]
pub enum TrustError {
    /// The store file is unreadable or invalid. The run exits 2; it is never treated as empty.
    #[error("{}", messages::store_damaged(.path, .why))]
    Corrupt {
        /// The file.
        path: PathBuf,
        /// What is wrong.
        why: String,
    },
    /// A file system operation failed.
    #[error("{}", messages::trust_io(.what, &.source.to_string()))]
    Io {
        /// The operation.
        what: String,
        /// The cause.
        #[source]
        source: io::Error,
    },
    /// `SLOPVAC_TRUST` named something other than a content hash.
    #[error("{}", messages::not_a_hash(.0))]
    NotAHash(String),
    /// The pair of registry host and namespace already has a different pinned key.
    #[error("{}", messages::namespace_bound(.registry, .namespace, .existing))]
    NamespaceBound {
        /// The registry host.
        registry: String,
        /// The namespace.
        namespace: String,
        /// Fingerprint of the key that holds it.
        existing: String,
    },
    /// The namespace is reserved or first-party, so no key can be pinned for it.
    #[error("{}", messages::namespace_reserved(.namespace))]
    ReservedNamespace {
        /// The namespace.
        namespace: String,
    },
    /// The plugin tree failed verification.
    #[error(transparent)]
    Tree(#[from] ExtractError),
    /// The program cannot be tied to the verified tree or the allowlist.
    #[error("{}", messages::program_refused(.0))]
    Program(String),
    /// The environment or `PATH` handed to the plugin is not allowed.
    #[error("{}", messages::environment_refused(.0))]
    Environment(String),
    /// The managed Python environment failed its full check.
    #[error(transparent)]
    PythonEnv(#[from] PyEnvError),
    /// A rotation statement failed verification. A `notBefore` that is not later than the
    /// pin's is a rollback; any other failure makes the rotation invalid.
    #[error("{}", messages::rotation_refused(.0))]
    Rotation(SignError),
    /// A rotation file cannot be read or is over its cap.
    #[error("{}", messages::rotation_file(.file))]
    RotationFile {
        /// The file, shown under the project root or as `<external>/<file name>`.
        file: String,
        /// Why, as a kind with no path and no bytes from the file.
        #[source]
        cause: FileCause,
    },
    /// A rotation names a pair of registry host and namespace that has no pinned key.
    #[error("{}", messages::not_pinned(.registry, .namespace))]
    NotPinned {
        /// The registry host.
        registry: String,
        /// The namespace.
        namespace: String,
    },
    /// The key file that `plugins pin --key` names cannot be used.
    #[error("{}", messages::key_invalid(.file, .why))]
    KeyInvalid {
        /// The file, shown under the project root or as `<external>/<file name>`.
        file: String,
        /// Why, as a fixed phrase that quotes nothing from the file.
        why: String,
        /// The same cause as a kind with no path and no bytes from the file.
        #[source]
        cause: FileCause,
    },
    /// The key does not have the fingerprint that the command names.
    #[error("{}", messages::fingerprint_mismatch(.expected, .actual))]
    FingerprintMismatch {
        /// The fingerprint the command names.
        expected: String,
        /// The fingerprint of the key.
        actual: String,
    },
    /// No revocation list was ever fetched, so a rotation cannot rule out a revoked key.
    #[error("{}", messages::rotation_revocations_missing())]
    RevocationsMissing,
    /// The revocation cache could not be read or verified. Spawns are refused.
    #[error(transparent)]
    Revocation(RevocationError),
    /// The revocation list names a key id of the rotation.
    #[error("{}", messages::rotation_key_revoked(.0))]
    Revoked(String),
    /// A write would push the store past the size the reader accepts.
    #[error("{}", messages::store_too_large(*.bytes))]
    TooLarge {
        /// Size the write would have.
        bytes: usize,
    },
    /// Trust records exist but the key they were made under is gone.
    #[error("{}", messages::key_missing(.file))]
    KeyMissing {
        /// The key file as policies section 1.7 shows a path outside the project.
        file: String,
    },
    /// A trust record was made under another key than the one in the store.
    #[error("{}", messages::key_mismatch())]
    KeyMismatch,
    /// The key file is not private to the user or does not hold a key.
    #[error("{}", messages::key_unusable(.file, .why))]
    KeyUnusable {
        /// The key file, redacted like [`Self::KeyMissing`]'s.
        file: String,
        /// What is wrong with it.
        why: String,
    },
}

impl TrustError {
    /// The stable error code (policies.md section 3.5) where one exists.
    #[must_use]
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Self::ReservedNamespace { .. } => Some(codes::NAMESPACE_RESERVED),
            Self::KeyInvalid { .. } => Some(codes::KEY_INVALID),
            Self::FingerprintMismatch { .. } => Some(codes::KEY_FINGERPRINT_MISMATCH),
            Self::NamespaceBound { .. } => Some(codes::NAMESPACE_BOUND),
            Self::NotPinned { .. } => Some(codes::NAMESPACE_NOT_PINNED),
            Self::Rotation(SignError::Rollback(_)) => Some(codes::ROTATION_ROLLBACK),
            Self::Rotation(_) | Self::RotationFile { .. } => Some(codes::ROTATION_INVALID),
            Self::Revoked(_) => Some(codes::ROTATION_KEY_REVOKED),
            Self::RevocationsMissing => Some(codes::ROTATION_REVOCATIONS_MISSING),
            _ => None,
        }
    }
}

/// Why a file that `plugins pin` or `plugins rotate` names cannot be used, kept as a kind only.
///
/// It holds no path and no bytes from the file, because the refusal can reach a log or an error
/// chain (policies section 1.7). The path the user named is in the refusal's own text, shown
/// under the project root or as `<external>/<file name>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum FileCause {
    /// The confined read refused the file or failed: not found, not a regular file, over the
    /// cap, not UTF-8, a link, or another failure.
    #[error("{0}")]
    Read(slopvac_fs::InputKind),
    /// The read failed with this I/O error kind.
    #[error("{0}")]
    Io(io::ErrorKind),
    /// The file holds no minisign public key.
    #[error("{}", TrustPhrase::KeyFileNoKey.as_static())]
    NoKey,
}

impl FileCause {
    fn of(e: &slopvac_fs::Error) -> Self {
        match e {
            slopvac_fs::Error::Input {
                kind: slopvac_fs::InputKind::Io,
                ..
            }
            | slopvac_fs::Error::Io { .. }
            | slopvac_fs::Error::Read { .. } => std::error::Error::source(e)
                .and_then(|s| s.downcast_ref::<io::Error>())
                .map_or(Self::Read(slopvac_fs::InputKind::Io), |s| {
                    Self::Io(s.kind())
                }),
            slopvac_fs::Error::CapExceeded { .. } => Self::Read(slopvac_fs::InputKind::TooLarge),
            slopvac_fs::Error::NotUtf8 { .. } => Self::Read(slopvac_fs::InputKind::NotUtf8),
            _ => Self::Read(e.input_kind().unwrap_or(slopvac_fs::InputKind::Io)),
        }
    }
}

/// Read a file that `plugins pin` or `plugins rotate` names, under `cap`, through the shared
/// confined reader (policies section 1.9): the open does not block, so a FIFO with no writer is
/// refused at once, and only a regular file is read.
fn read_named(path: &Path, cap: u64) -> Result<String, FileCause> {
    slopvac_fs::read_capped_regular_to_string(path, cap, slopvac_fs::ReadOptions::new())
        .map_err(|e| FileCause::of(&e))
}

fn io_err(what: impl Into<String>) -> impl FnOnce(io::Error) -> TrustError {
    let what = what.into();
    move |source| TrustError::Io { what, source }
}

/// Everything that goes into the trust hash.
#[derive(Debug, Clone)]
pub struct TrustInputs<'a> {
    /// Plugin name.
    pub name: &'a str,
    /// Hash of the verified, extracted tree.
    pub tree: &'a TreeHash,
    /// The declaration from the trusted manifest.
    pub declaration: &'a PluginDeclaration,
    /// The tagged program identity: `tree:<relative path>` or `external:<absolute path>`.
    pub program: &'a str,
    /// Arguments from the manifest.
    pub args: &'a [String],
    /// Names and values of the variables passed to the plugin, in any order.
    pub env: &'a [(String, String)],
    /// The `PATH` passed to the plugin.
    pub path_env: &'a str,
    /// Digest of the OCI manifest the tree came from.
    pub manifest_digest: Option<&'a str>,
    /// Fingerprint of the publisher key.
    pub key_fingerprint: Option<&'a str>,
    /// Identity of the managed interpreter, for Python plugins.
    pub interpreter: Option<&'a str>,
    /// Hash of the locked dependency set and environment file listing.
    pub environment: Option<&'a str>,
}

/// The trust hash of a plugin.
///
/// Each environment value enters as an HMAC under `key` (see [`TrustKey`]), so
/// the hash commits to the value without letting anyone test a guess for it.
///
/// # Panics
/// Never: the hash input always serialises.
#[must_use]
pub fn trust_hash(i: &TrustInputs<'_>, key: &TrustKey) -> Digest {
    let mut env: Vec<[String; 2]> = i
        .env
        .iter()
        .map(|(k, v)| [k.clone(), key.commit_env(k, v)])
        .collect();
    env.sort_unstable();
    let doc = json!({
        "v": 2,
        "name": i.name,
        "tree": i.tree.as_str(),
        "declaration": serde_json::to_value(i.declaration).expect("declaration serialises"),
        "program": i.program,
        "args": i.args,
        "env": env,
        "path": i.path_env,
        "manifest": i.manifest_digest,
        "key": i.key_fingerprint,
        "interpreter": i.interpreter,
        "environment": i.environment,
    });
    Digest::of(&canonical_json(&doc))
}

/// The declaration, flattened for storage and for update diffs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclSummary {
    /// May open connections.
    pub network: bool,
    /// `kind/id`, sorted.
    pub hooks: Vec<String>,
    /// Required features, sorted.
    pub required_features: Vec<String>,
    /// Optional features, sorted.
    pub optional_features: Vec<String>,
}

impl DeclSummary {
    /// Summarise a declaration.
    #[must_use]
    pub fn of(d: &PluginDeclaration) -> Self {
        let sorted = |v: &[String]| {
            v.iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        };
        Self {
            network: d.network,
            hooks: d
                .hooks
                .iter()
                .map(|h| format!("{}/{}", h.kind, h.id))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            required_features: sorted(&d.features.required),
            optional_features: sorted(&d.features.optional),
        }
    }

    /// Human-readable differences from `self` (old) to `new`. Names from the
    /// manifest are escaped, so a line is safe to print as it is.
    #[must_use]
    pub fn diff(&self, new: &Self) -> Vec<String> {
        let mut out = Vec::new();
        if self.network != new.network {
            out.push(messages::diff_network(self.network, new.network));
        }
        let mut set_diff = |label: &str, old: &[String], new: &[String]| {
            for added in new.iter().filter(|x| !old.contains(x)) {
                out.push(messages::diff_added(label, added));
            }
            for removed in old.iter().filter(|x| !new.contains(x)) {
                out.push(messages::diff_removed(label, removed));
            }
        };
        set_diff(TrustPhrase::LabelHook.as_static(), &self.hooks, &new.hooks);
        set_diff(
            TrustPhrase::LabelRequiredFeature.as_static(),
            &self.required_features,
            &new.required_features,
        );
        set_diff(
            TrustPhrase::LabelOptionalFeature.as_static(),
            &self.optional_features,
            &new.optional_features,
        );
        out
    }
}

/// One approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustRecord {
    /// The trust hash.
    pub hash: String,
    /// Plugin name.
    pub name: String,
    /// Plugin version, when known.
    pub version: Option<String>,
    /// Where the content came from, for example `oci:ghcr.io/acme/p@sha256:...`.
    pub source: String,
    /// Fingerprint of the publisher key, when signed.
    pub key_fingerprint: Option<String>,
    /// Digest of the OCI manifest, so `plugins status` can flag a revoked one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_digest: Option<String>,
    /// Key id of the publisher key, so `plugins status` can flag a revoked one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    /// Id of the trust key the hash was made under ([`TrustKey::id`]). A record
    /// without one, or with another id than the store's key, fails closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_key_id: Option<String>,
    /// What the plugin declared when it was trusted.
    pub declaration: DeclSummary,
    /// When, as `YYYY-MM-DDTHH:MM:SSZ`. The caller supplies the clock.
    pub recorded_at: String,
}

/// A pinned publisher key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyPin {
    /// Registry host the pin applies to, in ASCII lowercase. Required: a pin without a host
    /// does not parse, so the store is reported as damaged and never read as wider trust.
    pub registry: String,
    /// Namespace the key signs.
    pub namespace: String,
    /// SHA-256 fingerprint shown to the user.
    pub fingerprint: String,
    /// Base64 key.
    pub key: String,
    /// When it was pinned.
    pub pinned_at: String,
    /// The `notBefore` of the rotation that installed this key. A later
    /// rotation must be newer. A first pin has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_before: Option<String>,
}

impl KeyPin {
    /// The pair of registry host and namespace this pin binds.
    #[must_use]
    pub fn scope(&self) -> PinScope {
        PinScope::new(&self.registry, &self.namespace)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreFile {
    schema_version: u32,
    #[serde(default)]
    records: Vec<TrustRecord>,
    #[serde(default)]
    pins: Vec<KeyPin>,
}

/// Drop `x-` keys at every depth (policies 3.3: they are ignored; other unknown keys stay errors).
fn strip_extension_keys(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            map.retain(|k, _| !k.starts_with("x-"));
            map.values_mut().for_each(strip_extension_keys);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(strip_extension_keys),
        _ => {}
    }
}

fn parse_store_file(raw: &[u8], path: &Path) -> Result<StoreFile, TrustError> {
    let corrupt = |why: String| TrustError::Corrupt {
        path: path.to_owned(),
        why,
    };
    if raw.len() as u64 > MAX_STORE_BYTES {
        return Err(corrupt(TrustPhrase::StoreOverCap.as_static().into()));
    }
    let mut value: serde_json::Value =
        serde_json::from_slice(raw).map_err(|e| corrupt(e.to_string()))?;
    strip_extension_keys(&mut value);
    // Only version 2 is a trust store. Any other version, and a pin without a registry host,
    // is a damaged store: nothing is migrated and nothing is read as a pin on every host.
    if value.get("schema_version") != Some(&serde_json::Value::from(SCHEMA_VERSION)) {
        return Err(corrupt(TrustPhrase::StoreSchema.as_static().into()));
    }
    serde_json::from_value(value).map_err(|e| corrupt(e.to_string()))
}

/// Largest key file that `plugins pin --key` reads.
pub const MAX_KEY_FILE_BYTES: u64 = 4 * 1024;

/// Largest rotation statement or signature file that `plugins rotate` reads.
pub const MAX_ROTATION_FILE_BYTES: u64 = 4 * 1024;

/// One file that `plugins rotate` reads: where it is, and how messages name it (under the
/// project root, or as `<external>/<file name>`).
#[derive(Debug, Clone, Copy)]
pub struct NamedFile<'a> {
    /// The path to read.
    pub path: &'a Path,
    /// The path as messages show it.
    pub shown: &'a str,
}

/// The three files of a rotation: the statement and its two signatures.
#[derive(Debug, Clone, Copy)]
pub struct RotationFiles<'a> {
    /// The canonical rotation statement.
    pub statement: NamedFile<'a>,
    /// The `.minisig` text by the pinned key.
    pub old_signature: NamedFile<'a>,
    /// The `.minisig` text by the new key.
    pub new_signature: NamedFile<'a>,
}

/// A rotation that replaced a pin.
#[derive(Debug, Clone)]
pub struct Rotated {
    /// The new pin.
    pub pin: KeyPin,
    /// The fingerprint of the key it replaced.
    pub old_fingerprint: String,
    /// Revocation notices to print, such as a stale cached list. They do not block.
    pub notices: Vec<RevocationNotice>,
}

fn read_rotation_file(file: NamedFile<'_>) -> Result<String, TrustError> {
    read_named(file.path, MAX_ROTATION_FILE_BYTES).map_err(|cause| TrustError::RotationFile {
        file: file.shown.to_owned(),
        cause,
    })
}

pub(crate) fn reject_reserved(scope: &PinScope) -> Result<(), TrustError> {
    if is_reserved_namespace(scope.namespace()) {
        return Err(TrustError::ReservedNamespace {
            namespace: scope.namespace().to_owned(),
        });
    }
    Ok(())
}

/// `key` must have the fingerprint that the command names, compared byte for byte.
fn check_fingerprint(expected: &str, key: &PublicKey) -> Result<(), TrustError> {
    let actual = key.fingerprint();
    if actual == expected {
        Ok(())
    } else {
        Err(TrustError::FingerprintMismatch {
            expected: expected.to_owned(),
            actual,
        })
    }
}

/// Largest trust store file the reader accepts.
pub const MAX_STORE_FILE_BYTES: u64 = MAX_STORE_BYTES;

/// The contents of a trust store file, for read-only tools such as `doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustSnapshot {
    /// Plugin approvals.
    pub records: Vec<TrustRecord>,
    /// Pinned publisher keys.
    pub pins: Vec<KeyPin>,
}

/// Parse the bytes of a trust store file with the rules the store itself applies: at most
/// [`MAX_STORE_FILE_BYTES`], unknown keys refused, `x-` keys ignored, `schema_version` 2, and a
/// registry host on every key pin.
///
/// The caller supplies the bytes, so it can read them without blocking and under its own size
/// cap. `path` only names the file in the error.
///
/// # Errors
/// [`TrustError::Corrupt`] for bytes the store would refuse.
pub fn parse_store(raw: &[u8], path: &Path) -> Result<TrustSnapshot, TrustError> {
    let file = parse_store_file(raw, path)?;
    Ok(TrustSnapshot {
        records: file.records,
        pins: file.pins,
    })
}

/// The per-user trust store.
#[derive(Debug, Clone)]
pub struct TrustStore {
    dir: PathBuf,
}

impl TrustStore {
    /// A store in `dir`, the user's state directory. Nothing is created until a write.
    #[must_use]
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    fn read(&self) -> Result<StoreFile, TrustError> {
        let path = self.path();
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(StoreFile {
                    schema_version: SCHEMA_VERSION,
                    ..StoreFile::default()
                });
            }
            Err(e) => return Err(io_err(messages::opening(&path.display().to_string()))(e)),
        };
        let mut raw = Vec::new();
        file.take(MAX_STORE_BYTES + 1)
            .read_to_end(&mut raw)
            .map_err(io_err(messages::reading(&path.display().to_string())))?;
        if raw.len() as u64 > MAX_STORE_BYTES {
            return Err(TrustError::Corrupt {
                path,
                why: TrustPhrase::StoreOverCap.as_static().into(),
            });
        }
        parse_store_file(&raw, &path)
    }

    /// Take the exclusive store lock. It is released when the guard drops.
    fn lock(&self) -> Result<File, TrustError> {
        create_private_dir(&self.dir)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join(LOCK_NAME))
            .map_err(io_err(TrustPhrase::OpenLock.as_static()))?;
        lock.lock()
            .map_err(io_err(TrustPhrase::LockStore.as_static()))?;
        Ok(lock)
    }

    /// Run `f` on the store under an exclusive lock and write the result atomically.
    fn update<T>(
        &self,
        f: impl FnOnce(&mut StoreFile) -> Result<T, TrustError>,
    ) -> Result<T, TrustError> {
        let lock = self.lock()?;
        let out = self.update_held(&lock, f);
        drop(lock);
        out
    }

    /// [`Self::update`] for a caller that already holds the store lock (`_lock`, from
    /// [`Self::lock`]) and keeps it after the write.
    fn update_held<T>(
        &self,
        _lock: &File,
        f: impl FnOnce(&mut StoreFile) -> Result<T, TrustError>,
    ) -> Result<T, TrustError> {
        let mut file = self.read()?;
        let before = serde_json::to_vec_pretty(&file).map_err(|e| TrustError::Corrupt {
            path: self.path(),
            why: e.to_string(),
        })?;
        let out = f(&mut file)?;
        let bytes = serde_json::to_vec_pretty(&file).map_err(|e| TrustError::Corrupt {
            path: self.path(),
            why: e.to_string(),
        })?;
        if bytes == before && self.path().exists() {
            // Nothing changed: the file is not rewritten, so a version 1 file keeps its form.
            return Ok(out);
        }
        if bytes.len() as u64 > MAX_STORE_BYTES {
            return Err(TrustError::TooLarge { bytes: bytes.len() });
        }
        let mut tmp = tempfile::Builder::new()
            .prefix(".trust-")
            .tempfile_in(&self.dir)
            .map_err(io_err(TrustPhrase::TempStore.as_static()))?;
        set_private_file(tmp.path())?;
        tmp.write_all(&bytes)
            .map_err(io_err(TrustPhrase::WriteStore.as_static()))?;
        tmp.as_file()
            .sync_all()
            .map_err(io_err(TrustPhrase::SyncStore.as_static()))?;
        tmp.persist(self.path()).map_err(|e| TrustError::Io {
            what: TrustPhrase::ReplaceStore.as_static().into(),
            source: e.error,
        })?;
        Ok(out)
    }

    /// Whether `hash` has a record.
    ///
    /// # Errors
    /// [`TrustError::Corrupt`] for a damaged store.
    pub fn is_trusted(&self, hash: &Digest) -> Result<bool, TrustError> {
        Ok(self.read()?.records.iter().any(|r| r.hash == hash.as_str()))
    }

    /// Every record.
    ///
    /// # Errors
    /// [`TrustError::Corrupt`] for a damaged store.
    pub fn records(&self) -> Result<Vec<TrustRecord>, TrustError> {
        Ok(self.read()?.records)
    }

    /// Record trust. Only `slopvac plugins trust` calls this.
    ///
    /// # Errors
    /// Lock, read or write failures.
    pub fn record(&self, rec: TrustRecord) -> Result<(), TrustError> {
        self.update(|f| {
            f.schema_version = SCHEMA_VERSION;
            f.records.retain(|r| r.hash != rec.hash);
            f.records.push(rec);
            Ok(())
        })
    }

    /// Remove the record for `hash`. Returns whether one existed.
    ///
    /// # Errors
    /// Lock, read or write failures.
    pub fn forget(&self, hash: &Digest) -> Result<bool, TrustError> {
        self.update(|f| {
            let before = f.records.len();
            f.records.retain(|r| r.hash != hash.as_str());
            Ok(f.records.len() != before)
        })
    }

    /// Pin a publisher key for one pair of registry host and namespace. The first pin binds
    /// the pair to the key; a different key needs a rotation statement
    /// ([`Self::rotate_key`]). A pin on one host never binds the namespace on another.
    ///
    /// # Errors
    /// [`TrustError::NamespaceBound`] when the pair has another key,
    /// [`TrustError::ReservedNamespace`] for `slopvac` and `core`.
    pub fn pin_key(
        &self,
        scope: &PinScope,
        key: &PublicKey,
        now: &str,
    ) -> Result<KeyPin, TrustError> {
        let namespace = scope.namespace();
        if is_reserved_namespace(namespace) {
            return Err(TrustError::ReservedNamespace {
                namespace: namespace.to_owned(),
            });
        }
        let pin = KeyPin {
            registry: scope.registry().to_owned(),
            namespace: namespace.to_owned(),
            fingerprint: key.fingerprint(),
            key: key.to_base64(),
            pinned_at: now.to_owned(),
            not_before: None,
        };
        self.update(|f| {
            f.schema_version = SCHEMA_VERSION;
            if let Some(existing) = f.pins.iter().find(|p| p.scope() == *scope) {
                if existing.fingerprint == pin.fingerprint {
                    return Ok(existing.clone());
                }
                return Err(TrustError::NamespaceBound {
                    registry: scope.registry().to_owned(),
                    namespace: namespace.to_owned(),
                    existing: existing.fingerprint.clone(),
                });
            }
            f.pins.push(pin.clone());
            Ok(pin)
        })
    }

    /// Pin the key in `file` for the pair `scope`, after checking that it has `fingerprint`.
    ///
    /// The steps run in this order, and the first failure stops the call: a reserved
    /// namespace is refused before the file is read; the file is read under
    /// [`MAX_KEY_FILE_BYTES`] and must hold a minisign public key (a `minisign.pub` file or
    /// its base64 line alone); its fingerprint must equal `fingerprint`; then the pin is
    /// written as [`Self::pin_key`] writes it. `shown` names the file in errors, under the
    /// project root or as `<external>/<file name>`, and no error quotes the file.
    ///
    /// # Errors
    /// [`TrustError::ReservedNamespace`], [`TrustError::KeyInvalid`],
    /// [`TrustError::FingerprintMismatch`], [`TrustError::NamespaceBound`], plus store failures.
    pub fn pin_key_file(
        &self,
        scope: &PinScope,
        fingerprint: &str,
        file: &Path,
        shown: &str,
        now: &str,
    ) -> Result<KeyPin, TrustError> {
        reject_reserved(scope)?;
        let invalid = |cause: FileCause| {
            let why = match cause {
                FileCause::Read(slopvac_fs::InputKind::TooLarge) => TrustPhrase::KeyFileOverCap,
                FileCause::Read(slopvac_fs::InputKind::NotUtf8) | FileCause::NoKey => {
                    TrustPhrase::KeyFileNoKey
                }
                _ => TrustPhrase::KeyFileUnreadable,
            };
            TrustError::KeyInvalid {
                file: shown.to_owned(),
                why: why.as_static().to_owned(),
                cause,
            }
        };
        let text = read_named(file, MAX_KEY_FILE_BYTES).map_err(invalid)?;
        // The parse error can quote the file, so only its kind is kept.
        let key = PublicKey::from_pub_file(&text).map_err(|_| invalid(FileCause::NoKey))?;
        check_fingerprint(fingerprint, &key)?;
        self.pin_key(scope, &key, now)
    }

    /// Rotate the pin of the pair `scope` from the three files of a rotation, after checking
    /// that the new key has `fingerprint`. Nothing is fetched.
    ///
    /// The steps run in this order, and the first failure stops the call: a reserved
    /// namespace is refused before any file is read; each file is read under
    /// [`MAX_ROTATION_FILE_BYTES`], and the statement's `newKey` must parse; its fingerprint
    /// must equal `fingerprint`; the store lock and then the revocation cache lock are taken and
    /// held through the pin write; the cached revocation list is read, and a machine that never
    /// fetched one is refused; then [`Self::rotate_key`] checks the pin, the proof and the
    /// revocation list, and replaces the pin.
    ///
    /// # Errors
    /// [`TrustError::ReservedNamespace`], [`TrustError::RotationFile`],
    /// [`TrustError::Rotation`], [`TrustError::FingerprintMismatch`],
    /// [`TrustError::Revocation`] for a damaged cache, [`TrustError::RevocationsMissing`], and
    /// the errors of [`Self::rotate_key`].
    pub fn rotate_key_files(
        &self,
        scope: &PinScope,
        fingerprint: &str,
        files: &RotationFiles<'_>,
        revocations: &Revocations,
        now: &str,
    ) -> Result<Rotated, TrustError> {
        reject_reserved(scope)?;
        let statement = read_rotation_file(files.statement)?;
        let old_signature = read_rotation_file(files.old_signature)?;
        let new_signature = read_rotation_file(files.new_signature)?;
        let new_key =
            crate::sign::rotation_new_key(statement.as_bytes()).map_err(TrustError::Rotation)?;
        check_fingerprint(fingerprint, &new_key)?;
        // The list is read under the store lock and the cache lock, and both are held through
        // the pin write, so a refresh that caches a newer list while this call waits is the
        // list it checks.
        let store_lock = self.lock()?;
        let cache_lock = revocations.lock().map_err(TrustError::Revocation)?;
        let current = revocations.cached().map_err(TrustError::Revocation)?;
        if current.fetched_at.is_none() {
            return Err(TrustError::RevocationsMissing);
        }
        let proof = RotationProof {
            statement: statement.as_bytes(),
            old_signature: &old_signature,
            new_signature: &new_signature,
            new_key: &new_key,
        };
        let (old, pin) = self.rotate_held(&store_lock, scope, &proof, &current.list, now)?;
        drop(cache_lock);
        drop(store_lock);
        Ok(Rotated {
            pin,
            old_fingerprint: old.fingerprint,
            notices: current.notices,
        })
    }

    /// Replace the key pinned for the pair `scope` with the new key of a rotation.
    ///
    /// This is the pinning command's confirmation step: the caller has shown
    /// `proof.new_key`'s fingerprint first. Under the store lock it verifies the
    /// proof against the current pin ([`verify_rotation`], with the pin's
    /// `notBefore` as the floor) and refuses a rotation from or to a key on the
    /// revocation list, because a stolen key can sign a rotation to its thief's
    /// key (K5). The new pin records the rotation's `notBefore`.
    ///
    /// # Errors
    /// [`TrustError::ReservedNamespace`] for `slopvac` and `core`, before the store is read,
    /// [`TrustError::NotPinned`] for a pair with no pin,
    /// [`TrustError::Rotation`] for a proof that fails verification,
    /// [`TrustError::Revoked`] for a revoked key, plus store failures.
    pub fn rotate_key(
        &self,
        scope: &PinScope,
        proof: &RotationProof<'_>,
        revoked: &RevocationList,
        now: &str,
    ) -> Result<KeyPin, TrustError> {
        reject_reserved(scope)?;
        let lock = self.lock()?;
        let out = self.rotate_held(&lock, scope, proof, revoked, now);
        drop(lock);
        out.map(|(_, new)| new)
    }

    /// The rotation itself, under the store lock the caller holds: the pin before and the pin
    /// after.
    fn rotate_held(
        &self,
        lock: &File,
        scope: &PinScope,
        proof: &RotationProof<'_>,
        revoked: &RevocationList,
        now: &str,
    ) -> Result<(KeyPin, KeyPin), TrustError> {
        reject_reserved(scope)?;
        let namespace = scope.namespace();
        let path = self.path();
        self.update_held(lock, |f| {
            let Some(slot) = f.pins.iter_mut().find(|p| p.scope() == *scope) else {
                return Err(TrustError::NotPinned {
                    registry: scope.registry().to_owned(),
                    namespace: namespace.to_owned(),
                });
            };
            let pinned = PublicKey::from_base64(&slot.key).map_err(|e| TrustError::Corrupt {
                path: path.clone(),
                why: e.to_string(),
            })?;
            let st = verify_rotation(proof, namespace, &pinned, slot.not_before.as_deref())
                .map_err(TrustError::Rotation)?;
            for id in [&st.old_key_id, &st.new_key_id] {
                if revoked.revokes_key(id) {
                    return Err(TrustError::Revoked(id.clone()));
                }
            }
            let new = KeyPin {
                registry: scope.registry().to_owned(),
                namespace: namespace.to_owned(),
                fingerprint: proof.new_key.fingerprint(),
                key: proof.new_key.to_base64(),
                pinned_at: now.to_owned(),
                not_before: Some(st.not_before),
            };
            let old = std::mem::replace(slot, new.clone());
            Ok((old, new))
        })
    }

    /// Every pinned key by pair of registry host and namespace, ready for
    /// [`crate::sign::KeyPolicy::pinned`].
    ///
    /// # Errors
    /// [`TrustError::Corrupt`] for a damaged store or key,
    /// [`TrustError::ReservedNamespace`] for a pin in a reserved namespace.
    pub fn pinned_map(&self) -> Result<BTreeMap<PinScope, Vec<PublicKey>>, TrustError> {
        let mut map: BTreeMap<PinScope, Vec<PublicKey>> = BTreeMap::new();
        for p in self.read()?.pins {
            if is_reserved_namespace(&p.namespace) {
                return Err(TrustError::ReservedNamespace {
                    namespace: p.namespace,
                });
            }
            let key = PublicKey::from_base64(&p.key).map_err(|e| TrustError::Corrupt {
                path: self.path(),
                why: e.to_string(),
            })?;
            map.entry(p.scope()).or_default().push(key);
        }
        Ok(map)
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> Result<(), TrustError> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(io_err(messages::creating(&dir.display().to_string())))?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
        .map_err(io_err(TrustPhrase::SecureDir.as_static()))
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> Result<(), TrustError> {
    fs::create_dir_all(dir).map_err(io_err(messages::creating(&dir.display().to_string())))
}

#[cfg(unix)]
fn set_private_file(path: &Path) -> Result<(), TrustError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(io_err(TrustPhrase::SecureFile.as_static()))
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn set_private_file(_: &Path) -> Result<(), TrustError> {
    Ok(())
}

/// Parse `SLOPVAC_TRUST`: hashes separated by commas or white space.
///
/// # Errors
/// [`TrustError::NotAHash`] for any token that is not a `sha256:` content hash.
pub fn parse_trust_env(value: &str) -> Result<Vec<Digest>, TrustError> {
    value
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .map(|t| Digest::parse(t).map_err(|_| TrustError::NotAHash(t.to_owned())))
        .collect()
}

/// A plugin that sync verified and extracted.
#[derive(Debug, Clone)]
pub struct VerifiedPlugin {
    /// Plugin name.
    pub name: String,
    /// Version, when known.
    pub version: Option<String>,
    /// Source description for the trust record.
    pub source: String,
    /// The extracted tree. The plugin runs from here.
    pub root: PathBuf,
    /// Hash of that tree as extraction computed it.
    pub tree: TreeHash,
    /// Digest of the OCI manifest, when it came from a registry.
    pub manifest_digest: Option<String>,
    /// Fingerprint of the key that signed it.
    pub key_fingerprint: Option<String>,
    /// Key id of the key that signed it, which the revocation list names.
    pub key_id: Option<String>,
    /// Content of `slopvac-plugin.toml` as the pass that hashed the tree read it. Every verified
    /// plugin is a code plugin, and a code plugin has a manifest, so the field is not optional:
    /// no path can skip a runtime for a missing manifest. The gate selects a managed runtime
    /// from these bytes and never opens the file again.
    pub manifest: Vec<u8>,
}

/// Whether `name` may not be passed to a plugin: it loads code into the child
/// or changes how the program is found.
///
/// Matched case-insensitively. Prefixes: `LD_`, `DYLD_`, `PYTHON`, `PERL`,
/// `RUBY`, `NODE_`, `LUA_`. Exact names: `PATH` (the host sets it), `IFS`,
/// `ENV`, `BASH_ENV`, `CDPATH`, `GCONV_PATH`, `LOCPATH`, `NLSPATH`,
/// `HOSTALIASES`, `CLASSPATH`, `JAVA_TOOL_OPTIONS`, `_JAVA_OPTIONS`,
/// `JDK_JAVA_OPTIONS`, `PATHEXT`, `COMSPEC`.
#[must_use]
pub fn is_forbidden_env_name(name: &str) -> bool {
    const PREFIXES: [&str; 7] = ["LD_", "DYLD_", "PYTHON", "PERL", "RUBY", "NODE_", "LUA_"];
    const EXACT: [&str; 13] = [
        "PATH",
        "IFS",
        "ENV",
        "BASH_ENV",
        "CDPATH",
        "GCONV_PATH",
        "LOCPATH",
        "NLSPATH",
        "HOSTALIASES",
        "CLASSPATH",
        "JAVA_TOOL_OPTIONS",
        "_JAVA_OPTIONS",
        "JDK_JAVA_OPTIONS",
    ];
    let up = name.to_ascii_uppercase();
    PREFIXES.iter().any(|p| up.starts_with(p))
        || EXACT.contains(&up.as_str())
        || up == "PATHEXT"
        || up == "COMSPEC"
}

fn check_environment(spec: &PluginSpec) -> Result<(), TrustError> {
    let refuse = |why: String| Err(TrustError::Environment(why));
    let mut seen = HashSet::new();
    for (k, _) in &spec.env {
        if k.is_empty() || k.contains('=') || k.contains('\0') {
            return refuse(messages::env_bad_name(k));
        }
        if is_forbidden_env_name(k) {
            return refuse(messages::env_forbidden(k));
        }
        if !seen.insert(k.to_ascii_uppercase()) {
            return refuse(messages::env_duplicate(k));
        }
    }
    if spec.path_env.is_empty() {
        return refuse(TrustPhrase::PathEmpty.as_static().into());
    }
    for dir in std::env::split_paths(&spec.path_env) {
        if !dir.is_absolute() {
            return refuse(messages::env_path_relative(&dir));
        }
    }
    Ok(())
}

/// The tagged identity of the program, or the reason it is refused.
fn program_id(
    root: &Path,
    program: &Path,
    external: &HashSet<PathBuf>,
) -> Result<String, TrustError> {
    let refuse = |why: String| Err(TrustError::Program(why));
    if !program.is_absolute() {
        return refuse(messages::program_not_absolute(program));
    }
    if program
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return refuse(messages::program_dots(program));
    }
    let Ok(rel) = program.strip_prefix(root) else {
        if external.contains(program) {
            return Ok(format!("external:{}", program.display()));
        }
        return refuse(messages::program_outside(program));
    };
    let mut cur = root.to_path_buf();
    let mut parts = Vec::new();
    for c in rel.components() {
        let Component::Normal(name) = c else {
            return refuse(messages::program_not_plain(program));
        };
        cur.push(name);
        let meta = fs::symlink_metadata(&cur)
            .map_err(|e| TrustError::Program(messages::program_unreadable(&cur, &e.to_string())))?;
        if meta.file_type().is_symlink() {
            return refuse(messages::program_symlink(&cur));
        }
        parts.push(name.to_string_lossy().into_owned());
    }
    if parts.is_empty() || !cur.is_file() {
        return refuse(messages::program_not_file(program));
    }
    Ok(format!("tree:{}", parts.join("/")))
}

fn hash_for(
    vp: &VerifiedPlugin,
    spec: &PluginSpec,
    external: &HashSet<PathBuf>,
    python: Option<&ManagedEnv>,
    key: &TrustKey,
) -> Result<Digest, TrustError> {
    check_environment(spec)?;
    let (program, interpreter, environment) = match python {
        None => (program_id(&vp.root, &spec.program, external)?, None, None),
        Some(env) => {
            if spec.program != env.python() {
                return Err(TrustError::Program(messages::program_not_managed(
                    &spec.program,
                    &env.python(),
                )));
            }
            if !spec.args.starts_with(&PYTHON_FLAGS.map(String::from)) {
                return Err(TrustError::Program(
                    TrustPhrase::PythonFlags.as_static().into(),
                ));
            }
            (
                "managed-python".to_owned(),
                Some(env.interpreter_identity()),
                Some(env.environment_digest().to_string()),
            )
        }
    };
    Ok(trust_hash(
        &TrustInputs {
            name: &vp.name,
            tree: &vp.tree,
            declaration: &spec.declaration,
            program: &program,
            args: &spec.args,
            env: &spec.env,
            path_env: &spec.path_env,
            manifest_digest: vp.manifest_digest.as_deref(),
            key_fingerprint: vp.key_fingerprint.as_deref(),
            interpreter: interpreter.as_deref(),
            environment: environment.as_deref(),
        },
        key,
    ))
}

/// Where a plugin stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustState {
    /// Recorded in the store.
    Trusted,
    /// Named in `SLOPVAC_TRUST`.
    TrustedByEnv,
    /// On the revocation list. Records for it are inert.
    Revoked,
    /// Not trusted. `changes` lists what differs from earlier trusted versions of the same name.
    Untrusted {
        /// Declaration differences against the most recent trusted record of this name.
        changes: Vec<String>,
        /// Whether any earlier version of this plugin was trusted.
        previously_trusted: bool,
    },
}

/// An entry on the revocation list. Each type revokes on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Revocation {
    /// One exact trust hash.
    TrustHash(Digest),
    /// Every plugin built from this OCI manifest.
    ManifestDigest(Digest),
    /// Every plugin signed by this publisher key fingerprint.
    PublisherKey(String),
    /// Every plugin signed by the key with this key id (16 hex digits), the
    /// form the signed revocation list uses.
    KeyId(String),
}

/// Decides, before spawn, whether a plugin may run. Read-only.
pub struct TrustGate {
    store: TrustStore,
    env: Vec<Digest>,
    revoked_hashes: HashSet<String>,
    revoked_manifests: HashSet<String>,
    revoked_keys: HashSet<String>,
    revoked_key_ids: HashSet<String>,
    cache: Option<Revocations>,
    external: HashSet<PathBuf>,
    python: HashMap<String, ManagedEnv>,
    launches: HashMap<String, PythonRuntime>,
    plugins: HashMap<String, VerifiedPlugin>,
}

impl TrustGate {
    /// A gate over `plugins`, the verified plugins of this run.
    #[must_use]
    pub fn new(store: TrustStore, env: Vec<Digest>, plugins: Vec<VerifiedPlugin>) -> Self {
        Self {
            store,
            env,
            revoked_hashes: HashSet::new(),
            revoked_manifests: HashSet::new(),
            revoked_keys: HashSet::new(),
            revoked_key_ids: HashSet::new(),
            cache: None,
            external: HashSet::new(),
            python: HashMap::new(),
            launches: HashMap::new(),
            plugins: plugins.into_iter().map(|p| (p.name.clone(), p)).collect(),
        }
    }

    /// Revocations that must never run, from the revocation list. They win over
    /// the store and `SLOPVAC_TRUST`.
    #[must_use]
    pub fn with_revoked(mut self, list: impl IntoIterator<Item = Revocation>) -> Self {
        for r in list {
            match r {
                Revocation::TrustHash(d) => self.revoked_hashes.insert(d.to_string()),
                Revocation::ManifestDigest(d) => self.revoked_manifests.insert(d.to_string()),
                Revocation::PublisherKey(k) => self.revoked_keys.insert(k.to_ascii_lowercase()),
                Revocation::KeyId(k) => self.revoked_key_ids.insert(k.to_ascii_uppercase()),
            };
        }
        self
    }

    /// Check every spawn against the signed revocation cache. The gate reads
    /// and verifies the cache again at each [`SpawnGate::check`] and
    /// [`Self::state`], so a list that a refresh caches after the gate was
    /// built still stops the next spawn. A cache that fails verification
    /// refuses every spawn ([`TrustError::Revocation`]). A missing cache
    /// revokes nothing (residual risk R3).
    #[must_use]
    pub fn with_revocation_cache(mut self, cache: Revocations) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Allow programs outside the verified tree, by absolute path. Such a path
    /// goes into the trust hash as `external:<path>`. Nothing is allowed by
    /// default; a managed interpreter uses [`Self::with_managed_python`].
    #[must_use]
    pub fn with_external_programs(mut self, paths: impl IntoIterator<Item = PathBuf>) -> Self {
        self.external.extend(paths);
        self
    }

    /// Run `plugin` in its managed Python environment. Its spec must name
    /// [`ManagedEnv::python`] as the program and start its arguments with
    /// [`PYTHON_FLAGS`]; the interpreter identity and environment digest join
    /// the trust hash, and [`ManagedEnv::check_quick`] runs before every spawn.
    #[must_use]
    pub fn with_managed_python(mut self, plugin: impl Into<String>, env: ManagedEnv) -> Self {
        self.python.insert(plugin.into(), env);
        self
    }

    /// Wire every Python plugin among the gate's verified plugins to its
    /// environment in `envs`. A plugin whose manifest selects
    /// `runtime = "python"` ([`PythonRuntime::read`]) goes through
    /// [`Self::with_managed_python`] with the environment that `sync` built
    /// for its tree and interpreter request; [`Self::python_launch`] then
    /// gives the program and arguments for its spec.
    ///
    /// The environment is loaded, not checked: [`SpawnGate::check`] runs the
    /// quick check before every spawn.
    ///
    /// # Errors
    /// [`PyEnvError::Manifest`] for a manifest that cannot select an
    /// environment, [`PyEnvError::EnvMissing`] for a plugin that `sync` has
    /// not built one for, and the load failures of [`EnvStore::load`]. The
    /// gate is dropped: a Python plugin never runs half-wired.
    pub fn with_python_plugins(mut self, envs: &EnvStore) -> Result<Self, PyEnvError> {
        let mut names: Vec<String> = self.plugins.keys().cloned().collect();
        names.sort();
        for name in names {
            let vp = &self.plugins[&name];
            // The bytes the hash pass read, never the file on disk.
            let Some(runtime) = PythonRuntime::from_manifest(&vp.root, &vp.manifest)? else {
                continue;
            };
            let env =
                envs.load(&vp.tree, &runtime.python)?
                    .ok_or_else(|| PyEnvError::EnvMissing {
                        plugin: name.clone(),
                        python: runtime.python.clone(),
                    })?;
            self.python.insert(name.clone(), env);
            self.launches.insert(name, runtime);
        }
        Ok(self)
    }

    /// The program and arguments for the spec of a plugin that
    /// [`Self::with_python_plugins`] wired, or `None` for any other plugin.
    #[must_use]
    pub fn python_launch(&self, plugin: &str) -> Option<(PathBuf, Vec<String>)> {
        let env = self.python.get(plugin)?;
        Some(self.launches.get(plugin)?.launch(env))
    }

    fn revoked(&self, vp: &VerifiedPlugin, hash: &Digest) -> Result<bool, TrustError> {
        if let Some(cache) = &self.cache {
            let list = cache.cached().map_err(TrustError::Revocation)?.list;
            let by_digest = vp
                .manifest_digest
                .as_deref()
                .is_some_and(|m| list.revokes_digest(m));
            let by_key = vp.key_id.as_deref().is_some_and(|k| list.revokes_key(k));
            if by_digest || by_key {
                return Ok(true);
            }
        }
        Ok(self.revoked_hashes.contains(hash.as_str())
            || vp
                .manifest_digest
                .as_ref()
                .is_some_and(|m| self.revoked_manifests.contains(m))
            || vp
                .key_fingerprint
                .as_ref()
                .is_some_and(|k| self.revoked_keys.contains(&k.to_ascii_lowercase()))
            || vp
                .key_id
                .as_ref()
                .is_some_and(|k| self.revoked_key_ids.contains(&k.to_ascii_uppercase())))
    }

    /// The state of a plugin started as `spec`.
    ///
    /// # Errors
    /// [`TrustError::Program`] or [`TrustError::Environment`] for a spec that can
    /// never be trusted, [`TrustError::Revocation`] for a revocation cache that
    /// fails verification, plus store failures.
    pub fn state(
        &self,
        vp: &VerifiedPlugin,
        spec: &PluginSpec,
    ) -> Result<(Digest, TrustState), TrustError> {
        let key = self.store.key()?;
        let hash = hash_for(vp, spec, &self.external, self.python.get(&vp.name), &key)?;
        if self.revoked(vp, &hash)? {
            return Ok((hash, TrustState::Revoked));
        }
        if self.env.contains(&hash) {
            return Ok((hash, TrustState::TrustedByEnv));
        }
        if self.store.is_trusted(&hash)? {
            return Ok((hash, TrustState::Trusted));
        }
        let new = DeclSummary::of(&spec.declaration);
        let earlier: Vec<TrustRecord> = self
            .store
            .records()?
            .into_iter()
            .filter(|r| r.name == vp.name)
            .collect();
        let changes = earlier
            .last()
            .map(|r| r.declaration.diff(&new))
            .unwrap_or_default();
        Ok((
            hash,
            TrustState::Untrusted {
                changes,
                previously_trusted: !earlier.is_empty(),
            },
        ))
    }

    /// What `slopvac plugins trust` does: verify the tree and, for a managed
    /// Python environment, run its full check, then record the hash.
    ///
    /// # Errors
    /// [`TrustError::Tree`] when the tree no longer matches,
    /// [`TrustError::PythonEnv`] when the environment does not, a refused
    /// spec, plus store failures.
    pub fn record_trust(
        &self,
        vp: &VerifiedPlugin,
        spec: &PluginSpec,
        now: &str,
    ) -> Result<TrustRecord, TrustError> {
        verify_tree(&vp.root, &vp.tree)?;
        let python = self.python.get(&vp.name);
        if let Some(env) = python {
            env.verify_full()?;
        }
        self.store.record_with_key(|key| {
            let hash = hash_for(vp, spec, &self.external, python, key)?;
            Ok(TrustRecord {
                hash: hash.to_string(),
                name: vp.name.clone(),
                version: vp.version.clone(),
                source: vp.source.clone(),
                key_fingerprint: vp.key_fingerprint.clone(),
                manifest_digest: vp.manifest_digest.clone(),
                key_id: vp.key_id.clone(),
                trust_key_id: Some(key.id().to_owned()),
                declaration: DeclSummary::of(&spec.declaration),
                recorded_at: now.to_owned(),
            })
        })
    }
}

impl SpawnGate for TrustGate {
    fn check(&self, spec: &PluginSpec) -> Result<(), PluginError> {
        let name = &spec.name;
        let vp = self
            .plugins
            .get(name)
            .ok_or_else(|| PluginError::Untrusted(name.clone()))?;
        if spec.cwd != vp.root {
            return Err(PluginError::HashMismatch(messages::mismatch_cwd(name)));
        }
        // Spawning recomputes the tree hash: a cached tree is untrusted input (E8, P10).
        // Same comparison as `verify_tree`, kept inline so the error can name the new hash.
        match tree_hash(&vp.root) {
            Ok(actual) if actual == vp.tree => {}
            Ok(actual) => {
                return Err(PluginError::HashMismatch(messages::mismatch_tree_changed(
                    name,
                    &actual.to_string(),
                    &vp.tree.to_string(),
                )));
            }
            Err(e) => {
                return Err(PluginError::HashMismatch(messages::mismatch_tree_refused(
                    name,
                    &e.unescaped(),
                )));
            }
        }
        // The Y5 trade-off: metadata only here, content hashes in `plugins status` and `doctor`.
        if let Some(env) = self.python.get(name)
            && let Err(e) = env.check_quick()
        {
            return Err(PluginError::HashMismatch(messages::name_with(
                name,
                &e.unescaped(),
            )));
        }
        let (hash, state) = match self.state(vp, spec) {
            Ok(x) => x,
            // Raw payloads: `PluginError`'s `Display` escapes the whole message once.
            Err(TrustError::Program(m)) => {
                return Err(PluginError::Usage(messages::spawn_program_refused(
                    name, &m,
                )));
            }
            Err(TrustError::Environment(m)) => {
                return Err(PluginError::Usage(messages::spawn_environment_refused(
                    name, &m,
                )));
            }
            Err(TrustError::KeyMissing { .. }) => {
                return Err(PluginError::TrustKeyMissing(name.clone()));
            }
            Err(TrustError::KeyMismatch) => {
                return Err(PluginError::TrustKeyMismatch(name.clone()));
            }
            Err(TrustError::KeyUnusable { file, why }) => {
                return Err(PluginError::TrustKeyUnusable(messages::spawn_key_unusable(
                    name, &file, &why,
                )));
            }
            Err(_) => return Err(PluginError::Untrusted(name.clone())),
        };
        match state {
            TrustState::Trusted | TrustState::TrustedByEnv => Ok(()),
            TrustState::Revoked | TrustState::Untrusted { .. } => Err(PluginError::Untrusted(
                messages::name_with(name, &hash.to_string()),
            )),
        }
    }
}
