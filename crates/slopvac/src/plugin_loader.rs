//! The loader that gives `plugins trust` and `plugins status` the files of a code plugin.
//!
//! It reads the lock entry, finds where sync left the tree (the store, or the project directory
//! for a path source) and checks it against the lock digest, then hands it to
//! [`slopvac_packs::load_code_plugin`]. A plugin that is not locked, not installed or changed
//! since sync is a refusal, never a load from a tree that fails its digest.

use std::path::{Path, PathBuf};

use slopvac_fs::{ReadOptions, read_capped_regular_to_string};
use slopvac_packs::{CodePluginContext, SyncedError, load_code_plugin};
use slopvac_plugins::PluginLoader;
use slopvac_plugins::extract::tree_hash;
use slopvac_plugins::host::PluginSpec;
use slopvac_plugins::lock::{LockedPack, Lockfile, MAX_LOCK_BYTES, PackId};
use slopvac_plugins::sign::{KeyPolicy, PublicKey};
use slopvac_plugins::sync::{PackState, Store, status};
use slopvac_plugins::trust::VerifiedPlugin;

/// The `PATH` of a plugin child. The host never passes its own, and the value enters the trust
/// hash, so it is the same on every machine: the system directories only.
const PLUGIN_PATH: &str = "/usr/bin:/bin";

/// Loads code plugins from the lock of one project and the plugin store.
pub(crate) struct InstalledPlugins<'a> {
    project_root: PathBuf,
    store: Store,
    policy: &'a KeyPolicy,
}

impl<'a> InstalledPlugins<'a> {
    /// A loader over `<project_root>/.slopvac/plugins.lock` and the store at `store_dir`.
    pub(crate) fn new(project_root: &Path, store_dir: &Path, policy: &'a KeyPolicy) -> Self {
        Self {
            project_root: project_root.to_path_buf(),
            store: Store::new(store_dir),
            policy,
        }
    }

    fn lock(&self) -> Option<Lockfile> {
        let text = read_capped_regular_to_string(
            &self.project_root.join(".slopvac/plugins.lock"),
            MAX_LOCK_BYTES as u64,
            ReadOptions::default(),
        )
        .ok()?;
        Lockfile::parse(&text).ok()
    }

    /// The fingerprint of the key that signed `locked`, when the policy holds that key.
    fn fingerprint(&self, locked: &LockedPack) -> Option<String> {
        let key_id = locked.signature.as_ref()?.key_id.as_str();
        self.policy
            .first_party
            .iter()
            .chain(self.policy.pinned.values().flatten())
            .find(|key| key.key_id() == key_id)
            .map(PublicKey::fingerprint)
    }
}

impl PluginLoader for InstalledPlugins<'_> {
    fn load(&self, id: &PackId) -> Result<(VerifiedPlugin, PluginSpec), String> {
        let name = id.to_string();
        let refusal = |error: SyncedError| error.to_string();
        let not_locked = || refusal(SyncedError::NotLocked { id: name.clone() });
        let lock = self.lock().ok_or_else(not_locked)?;
        let locked = lock.get(id).ok_or_else(not_locked)?;
        let state = status(&self.store, &self.project_root, &lock)
            .map_err(|source| {
                refusal(SyncedError::StoreUnreadable {
                    id: name.clone(),
                    source: Box::new(source),
                })
            })?
            .into_iter()
            .find(|s| &s.id == id)
            .map(|s| s.state);
        let root = match state {
            Some(PackState::Recorded(dir) | PackState::InPlace(dir)) => dir,
            Some(PackState::Tampered(_) | PackState::InPlaceChanged(_)) => {
                return Err(refusal(SyncedError::Tampered { id: name }));
            }
            Some(PackState::NotInstalled) | None => {
                return Err(refusal(SyncedError::NotInstalled { id: name }));
            }
        };
        // The state above checked this tree a moment ago; one that cannot be hashed now changed.
        let tree = tree_hash(&root).map_err(|_| refusal(SyncedError::Tampered { id: name }))?;
        let mut context = CodePluginContext::new(tree, PLUGIN_PATH);
        if let Some(fingerprint) = self.fingerprint(locked) {
            context = context.with_key_fingerprint(fingerprint);
        }
        load_code_plugin(locked, &root, &context).map_err(|e| e.to_string())
    }
}
