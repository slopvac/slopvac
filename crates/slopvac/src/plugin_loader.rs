//! The loader that gives `plugins trust` and `plugins status` the files of a code plugin.
//!
//! It reads the lock entry, has the store hash the installed tree again, and hands the tree to
//! [`slopvac_packs::load_code_plugin`]. A plugin that is not locked, not installed or changed
//! since sync is a refusal, never a load from a tree that fails its digest.

use std::path::PathBuf;

use slopvac_fs::{read_capped_regular_to_string, ReadOptions};
use slopvac_packs::{load_code_plugin, CodePluginContext, SyncedError};
use slopvac_plugins::extract::tree_hash;
use slopvac_plugins::host::PluginSpec;
use slopvac_plugins::lock::{LockedPack, Lockfile, PackId, MAX_LOCK_BYTES};
use slopvac_plugins::sign::KeyPolicy;
use slopvac_plugins::sync::{Store, TreeState};
use slopvac_plugins::trust::VerifiedPlugin;
use slopvac_plugins::PluginLoader;

/// The `PATH` of a plugin child. The host never passes its own, and the value enters the trust
/// hash, so it is the same on every machine: the system directories only.
const PLUGIN_PATH: &str = "/usr/bin:/bin";

/// Loads code plugins from the lock of one project and the plugin store.
pub(crate) struct InstalledPlugins<'a> {
    lock_path: PathBuf,
    store: Store,
    policy: &'a KeyPolicy,
}

impl<'a> InstalledPlugins<'a> {
    /// A loader over `<project_root>/.slopvac/plugins.lock` and the store at `store_dir`.
    pub(crate) fn new(
        project_root: &std::path::Path,
        store_dir: &std::path::Path,
        policy: &'a KeyPolicy,
    ) -> Self {
        Self {
            lock_path: project_root.join(".slopvac/plugins.lock"),
            store: Store::new(store_dir),
            policy,
        }
    }

    fn locked(&self, id: &PackId) -> Option<LockedPack> {
        let text = read_capped_regular_to_string(
            &self.lock_path,
            MAX_LOCK_BYTES as u64,
            ReadOptions::default(),
        )
        .ok()?;
        Lockfile::parse(&text).ok()?.get(id).cloned()
    }

    /// The fingerprint of the key that signed `locked`, when the policy holds that key.
    fn fingerprint(&self, locked: &LockedPack) -> Option<String> {
        let key_id = locked.signature.as_ref()?.key_id.as_str();
        self.policy
            .first_party
            .iter()
            .chain(self.policy.pinned.values().flatten())
            .find(|key| key.key_id() == key_id)
            .map(slopvac_plugins::sign::PublicKey::fingerprint)
    }
}

impl PluginLoader for InstalledPlugins<'_> {
    fn load(&self, id: &PackId) -> Result<(VerifiedPlugin, PluginSpec), String> {
        let name = id.to_string();
        let refusal = |error: SyncedError| error.to_string();
        let locked = self
            .locked(id)
            .ok_or_else(|| refusal(SyncedError::NotLocked { id: name.clone() }))?;
        let root = match self.store.inspect(&locked.digest) {
            Ok(TreeState::Intact(dir)) => dir,
            Ok(TreeState::Absent) => {
                return Err(refusal(SyncedError::NotInstalled { id: name }));
            }
            Ok(TreeState::Tampered(_)) => {
                return Err(refusal(SyncedError::Tampered { id: name }));
            }
            Err(source) => {
                return Err(refusal(SyncedError::StoreUnreadable {
                    id: name,
                    source: Box::new(source),
                }));
            }
        };
        // The store checked this tree a moment ago; a tree that cannot be hashed now changed.
        let tree = tree_hash(&root).map_err(|_| refusal(SyncedError::Tampered { id: name }))?;
        let mut context = CodePluginContext::new(tree, PLUGIN_PATH);
        if let Some(fingerprint) = self.fingerprint(&locked) {
            context = context.with_key_fingerprint(fingerprint);
        }
        load_code_plugin(&locked, &root, &context).map_err(|e| e.to_string())
    }
}
