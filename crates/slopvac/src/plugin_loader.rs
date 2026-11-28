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

use slopvac_messages::Catalog;
use slopvac_packs::{
    CodePluginContext, CodePluginError, SyncedError, TooLargePart, load_code_plugin,
};
use slopvac_plugins::extract::{ExtractError, Limits, TreeHash, tree_hash_capturing};
use slopvac_plugins::host::PluginSpec;
use slopvac_plugins::lock::{LockedPack, SourceKind};
use slopvac_plugins::oci::OciError;
use slopvac_plugins::sign::{KeyPolicy, PublicKey};
use slopvac_plugins::sync::{Store, SyncError, pinned_signer};
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
    /// A signed entry whose OCI source does not parse. Sync parses the same source before it
    /// fetches or verifies anything, so it installed nothing for the entry, and the refusal has
    /// the text and the code of a plugin that is not installed.
    #[error("{}", not_installed(.id))]
    SourceUnparsed {
        /// The plugin id.
        id: String,
        /// The kind of the parse failure.
        #[source]
        cause: SourceCause,
    },
}

/// The refusal of a plugin that sync did not install.
fn not_installed(id: &str) -> SyncedError {
    SyncedError::NotInstalled { id: id.to_owned() }
}

impl LoadRefusal {
    /// The stable code of the text of this refusal.
    fn code(&self) -> &'static str {
        match self {
            Self::Synced(e) => e.code().unwrap_or("plugin-not-loaded"),
            Self::Code(e) => e.code(),
            Self::SourceUnparsed { id, .. } => {
                not_installed(id).code().unwrap_or("plugin-not-loaded")
            }
        }
    }
}

/// Why the OCI source of a signed entry does not parse, as a kind only. The text of an
/// [`OciError`] can name the registry and the source, so none of it is kept (the approved
/// departures in `docs/engineering.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SourceCause {
    /// The source is not a valid OCI reference.
    #[error("{}", LoaderPhrase::SourceInvalid.as_static())]
    Invalid,
    /// The parse failed in another way.
    #[error("{}", LoaderPhrase::SourceUnusable.as_static())]
    Other,
}

impl SourceCause {
    fn of(e: &OciError) -> Self {
        match e {
            OciError::Invalid(_) => Self::Invalid,
            _ => Self::Other,
        }
    }
}

/// Fixed phrases of the code plugin loader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Catalog)]
pub(crate) enum LoaderPhrase {
    /// An OCI source that is not a valid reference.
    #[code("plugin-loader-phrase-source-invalid")]
    #[text("the OCI source is not a valid reference")]
    SourceInvalid,
    /// An OCI source whose parse failed in another way.
    #[code("plugin-loader-phrase-source-unusable")]
    #[text("the OCI source cannot be used")]
    SourceUnusable,
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

