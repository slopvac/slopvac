//! Load the plugins that the config lists from the lock and the store.
//!
//! The config decides which plugins load (threat model X6). The lock pins each one, and the
//! store holds its tree, which `Store::inspect` hashes again before the loader reads a file.
//! A plugin the lock lacks, a tree that is absent or differs from its digest, an extension API
//! this build does not implement and a `requires` loop each stop the run.

use std::collections::HashSet;

use semver::Version;
use slopvac_model::{ApiVersion, PackId};
use slopvac_plugins::lock::{LockedPack, Lockfile, PackId as LockPackId, PackKind as LockKind};
use slopvac_plugins::sync::{Store, SyncError, TreeState};

use crate::messages::PackMessage;
use crate::{DirSource, LoadedPack, PackError, PackKind, Packs, Source, to_lock_id};

/// The id of the built-in pack, which no lock pins.
const CORE_ID: &str = "core";

/// Why the synced plugins did not load.
#[derive(Debug)]
#[non_exhaustive]
pub enum SyncedError {
    /// A plugin's files break the contract.
    Pack(PackError),
    /// The config or a `requires` list names a plugin that the lock does not pin (X6).
    NotLocked {
        /// The plugin id.
        id: String,
    },
    /// The lock pins the plugin but the store has no tree for it.
    NotInstalled {
        /// The plugin id.
        id: String,
    },
    /// The stored tree differs from the lock digest. The run exits 2.
    Tampered {
        /// The plugin id.
        id: String,
    },
    /// The store could not be read.
    StoreUnreadable {
        /// The plugin id.
        id: String,
        /// The store failure.
        source: Box<SyncError>,
    },
    /// The plugin targets an extension API this build does not implement.
    ApiUnsupported {
        /// The plugin id.
        id: String,
        /// The API the plugin targets.
        api: u64,
    },
    /// The installed manifest disagrees with the lock entry.
    ManifestMismatch {
        /// The plugin id.
        id: String,
        /// The field that differs: `id`, `version` or `kind`.
        field: &'static str,
    },
    /// Plugins require each other in a loop.
    RequiresCycle {
        /// The plugin ids on the loop, the first repeated at the end.
        cycle: Vec<String>,
    },
    /// A required plugin is not loaded at a version that satisfies the requirement.
    RequirementUnmet {
        /// The plugin that requires.
        id: String,
        /// The required plugin.
        needs: String,
        /// The version requirement.
        requirement: String,
    },
}

impl From<PackError> for SyncedError {
    fn from(error: PackError) -> Self {
        Self::Pack(error)
    }
}

impl SyncedError {
    fn message(&self) -> Option<PackMessage> {
        Some(match self {
            Self::Pack(_) => return None,
            Self::NotLocked { id } => PackMessage::NotLocked { id: id.clone() },
            Self::NotInstalled { id } => PackMessage::NotInstalled { id: id.clone() },
            Self::Tampered { id } => PackMessage::Tampered { id: id.clone() },
            Self::StoreUnreadable { id, .. } => PackMessage::StoreUnreadable { id: id.clone() },
            Self::ApiUnsupported { id, api } => PackMessage::ApiUnsupported {
                id: id.clone(),
                api: api.to_string(),
            },
            Self::ManifestMismatch { id, field } => PackMessage::ManifestMismatch {
                id: id.clone(),
                field: (*field).to_owned(),
            },
            Self::RequiresCycle { cycle } => PackMessage::RequiresCycle {
                cycle: cycle.join(" -> "),
            },
            Self::RequirementUnmet {
                id,
                needs,
                requirement,
            } => PackMessage::RequirementUnmet {
                id: id.clone(),
                needs: needs.clone(),
                requirement: requirement.clone(),
            },
        })
    }

    /// The stable code of this error. `None` for a wrapped [`PackError`], which has its own
    /// codes where it has any.
    #[must_use]
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Self::Pack(PackError::Registry(e)) => e.code(),
            Self::Pack(_) => None,
            Self::NotLocked { .. } => Some("plugin-not-locked"),
            Self::NotInstalled { .. } => Some("plugin-not-installed"),
            Self::Tampered { .. } => Some("plugin-tampered"),
            Self::StoreUnreadable { .. } => Some("plugin-store-unreadable"),
            Self::ApiUnsupported { .. } => Some("plugin-api-unsupported"),
            Self::ManifestMismatch { .. } => Some("plugin-manifest-mismatch"),
            Self::RequiresCycle { .. } => Some("plugin-requires-cycle"),
            Self::RequirementUnmet { .. } => Some("plugin-requirement-unmet"),
        }
    }
}

/// Every text goes through the message registry (`PackMessage`), so `Display` only forwards.
impl std::fmt::Display for SyncedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.message(), self) {
            (Some(message), _) => write!(f, "{message}"),
            (None, Self::Pack(e)) => write!(f, "{e}"),
            (None, _) => Ok(()),
        }
    }
}

