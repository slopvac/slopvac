//! The loader that gives `plugins trust` and `plugins status` the files of a code plugin.
//!
//! It reads the lock entry, finds where sync left the tree (the store, or the project directory
//! for a path source) and pins the load to a hash derived from the lock: the locked digest, or
//! the hash of the cached archive that the digest names. The cache is not trusted (threat model
//! E8), so no record in the store sets that hash. One pass hashes the tree and keeps the
//! manifest bytes it read, and the load parses only those bytes: a tree that hashes to anything
//! else is a refusal before any parse.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use slopvac_packs::{CodePluginContext, CodePluginError, SyncedError, load_code_plugin};
use slopvac_plugins::extract::{ExtractError, Limits, TreeHash, tree_hash_capturing};
use slopvac_plugins::host::PluginSpec;
use slopvac_plugins::lock::{LockedPack, SourceKind};
use slopvac_plugins::sign::{KeyPolicy, PublicKey};
use slopvac_plugins::sync::{Store, SyncError};
use slopvac_plugins::trust::VerifiedPlugin;
use slopvac_plugins::{LoadError, PluginLoader};

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
}

impl LoadRefusal {
    /// The stable code of the text of this refusal.
    fn code(&self) -> &'static str {
        match self {
            Self::Synced(e) => e.code().unwrap_or("plugin-not-loaded"),
            Self::Code(e) => e.code(),
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

    /// The directory sync left for `locked` and the tree hash the lock pins for it. A path
    /// source that sync used in place has the hash in the lock; any other source has it derived
    /// from the lock and the cached archive.
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
        // STUB(slopvac-8l4.9.28): no executable set is configured yet, so sync and the loader
        // both use the empty one.
        match self
            .store
            .verified_tree(locked, &HashSet::new(), Limits::default())
        {
            Ok(Some(tree)) => Ok((tree.dir, tree.hash)),
            Ok(None) => Err(SyncedError::NotInstalled { id }.into()),
            Err(SyncError::DigestMismatch { .. }) => Err(SyncedError::Tampered { id }.into()),
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
        locked: &LockedPack,
    ) -> Result<(VerifiedPlugin, PluginSpec), LoadRefusal> {
        let name = locked.id.to_string();
        let (root, expected) = self.synced_tree(locked)?;
        // One pass hashes the tree and keeps the manifest bytes it read. The hash is checked
        // before anything is parsed, and the parse reads only the kept bytes.
        let (hash, manifest) = match tree_hash_capturing(&root, "slopvac-plugin.toml") {
            Ok(done) => done,
            Err(ExtractError::CaptureLimit { cap, .. }) => {
                return Err(SyncedError::TooLarge { id: name, cap }.into());
            }
            // A failed read, a link or another file that is not regular: the walk did not
            // finish, so nothing was compared and this is not tampering.
            Err(source) => {
                return Err(SyncedError::StoreUnreadable {
                    id: name,
                    source: Box::new(SyncError::Extract {
                        id: locked.id.clone(),
                        source,
                    }),
                }
                .into());
            }
        };
        if hash != expected {
            return Err(SyncedError::Tampered { id: name }.into());
        }
        let mut context = CodePluginContext::new(expected, PLUGIN_PATH);
        if let Some(fingerprint) = self.fingerprint(locked) {
            context = context.with_key_fingerprint(fingerprint);
        }
        Ok(load_code_plugin(
            locked,
            &root,
            manifest.as_deref(),
            &context,
        )?)
    }
}

impl PluginLoader for InstalledPlugins<'_> {
    /// The command withholds the text of a refusal, so the cause is logged here, as a code and a
    /// plugin id only.
    fn load(&self, locked: &LockedPack) -> Result<(VerifiedPlugin, PluginSpec), LoadError> {
        self.try_load(locked).map_err(|refusal| {
            tracing::debug!(code = refusal.code(), plugin = %locked.id, "plugin loader refused");
            let error = LoadError::new(refusal.to_string());
            // A cache that cannot be read is reported under its own code, with the sentence the
            // catalog gives, which names the plugin and no path.
            if matches!(
                refusal,
                LoadRefusal::Synced(SyncedError::StoreUnreadable { .. })
            ) {
                return error.with_code(refusal.code());
            }
            error
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
    use std::collections::HashSet;
    use std::fs;
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt;

    use rstest::rstest;
    use slopvac_plugins::extract::{Extracted, Limits, extract_targz};
    use slopvac_plugins::lock::read_project_lock;
    use slopvac_plugins::oci::Digest;

    /// The tree hash of the fixture below (`bin/fmt` empty, `PLUGIN_TOML` as the manifest),
    /// worked out by hand from the hash format in the threat model and not by the code under
    /// test.
    const FMT_TREE: &str =
        "sha256:d37e9bde3d46a84776306e32374fad0485d40b2f9bb885ebfe7de720dc91b34a";

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
            fs::write(
                self.lock_path(),
                format!(
                    "schema_version = 3\n[[plugins]]\nid = \"fmt\"\nkind = \"code\"\nversion = \"1.0.0\"\n\
                     slopvac_api = 1\nsource = \"plugins/fmt\"\ndigest = \"{FMT_TREE}\"\n"
                ),
            )
            .unwrap();
            FMT_TREE.to_owned()
        }

        fn store(&self) -> PathBuf {
            self.dir.path().join("store")
        }

        /// A lock with one code plugin from `source` pinned to `digest`.
        fn lock(&self, source: &str, digest: &str) {
            fs::write(
                self.lock_path(),
                format!(
                    "schema_version = 3\n[[plugins]]\nid = \"fmt\"\nkind = \"code\"\nversion = \"1.0.0\"\n\
                     slopvac_api = 1\nsource = \"{source}\"\ndigest = \"{digest}\"\n"
                ),
            )
            .unwrap();
        }

        /// The files of `archive` as the tree that sync keeps under `name`.
        fn install_as(&self, archive: &[u8], name: &TreeHash) {
            let done = extract(archive, &self.dir.path().join("scratch"));
            fs::set_permissions(&done.root, fs::Permissions::from_mode(0o755)).unwrap();
            let trees = self.store().join("trees");
            fs::create_dir_all(&trees).unwrap();
            fs::rename(&done.root, trees.join(format!("tree-{}", name.hex()))).unwrap();
        }

        fn loader(&self) -> InstalledPlugins<'_> {
            InstalledPlugins::new(
                &self.project(),
                &self.dir.path().join("store"),
                &self.policy,
            )
        }

        /// The lock entry of `fmt`, as the command reads it once and hands it to the loader.
        fn fmt(&self) -> LockedPack {
            let lock = read_project_lock(&self.project(), false).unwrap();
            lock.get(&"fmt".parse().unwrap()).unwrap().clone()
        }
    }