    /// The fingerprint of the key that signed `locked`: the key that sync's verification
    /// selected for the registry and namespace of its OCI source. Only an OCI source is signed.
    ///
    /// A signed entry whose source does not parse is refused as not installed: sync parses the
    /// same source before it fetches or verifies anything, so it installed no tree for it.
    fn fingerprint(&self, locked: &LockedPack) -> Result<Option<String>, LoadRefusal> {
        match pinned_signer(locked, self.policy) {
            Ok(key) => Ok(key.map(PublicKey::fingerprint)),
            Err(SyncError::Fetch { source, .. }) => Err(LoadRefusal::SourceUnparsed {
                id: locked.id.to_string(),
                cause: SourceCause::of(&source),
            }),
            Err(e) => Err(SyncedError::Signature(Box::new(e)).into()),
        }
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
        // The pin decides before any file of the tree is read.
        let fingerprint = self.fingerprint(locked)?;
        let (root, expected) = self.synced_tree(locked)?;
        // One pass hashes the tree and keeps the manifest bytes it read. The hash is checked
        // before anything is parsed, and the parse reads only the kept bytes.
        let (hash, manifest) = match tree_hash_capturing(&root, "slopvac-plugin.toml") {
            Ok(done) => done,
            Err(ExtractError::CaptureLimit { cap, .. }) => {
                return Err(SyncedError::TooLarge {
                    id: name,
                    part: TooLargePart::Manifest,
                    cap,
                }
                .into());
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
        if let Some(fingerprint) = fingerprint {
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
            // A cache that cannot be read is reported under its own code. Every other refusal
            // is logged above and reaches the command as an opaque refusal.
            match refusal {
                LoadRefusal::Synced(SyncedError::StoreUnreadable { .. }) => {
                    LoadError::store_unreadable(&locked.id)
                }
                _ => LoadError::from(refusal.to_string()),
            }
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
    use slopvac_plugins::sign::PinScope;

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

    /// A minisign public key with the key id of `key` and other key bytes.
    fn twin_of(key: &minisign::PublicKey) -> String {
        let mut bytes = key.to_bytes();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        minisign::PublicKey::from_bytes(&bytes).unwrap().to_base64()
    }

    /// Two registries pin keys that share one minisign key id. A plugin from the second
    /// registry is recorded with the key that its own registry pins: the key that verification
    /// on that registry selects, never the first key with the id in another scope.
    #[test]
    fn the_recorded_fingerprint_is_the_key_pinned_on_the_plugins_own_registry() {
        let mut world = World::new();
        let pk = minisign::KeyPair::generate_unencrypted_keypair()
            .unwrap()
            .pk;
        let on_ghcr = PublicKey::from_base64(&pk.to_base64()).unwrap();
        let on_example = PublicKey::from_base64(&twin_of(&pk)).unwrap();
        assert_eq!(on_ghcr.key_id(), on_example.key_id());
        assert_ne!(on_ghcr.fingerprint(), on_example.fingerprint());
        world.policy.pinned = std::collections::BTreeMap::from([
            (PinScope::new("ghcr.io", "acme"), vec![on_ghcr]),
            (
                PinScope::new("registry.example", "acme"),
                vec![on_example.clone()],
            ),
        ]);

        // What sync leaves in the store for an OCI plugin: the manifest that the lock pins,
        // its one pack layer, and the tree that sync unpacks from that layer.
        let archive = plugin_archive(PLUGIN_TOML);
        let layer = Digest::of(&archive);
        let manifest = format!(
            "{{\"schemaVersion\":2,\"mediaType\":\"application/vnd.oci.image.manifest.v1+json\",\
             \"config\":{{\"mediaType\":\"application/vnd.oci.empty.v1+json\",\"digest\":\"{}\",\"size\":2}},\
             \"layers\":[{{\"mediaType\":\"application/vnd.slopvac.pack.layer.v1.tar+gzip\",\"digest\":\"{layer}\",\"size\":{}}}]}}",
            Digest::of(b"{}"),
            archive.len()
        );
        let manifest_digest = Digest::of(manifest.as_bytes());
        let blobs = world.store().join("blobs");
        fs::create_dir_all(&blobs).unwrap();
        fs::write(blobs.join(layer.hex()), &archive).unwrap();
        fs::write(blobs.join(manifest_digest.hex()), &manifest).unwrap();
        let tree = extract(&archive, &world.dir.path().join("hash")).hash;
        world.install_as(&archive, &tree);
        fs::write(
            world.lock_path(),
            format!(
                "schema_version = 3\n[[plugins]]\nid = \"fmt\"\nkind = \"code\"\nversion = \"1.0.0\"\n\
                 slopvac_api = 1\nsource = \"oci://registry.example/acme/fmt:1.0.0\"\n\
                 digest = \"{manifest_digest}\"\n[plugins.signature]\nkey_id = \"{}\"\n\
                 not_before = \"2026-11-01T00:00:00Z\"\n",
                on_example.key_id()
            ),
        )
        .unwrap();

        let (plugin, _) = world.loader().try_load(&world.fmt()).unwrap();

        assert_eq!(
            plugin.key_fingerprint.as_deref(),
            Some(on_example.fingerprint().as_str())
        );
    }

    /// A code plugin cached the way sync leaves an OCI entry on `registry.example/acme`, with
    /// a lock entry signed by `key_id`.
    fn cache_signed_fmt(world: &World, key_id: &str) {
        let archive = plugin_archive(PLUGIN_TOML);
        let layer = Digest::of(&archive);
        let manifest = format!(
            "{{\"schemaVersion\":2,\"mediaType\":\"application/vnd.oci.image.manifest.v1+json\",\
             \"config\":{{\"mediaType\":\"application/vnd.oci.empty.v1+json\",\"digest\":\"{}\",\"size\":2}},\
             \"layers\":[{{\"mediaType\":\"application/vnd.slopvac.pack.layer.v1.tar+gzip\",\"digest\":\"{layer}\",\"size\":{}}}]}}",
            Digest::of(b"{}"),
            archive.len()
        );
        let manifest_digest = Digest::of(manifest.as_bytes());
        let blobs = world.store().join("blobs");
        fs::create_dir_all(&blobs).unwrap();
        fs::write(blobs.join(layer.hex()), &archive).unwrap();
        fs::write(blobs.join(manifest_digest.hex()), &manifest).unwrap();
        let tree = extract(&archive, &world.dir.path().join("hash")).hash;
        world.install_as(&archive, &tree);
        fs::write(
            world.lock_path(),
            format!(
                "schema_version = 3\n[[plugins]]\nid = \"fmt\"\nkind = \"code\"\nversion = \"1.0.0\"\n\
                 slopvac_api = 1\nsource = \"oci://registry.example/acme/fmt:1.0.0\"\n\
                 digest = \"{manifest_digest}\"\n[plugins.signature]\nkey_id = \"{key_id}\"\n\
                 not_before = \"2026-11-01T00:00:00Z\"\n"
            ),
        )
        .unwrap();
    }

    /// A signed code plugin loads only while its key is pinned for its registry and namespace.
    /// With no key for the pair the next step is to pin one; with another key pinned, as after
    /// a rotation, it is to update.
    #[rstest]
    #[case::unpinned(
        false,
        "is not pinned for namespace `acme`; pin it with `slopvac plugins pin acme <fingerprint> --key <file>`"
    )]
    #[case::rotated(
        true,
        "is not pinned for namespace `acme`, which pins another key; run `slopvac plugins update` for a release that key signs"
    )]
    fn a_signed_code_plugin_whose_key_is_not_pinned_is_refused(
        #[case] other_key_pinned: bool,
        #[case] next_step: &str,
    ) {
        let mut world = World::new();
        let key = |pair: minisign::KeyPair| PublicKey::from_base64(&pair.pk.to_base64()).unwrap();
        let signer = key(minisign::KeyPair::generate_unencrypted_keypair().unwrap());
        if other_key_pinned {
            let other = key(minisign::KeyPair::generate_unencrypted_keypair().unwrap());
            world.policy.pinned = std::collections::BTreeMap::from([(
                PinScope::new("registry.example", "acme"),
                vec![other],
            )]);
        }
        cache_signed_fmt(&world, &signer.key_id());

        let result = world.loader().try_load(&world.fmt());

        assert!(
            result.is_err(),
            "the plugin loads: {:?}",
            result.map(|(p, _)| p.name)
        );
        let Err(refusal) = result else { return };
        assert_eq!(refusal.code(), "signature-invalid");
        assert_eq!(
            refusal.to_string(),
            format!(
                "plugin `fmt`: signature check failed: key {} {next_step}",
                signer.key_id()
            )
        );
    }