impl std::error::Error for SyncedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Pack(e) => Some(e),
            Self::StoreUnreadable { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

/// The highest extension API this build implements.
fn host_api() -> u64 {
    ApiVersion::SUPPORTED.into_iter().max().map_or(0, u64::from)
}

fn lock_id(id: &PackId) -> Result<LockPackId, SyncedError> {
    to_lock_id(id).map_err(|_| SyncedError::NotLocked { id: id.to_string() })
}

/// The requested plugins and everything they require, dependencies first. A plugin comes
/// after each plugin it requires, and the order is otherwise that of the request.
fn dependency_order(requested: &[PackId], lock: &Lockfile) -> Result<Vec<LockedPack>, SyncedError> {
    fn visit(
        id: &LockPackId,
        lock: &Lockfile,
        stack: &mut Vec<String>,
        done: &mut HashSet<String>,
        out: &mut Vec<LockedPack>,
    ) -> Result<(), SyncedError> {
        if id.as_str() == CORE_ID || done.contains(id.as_str()) {
            return Ok(());
        }
        if let Some(at) = stack.iter().position(|s| s == id.as_str()) {
            let mut cycle = stack[at..].to_vec();
            cycle.push(id.to_string());
            return Err(SyncedError::RequiresCycle { cycle });
        }
        let locked = lock
            .get(id)
            .ok_or_else(|| SyncedError::NotLocked { id: id.to_string() })?;
        stack.push(id.to_string());
        for required in &locked.requires {
            visit(required, lock, stack, done, out)?;
        }
        stack.pop();
        done.insert(id.to_string());
        out.push(locked.clone());
        Ok(())
    }
    let mut out = Vec::new();
    let mut done = HashSet::new();
    for id in requested {
        visit(&lock_id(id)?, lock, &mut Vec::new(), &mut done, &mut out)?;
    }
    Ok(out)
}

/// Check the installed manifest against the lock entry and the API this build implements.
fn check_manifest(locked: &LockedPack, pack: &LoadedPack) -> Result<(), SyncedError> {
    let manifest = pack.manifest();
    let id = locked.id.to_string();
    let kind = match manifest.kind {
        PackKind::Data => LockKind::Data,
        PackKind::Code => LockKind::Code,
    };
    let differs = |field| SyncedError::ManifestMismatch {
        id: id.clone(),
        field,
    };
    if manifest.id.as_str() != locked.id.as_str() {
        return Err(differs("id"));
    }
    if manifest.version != locked.version {
        return Err(differs("version"));
    }
    if kind != locked.kind {
        return Err(differs("kind"));
    }
    let api = u64::from(manifest.slopvac_api.get());
    if api > host_api() {
        return Err(SyncedError::ApiUnsupported { id, api });
    }
    Ok(())
}

/// Check that each plugin the manifest requires is loaded already, at a satisfying version.
fn check_requirements(pack: &LoadedPack, loaded: &[LoadedPack]) -> Result<(), SyncedError> {
    let manifest = pack.manifest();
    for requirement in &manifest.requires {
        let version: Option<Version> = if requirement.id.as_str() == CORE_ID {
            Version::parse(env!("CARGO_PKG_VERSION")).ok()
        } else {
            loaded
                .iter()
                .map(LoadedPack::manifest)
                .find(|m| m.id == requirement.id)
                .map(|m| m.version.clone())
        };
        if !version.is_some_and(|v| requirement.version.matches(&v)) {
            return Err(SyncedError::RequirementUnmet {
                id: manifest.id.to_string(),
                needs: requirement.id.to_string(),
                requirement: requirement.version.to_string(),
            });
        }
    }
    Ok(())
}

/// Load the plugins in `requested`, and the plugins they require, from `lock` and `store`.
///
/// The result is in dependency order. The built-in core is not part of it: put it first with
/// [`Packs::with_synced`]. Pack taggers are not registered (see [`LoadedPack::load`]).
///
/// # Errors
/// [`SyncedError`]: a plugin the lock does not pin ([`SyncedError::NotLocked`]), a tree that is
/// absent or differs from its digest, an unsupported extension API, a manifest that disagrees
/// with the lock, an unmet requirement, a `requires` cycle, or a file that does not load.
pub fn load_synced(
    requested: &[PackId],
    lock: &Lockfile,
    store: &Store,
) -> Result<Vec<LoadedPack>, SyncedError> {
    let mut loaded: Vec<LoadedPack> = Vec::new();
    for locked in dependency_order(requested, lock)? {
        let id = locked.id.to_string();
        if locked.slopvac_api > host_api() {
            return Err(SyncedError::ApiUnsupported {
                id,
                api: locked.slopvac_api,
            });
        }
        let dir = match store.inspect(&locked.digest) {
            Ok(TreeState::Intact(dir)) => dir,
            Ok(TreeState::Absent) => return Err(SyncedError::NotInstalled { id }),
            Ok(TreeState::Tampered(_)) => return Err(SyncedError::Tampered { id }),
            Err(source) => {
                return Err(SyncedError::StoreUnreadable {
                    id,
                    source: Box::new(source),
                });
            }
        };
        let origin = match locked.kind {
            LockKind::Data => Source::Pack,
            LockKind::Code => Source::Plugin,
        };
        let pack = LoadedPack::load(&DirSource::new(dir), origin)?;
        check_manifest(&locked, &pack)?;
        check_requirements(&pack, &loaded)?;
        loaded.push(pack);
    }
    Ok(loaded)
}

impl Packs {
    /// The core pack followed by the synced plugins that `requested` names, in dependency
    /// order. File kinds and harnesses are not part of a pack, so no data pack can add one.
    ///
    /// # Errors
    /// [`SyncedError`] as for [`load_synced`], and for the registries of [`Packs::new`].
    pub fn with_synced(
        requested: &[PackId],
        lock: &Lockfile,
        store: &Store,
    ) -> Result<Self, SyncedError> {
        let mut packs = vec![LoadedPack::load(&crate::core_source(), Source::Core)?];
        packs.extend(load_synced(requested, lock, store)?);
        Ok(Self::new(packs)?)
    }
}
