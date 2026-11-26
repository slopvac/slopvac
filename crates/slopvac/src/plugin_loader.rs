//! The loader that gives `plugins trust` and `plugins status` the files of a code plugin.
//!
//! It reads the lock entry, finds where sync left the tree (the store, or the project directory
//! for a path source) and pins the load to the hash sync recorded for it. The trusted tree hash
//! is that recorded hash, never one computed here: a tree that hashes to anything else, before
//! or after the manifest is read, is a refusal.

use std::path::{Path, PathBuf};

use slopvac_packs::{load_code_plugin, CodePluginContext, CodePluginError, SyncedError};
use slopvac_plugins::extract::{tree_hash, TreeHash};
use slopvac_plugins::host::PluginSpec;
use slopvac_plugins::lock::{read_project_lock, LockedPack, PackId, SourceKind};
use slopvac_plugins::sign::{KeyPolicy, PublicKey};
use slopvac_plugins::sync::{Store, TreeState};
use slopvac_plugins::trust::VerifiedPlugin;
use slopvac_plugins::{PluginLoader, RunError};

/// The `PATH` of a plugin child. The host never passes its own, and the value enters the trust
/// hash, so it is the same on every machine: the system directories only.
const PLUGIN_PATH: &str = "/usr/bin:/bin";

/// Why a plugin did not load. Each cause renders catalog text and has a stable code.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LoadRefusal {
    /// The plugin is not locked, not installed or changed since sync.
    #[error(transparent)]
    Synced(#[from] SyncedError),
    /// The installed files break the plugin contract.
    #[error(transparent)]
    Code(#[from] CodePluginError),
    /// The lock could not be read or is not valid.
    #[error(transparent)]
    Lock(#[from] RunError),
}

impl LoadRefusal {
    /// The stable code of the text of this refusal.
    fn code(&self) -> &'static str {
        match self {
            Self::Synced(e) => e.code().unwrap_or("plugin-not-loaded"),
            Self::Code(e) => e.code(),
            Self::Lock(e) => e.code(),
        }
    }
}

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

    /// The directory sync left for `locked` and the tree hash it recorded. A path source that
    /// sync used in place has no record of its own: the lock pins its hash.
    fn synced_tree(&self, locked: &LockedPack) -> Result<(PathBuf, TreeHash), LoadRefusal> {
        let id = locked.id.to_string();
        if let SourceKind::Path(rel) = locked.source.kind() {
            let dir = self.project_root.join(rel);
            if dir.is_dir() {
                let pinned = TreeHash::parse(locked.digest.as_str())
                    .map_err(|_| SyncedError::Tampered { id })?;
                return Ok((dir, pinned));
            }
        }
        match self.store.inspect(&locked.digest) {
            Ok(TreeState::Intact(dir)) => self
                .store
                .recorded(&locked.digest)
                .map(|recorded| (dir, recorded))
                .ok_or_else(|| SyncedError::NotInstalled { id }.into()),
            Ok(TreeState::Absent) => Err(SyncedError::NotInstalled { id }.into()),
            Ok(TreeState::Tampered(_)) => Err(SyncedError::Tampered { id }.into()),
            Err(source) => Err(SyncedError::StoreUnreadable {
                id,
                source: Box::new(source),
            }
            .into()),
        }
    }

    /// Load the plugin with the cause of a refusal kept typed.
    pub(crate) fn try_load(
        &self,
        id: &PackId,
    ) -> Result<(VerifiedPlugin, PluginSpec), LoadRefusal> {
        let name = id.to_string();
        let lock = read_project_lock(&self.project_root, false)?;
        let locked = lock
            .get(id)
            .ok_or_else(|| SyncedError::NotLocked { id: name.clone() })?;
        let (root, recorded) = self.synced_tree(locked)?;
        let mut context = CodePluginContext::new(recorded.clone(), PLUGIN_PATH);
        if let Some(fingerprint) = self.fingerprint(locked) {
            context = context.with_key_fingerprint(fingerprint);
        }
        let loaded = load_code_plugin(locked, &root, &context)?;
        // Hash after the manifest was read: the files the load saw are the files that were
        // recorded, so trust is never pinned to a tree that changed under the loader.
        match tree_hash(&root) {
            Ok(fresh) if fresh == recorded => Ok(loaded),
            _ => Err(SyncedError::Tampered { id: name }.into()),
        }
    }
}