    /// A signed entry whose OCI source does not parse names no registry or namespace, so the
    /// policy holds no key for it. Sync parses that source first and installs nothing for it.
    /// `try_load` returns `NotInstalled` for the entry, so a signed entry loads only with its
    /// fingerprint.
    #[test]
    fn a_signed_entry_whose_oci_source_does_not_parse_is_refused() {
        let mut world = World::new();
        let pk = minisign::KeyPair::generate_unencrypted_keypair()
            .unwrap()
            .pk;
        let key = PublicKey::from_base64(&pk.to_base64()).unwrap();
        world.policy.pinned = std::collections::BTreeMap::from([(
            PinScope::new("registry.example", "acme"),
            vec![key.clone()],
        )]);
        let archive = plugin_archive(PLUGIN_TOML);
        let layer = Digest::of(&archive);
        let manifest = format!(
            "{{\"schemaVersion\":2,\"mediaType\":\"application/vnd.oci.image.manifest.v1+json\",\
             \"config\":{{\"mediaType\":\"application/vnd.oci.empty.v1+json\",\"digest\":\"{}\",\"size\":2}},\
             \"layers\":[{{\"mediaType\":\"application/vnd.slopvac.pack.layer.v1.tar+gzip\",\"digest\":\"{layer}\",\"size\":{}}}]}}",
            Digest::of(b"{}"),
            archive.len()
        );
        let manifest_digest = Digest::of(manifest.as_bytes());
        let blobs = world.store().join("blobs");
        fs::create_dir_all(&blobs).unwrap();
        fs::write(blobs.join(layer.hex()), &archive).unwrap();
        fs::write(blobs.join(manifest_digest.hex()), &manifest).unwrap();
        let tree = extract(&archive, &world.dir.path().join("hash")).hash;
        world.install_as(&archive, &tree);
        // No tag and no digest: the lock reader keeps the text, and a reference needs a target.
        fs::write(
            world.lock_path(),
            format!(
                "schema_version = 3\n[[plugins]]\nid = \"fmt\"\nkind = \"code\"\nversion = \"1.0.0\"\n\
                 slopvac_api = 1\nsource = \"oci://registry.example/acme/fmt\"\n\
                 digest = \"{manifest_digest}\"\n[plugins.signature]\nkey_id = \"{}\"\n\
                 not_before = \"2026-11-01T00:00:00Z\"\n",
                key.key_id()
            ),
        )
        .unwrap();

        let refusal = world.loader().try_load(&world.fmt()).unwrap_err();

        assert!(
            matches!(
                &refusal,
                LoadRefusal::SourceUnparsed { id, cause: SourceCause::Invalid } if id == "fmt"
            ),
            "{refusal:?}"
        );
        assert_eq!(refusal.code(), "plugin-not-installed");
        assert_eq!(
            refusal.to_string(),
            "plugin `fmt` is locked but not installed; run `slopvac plugins sync`"
        );
        // The chain keeps the kind of the parse failure, and no text in it names the source or
        // the registry.
        let source = std::error::Error::source(&refusal).expect("the refusal keeps its cause");
        assert_eq!(
            source.downcast_ref::<SourceCause>(),
            Some(&SourceCause::Invalid),
            "{source:?}"
        );
        let mut next: Option<&dyn std::error::Error> = Some(&refusal);
        while let Some(e) = next {
            let text = e.to_string();
            for named in ["oci://", "registry.example", "acme/fmt"] {
                assert!(!text.contains(named), "{text}");
            }
            next = e.source();
        }
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

    /// The cap that stops a code plugin is the one on its manifest, so the message names the
    /// manifest and not the tree.
    #[test]
    fn a_code_plugin_manifest_over_1_mib_says_the_manifest_is_too_large() {
        let world = World::new();
        world.path_plugin();
        fs::write(
            world.project().join("plugins/fmt/slopvac-plugin.toml"),
            vec![b'#'; (1 << 20) + 1],
        )
        .unwrap();

        let refusal = world.loader().try_load(&world.fmt()).unwrap_err();

        assert_eq!(refusal.code(), "plugin-too-large");
        assert_eq!(
            refusal.to_string(),
            "the manifest `slopvac-plugin.toml` of plugin `fmt` is over the 1048576 byte limit for a load"
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