    fn extract(archive: &[u8], store: &Path) -> Extracted {
        extract_targz(archive, store, &HashSet::<String>::new(), Limits::default()).unwrap()
    }

    /// A `.tar.gz` of a plugin with this manifest and an empty script.
    fn plugin_archive(manifest: &str) -> Vec<u8> {
        let mut tar = tar::Builder::new(Vec::new());
        for (name, bytes) in [
            ("bin/fmt", &b""[..]),
            ("slopvac-plugin.toml", manifest.as_bytes()),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            tar.append_data(&mut header, name, bytes).unwrap();
        }
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&tar.into_inner().unwrap()).unwrap();
        gz.finish().unwrap()
    }

    #[derive(Debug, Clone, Copy)]
    enum Origin {
        Git,
        Archive,
    }

    /// An attacker who can write the store puts an evil tree where the locked one belongs and
    /// writes a record that points at a second copy of it. Nothing the store holds may set the
    /// hash that trust is pinned to.
    #[rstest]
    #[case::git(Origin::Git)]
    #[case::archive(Origin::Archive)]
    fn a_swapped_tree_with_a_forged_record_is_refused_as_tampered(#[case] origin: Origin) {
        let world = World::new();
        let honest = plugin_archive(PLUGIN_TOML);
        let evil = plugin_archive(&PLUGIN_TOML.replace("--serve", "--evil"));
        let honest_tree = extract(&honest, &world.dir.path().join("honest")).hash;
        let (source, digest) = match origin {
            Origin::Git => (
                format!(
                    "git+https://example.invalid/acme/fmt.git@{}",
                    "0".repeat(40)
                ),
                honest_tree.as_str().to_owned(),
            ),
            Origin::Archive => {
                let digest = Digest::of(&honest);
                let blobs = world.store().join("blobs");
                fs::create_dir_all(&blobs).unwrap();
                fs::write(blobs.join(digest.hex()), &honest).unwrap();
                ("plugins/fmt.tar.gz".to_owned(), digest.to_string())
            }
        };
        world.lock(&source, &digest);
        world.install_as(&evil, &honest_tree);
        let forged = extract(&evil, &world.store().join("trees")).hash;
        let refs = world.store().join("refs");
        fs::create_dir_all(&refs).unwrap();
        fs::write(refs.join(&digest[7..]), forged.hex()).unwrap();

        let refusal = world.loader().try_load(&world.fmt()).unwrap_err();

        assert!(
            matches!(refusal, LoadRefusal::Synced(SyncedError::Tampered { .. })),
            "{refusal:?}"
        );
    }

    /// The hash comes before any parse, so a manifest broken after sync is tampering and not a
    /// manifest error.
    #[test]
    fn a_manifest_broken_after_sync_is_refused_as_tampered_before_it_is_parsed() {
        let world = World::new();
        world.path_plugin();
        fs::write(
            world.project().join("plugins/fmt/slopvac-plugin.toml"),
            "not toml [[[",
        )
        .unwrap();
        let refusal = world.loader().try_load(&world.fmt()).unwrap_err();
        assert!(
            matches!(refusal, LoadRefusal::Synced(SyncedError::Tampered { .. })),
            "{refusal:?}"
        );
    }

    /// A failed read is a failure of the store, not evidence of tampering.
    #[test]
    fn a_file_that_cannot_be_read_is_a_store_failure_and_not_tampering() {
        let world = World::new();
        world.path_plugin();
        let script = world.project().join("plugins/fmt/bin/fmt");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::File::open(&script).is_ok() {
            return; // the test runs with rights that ignore modes
        }
        let refusal = world.loader().try_load(&world.fmt()).unwrap_err();
        assert!(
            matches!(
                refusal,
                LoadRefusal::Synced(SyncedError::StoreUnreadable { .. })
            ),
            "{refusal:?}"
        );
    }

    /// A link is not a regular file, so the walk cannot finish: nothing was compared.
    #[test]
    fn a_link_planted_in_an_in_place_plugin_is_a_store_failure_and_not_tampering() {
        let world = World::new();
        world.path_plugin();
        std::os::unix::fs::symlink(
            "slopvac-plugin.toml",
            world.project().join("plugins/fmt/link"),
        )
        .unwrap();
        let refusal = world.loader().try_load(&world.fmt()).unwrap_err();
        assert!(
            matches!(
                refusal,
                LoadRefusal::Synced(SyncedError::StoreUnreadable { .. })
            ),
            "{refusal:?}"
        );
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
        let refusal = world.loader().try_load(&world.fmt()).unwrap_err();
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
        let (plugin, _) = world.loader().try_load(&world.fmt()).unwrap();
        assert_eq!(plugin.tree.as_str(), digest);
    }

    #[test]
    fn an_in_place_plugin_that_changed_since_the_lock_is_refused_as_tampered() {
        let world = World::new();
        world.path_plugin();
        let manifest = world.project().join("plugins/fmt/slopvac-plugin.toml");
        fs::write(&manifest, PLUGIN_TOML.replace("--serve", "--other")).unwrap();
        let refusal = world.loader().try_load(&world.fmt()).unwrap_err();
        assert!(
            matches!(refusal, LoadRefusal::Synced(SyncedError::Tampered { .. })),
            "{refusal:?}"
        );
    }
}
