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
use crate::{
    Budget, DirSource, Limits, LoadedPack, PackError, PackKind, Packs, Source, to_lock_id,
};

/// The id of the built-in pack, which no lock pins.
const CORE_ID: &str = "core";

/// Why the synced plugins did not load. Every text comes from the message catalog.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SyncedError {
    /// A plugin's files break the contract, or a budget is crossed.
    #[error(transparent)]
    Pack(#[from] PackError),
    /// The config or a `requires` list names a plugin that the lock does not pin (X6).
    #[error("{}", PackMessage::NotLocked { id: String::from(.id) })]
    NotLocked {
        /// The plugin id.
        id: String,
    },
    /// The lock pins the plugin but the store has no tree for it.
    #[error("{}", PackMessage::NotInstalled { id: String::from(.id) })]
    NotInstalled {
        /// The plugin id.
        id: String,
    },
    /// The stored tree differs from the lock digest. The run exits 2.
    #[error("{}", PackMessage::Tampered { id: String::from(.id) })]
    Tampered {
        /// The plugin id.
        id: String,
    },
    /// The store could not be read.
    #[error("{}", PackMessage::StoreUnreadable { id: String::from(.id) })]
    StoreUnreadable {
        /// The plugin id.
        id: String,
        /// The store failure.
        #[source]
        source: Box<SyncError>,
    },
    /// The plugin targets an extension API this build does not implement.
    #[error("{}", PackMessage::ApiUnsupported { id: String::from(.id), api: ToString::to_string(.api) })]
    ApiUnsupported {
        /// The plugin id.
        id: String,
        /// The API the plugin targets.
        api: u64,
    },
    /// The installed manifest disagrees with the lock entry.
    #[error("{}", PackMessage::ManifestMismatch { id: String::from(.id), field: String::from(*.field) })]
    ManifestMismatch {
        /// The plugin id.
        id: String,
        /// The field that differs: `id`, `version` or `kind`.
        field: &'static str,
    },
    /// Plugins require each other in a loop.
    #[error("{}", PackMessage::RequiresCycle { cycle: crate::messages::join_cycle(.cycle) })]
    RequiresCycle {
        /// The plugin ids on the loop, the first repeated at the end.
        cycle: Vec<String>,
    },
    /// A required plugin is not loaded at a version that satisfies the requirement.
    #[error("{}", PackMessage::RequirementUnmet { id: String::from(.id), needs: String::from(.needs), requirement: String::from(.requirement) })]
    RequirementUnmet {
        /// The plugin that requires.
        id: String,
        /// The required plugin.
        needs: String,
        /// The version requirement.
        requirement: String,
    },
}

impl SyncedError {
    /// The stable code of this error, read from the message catalog. A wrapped [`PackError`]
    /// reports its own code, and `None` when it has none.
    #[must_use]
    pub fn code(&self) -> Option<&'static str> {
        use crate::messages::code_of;
        Some(match self {
            Self::Pack(e) => return e.code(),
            Self::NotLocked { .. } => code_of!(NotLocked { id }),
            Self::NotInstalled { .. } => code_of!(NotInstalled { id }),
            Self::Tampered { .. } => code_of!(Tampered { id }),
            Self::StoreUnreadable { .. } => code_of!(StoreUnreadable { id }),
            Self::ApiUnsupported { .. } => code_of!(ApiUnsupported { id, api }),
            Self::ManifestMismatch { .. } => code_of!(ManifestMismatch { id, field }),
            Self::RequiresCycle { .. } => code_of!(RequiresCycle { cycle }),
            Self::RequirementUnmet { .. } => {
                code_of!(RequirementUnmet {
                    id,
                    needs,
                    requirement
                })
            }
        })
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
    load_synced_within(requested, lock, store, &mut Budget::default())
}

/// Like [`load_synced`], and charges `budget` as each pack loads: file bytes as they are read,
/// rules before they are compiled, and pattern bytes after. The load stops at the first pack
/// that crosses a budget, so no later pack is read. A caller that loaded the core pack through
/// the same `budget` makes the budget cover the whole run.
///
/// # Errors
/// As [`load_synced`], and [`PackError::Limit`] or [`PackError::RunLimit`] (wrapped in
/// [`SyncedError::Pack`]) when a budget is crossed.
pub fn load_synced_within(
    requested: &[PackId],
    lock: &Lockfile,
    store: &Store,
    budget: &mut Budget,
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
        let pack = LoadedPack::load_within(&DirSource::new(dir), origin, budget)?;
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
        Self::with_synced_within(requested, lock, store, Limits::default())
    }

    /// Like [`Packs::with_synced`], under `limits` for the run. The core pack counts toward the
    /// same budget as the synced plugins.
    ///
    /// # Errors
    /// As [`Packs::with_synced`].
    pub fn with_synced_within(
        requested: &[PackId],
        lock: &Lockfile,
        store: &Store,
        limits: Limits,
    ) -> Result<Self, SyncedError> {
        let mut budget = Budget::new(limits);
        let mut packs = vec![LoadedPack::load_within(
            &crate::core_source(),
            Source::Core,
            &mut budget,
        )?];
        packs.extend(load_synced_within(requested, lock, store, &mut budget)?);
        Ok(Self::new_within(packs, limits)?)
    }
}