impl PluginLoader for InstalledPlugins<'_> {
    /// The command withholds the text of a refusal, so the cause is logged here, as a code and a
    /// plugin id only.
    fn load(&self, id: &PackId) -> Result<(VerifiedPlugin, PluginSpec), String> {
        self.try_load(id).map_err(|refusal| {
            tracing::debug!(code = refusal.code(), plugin = %id, "plugin loader refused");
            refusal.to_string()
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: a panic is a failed test"
)]
mod tests {
    use super::*;
    use std::fs;

    const PLUGIN_TOML: &str = "schema_version = 3\nid = \"fmt\"\nversion = \"1.0.0\"\n\
        slopvac_api = 1\nkind = \"code\"\n[provides]\n[plugin]\ncommand = \"bin/fmt\"\n\
        args = [\"--serve\"]\nnetwork = false\nhooks = []\n\
        features = { required = [], optional = [] }\n";

    struct World {
        dir: tempfile::TempDir,
        policy: KeyPolicy,
    }

    impl World {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir_all(dir.path().join("project/.slopvac")).unwrap();
            fs::create_dir_all(dir.path().join("store")).unwrap();
            Self {
                dir,
                policy: KeyPolicy::compiled().unwrap(),
            }
        }

        fn project(&self) -> PathBuf {
            self.dir.path().join("project")
        }

        fn lock_path(&self) -> PathBuf {
            self.project().join(".slopvac/plugins.lock")
        }

        /// A code plugin in the project at `plugins/fmt`, locked by its tree hash. Returns that hash.
        fn path_plugin(&self) -> String {
            let plugin = self.project().join("plugins/fmt");
            fs::create_dir_all(plugin.join("bin")).unwrap();
            fs::write(plugin.join("slopvac-plugin.toml"), PLUGIN_TOML).unwrap();
            fs::write(plugin.join("bin/fmt"), "").unwrap();
            let digest = tree_hash(&plugin).unwrap().as_str().to_owned();
            fs::write(
                self.lock_path(),
                format!(
                    "schema_version = 3\n[[plugins]]\nid = \"fmt\"\nkind = \"code\"\nversion = \"1.0.0\"\n\
                     slopvac_api = 1\nsource = \"plugins/fmt\"\ndigest = \"{digest}\"\n"
                ),
            )
            .unwrap();
            digest
        }

        fn loader(&self) -> InstalledPlugins<'_> {
            InstalledPlugins::new(
                &self.project(),
                &self.dir.path().join("store"),
                &self.policy,
            )
        }

        fn fmt() -> PackId {
            "fmt".parse().unwrap()
        }
    }

    #[test]
    fn a_locked_code_plugin_with_no_installed_tree_is_refused_as_not_installed() {
        let world = World::new();
        fs::write(
            world.lock_path(),
            "schema_version = 3\n[[plugins]]\nid = \"fmt\"\nkind = \"code\"\nversion = \"1.0.0\"\n\
             slopvac_api = 1\nsource = \"oci://registry.invalid/acme/fmt:1.0.0\"\n\
             digest = \"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"\n",
        )
        .unwrap();
        let refusal = world.loader().try_load(&World::fmt()).unwrap_err();
        assert!(
            matches!(
                refusal,
                LoadRefusal::Synced(SyncedError::NotInstalled { .. })
            ),
            "{refusal:?}"
        );
    }

    #[test]
    fn an_intact_in_place_plugin_carries_the_hash_the_lock_pins() {
        let world = World::new();
        let digest = world.path_plugin();
        let (plugin, _) = world.loader().try_load(&World::fmt()).unwrap();
        assert_eq!(plugin.tree.as_str(), digest);
    }

    #[test]
    fn an_in_place_plugin_that_changed_since_the_lock_is_refused_as_tampered() {
        let world = World::new();
        world.path_plugin();
        let manifest = world.project().join("plugins/fmt/slopvac-plugin.toml");
        fs::write(&manifest, PLUGIN_TOML.replace("--serve", "--other")).unwrap();
        let refusal = world.loader().try_load(&World::fmt()).unwrap_err();
        assert!(
            matches!(refusal, LoadRefusal::Synced(SyncedError::Tampered { .. })),
            "{refusal:?}"
        );
    }

    #[test]
    fn an_unreadable_lock_is_refused_as_unreadable_and_not_as_unlocked() {
        let world = World::new();
        fs::create_dir_all(world.lock_path()).unwrap();
        let refusal = world.loader().try_load(&World::fmt()).unwrap_err();
        assert!(
            matches!(refusal, LoadRefusal::Lock(RunError::LockUnreadable { .. })),
            "{refusal:?}"
        );
    }

    #[test]
    fn an_invalid_lock_is_refused_as_invalid_and_not_as_unlocked() {
        let world = World::new();
        fs::write(world.lock_path(), "schema_version = [\n").unwrap();
        let refusal = world.loader().try_load(&World::fmt()).unwrap_err();
        assert!(
            matches!(refusal, LoadRefusal::Lock(RunError::LockInvalid { .. })),
            "{refusal:?}"
        );
    }
}
