//! A synced data plugin in a namespace with a pinned key loads only when its cached signature
//! verifies against that key. The loader reads the signer from the verified bytes, not from
//! the lock.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

#[path = "../../slopvac-plugins/tests/signed_fixture/mod.rs"]
mod signed_fixture;

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use slopvac_model::PackId;
use slopvac_packs::{SyncedError, load_synced};
use slopvac_plugins::extract::{Limits, extract_targz};
use slopvac_plugins::lock::Lockfile;
use slopvac_plugins::sign::{KeyPolicy, PinScope, PublicKey};
use slopvac_plugins::sync::Store;

const REGISTRY: &str = "registry.example";
const NOT_BEFORE: &str = "2026-11-01T00:00:00Z";
const MANIFEST: &str = "schema_version = 3\nid = \"rulepack\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
    kind = \"data\"\n\n[provides]\ncategories = [\"categories/c.json\"]\n";
const CATEGORY: &str = r#"{"schema_version":3,"id":"c","title":"T","description":"D","rules":[{"id":"r0","name":"N","message":"m","match":{"kind":"pattern","pattern":"zorblax"},"citation":{"source":"s"}}]}"#;

/// A store that holds `rulepack` the way a sync of the artifact `key` signed leaves it: the
/// manifest, the layer, the statement and the signature in the blobs, and the tree.
struct World {
    dir: tempfile::TempDir,
    published: signed_fixture::Published,
}

impl World {
    fn signed_by(key: &signed_fixture::Key) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let files: [(&str, &[u8]); 2] = [
            ("slopvac-plugin.toml", MANIFEST.as_bytes()),
            ("categories/c.json", CATEGORY.as_bytes()),
        ];
        let published =
            signed_fixture::publish(key, "acme", "rulepack", "1.0.0", NOT_BEFORE, &files);
        let store = dir.path().join("store");
        published.cache_into(&store);
        let done = extract_targz(
            &signed_fixture::targz(&files)[..],
            &dir.path().join("scratch"),
            &HashSet::<String>::new(),
            Limits::default(),
        )
        .unwrap();
        fs::set_permissions(&done.root, fs::Permissions::from_mode(0o755)).unwrap();
        fs::create_dir_all(store.join("trees")).unwrap();
        fs::rename(
            &done.root,
            store
                .join("trees")
                .join(format!("tree-{}", done.hash.hex())),
        )
        .unwrap();
        Self { dir, published }
    }

    fn store(&self) -> Store {
        Store::new(self.dir.path().join("store"))
    }

    /// The lock entry of `rulepack`, with `signature` as its signature table.
    fn lock(&self, signature: &str) -> Lockfile {
        Lockfile::parse(&format!(
            "schema_version = 3\n[[plugins]]\nid = \"rulepack\"\nkind = \"data\"\nversion = \"1.0.0\"\n\
             slopvac_api = 1\nsource = \"oci://{REGISTRY}/acme/rulepack:1.0.0\"\n\
             digest = \"{}\"\n{signature}",
            self.published.digest
        ))
        .unwrap()
    }

    /// Every file of the store, by path, with its bytes.
    fn store_bytes(&self) -> BTreeMap<String, Vec<u8>> {
        fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, base, out);
                } else {
                    let name = path.strip_prefix(base).unwrap().display().to_string();
                    out.insert(name, fs::read(&path).unwrap());
                }
            }
        }
        let mut out = BTreeMap::new();
        let root = self.dir.path().join("store");
        walk(&root, &root, &mut out);
        out
    }
}

fn signature_table(key_id: &str) -> String {
    format!("[plugins.signature]\nkey_id = \"{key_id}\"\nnot_before = \"{NOT_BEFORE}\"\n")
}

/// A policy that pins `keys` for the plugin's registry host and the namespace `acme`.
fn pinned(keys: &[PublicKey]) -> KeyPolicy {
    KeyPolicy {
        pinned: BTreeMap::from([(PinScope::new(REGISTRY, "acme"), keys.to_vec())]),
        ..KeyPolicy::default()
    }
}

fn load(w: &World, lock: &Lockfile, policy: &KeyPolicy) -> Result<Vec<String>, SyncedError> {
    load_synced(
        &[PackId::new("rulepack").unwrap()],
        lock,
        &w.store(),
        policy,
    )
    .map(|packs| packs.iter().map(|p| p.manifest().id.to_string()).collect())
}

#[test]
fn an_intact_entry_loads_under_the_key_pinned_for_its_namespace() {
    let old = signed_fixture::Key::primary();
    let w = World::signed_by(&old);
    let lock = w.lock(&signature_table(&old.public().key_id()));

    let loaded = load(&w, &lock, &pinned(&[old.public()])).unwrap();

    assert_eq!(loaded, ["rulepack"]);
}

/// The lock is project text, so a `signature` table that someone deleted does not turn a
/// signed artifact into one that needs no signature.
#[test]
fn an_entry_whose_lock_records_no_signature_is_refused_as_unsigned_and_loads_nothing() {
    let old = signed_fixture::Key::primary();
    let w = World::signed_by(&old);
    let lock = w.lock("");
    let before = w.store_bytes();

    let result = load(&w, &lock, &pinned(&[old.public()]));

    assert_eq!(result.as_ref().ok(), None, "nothing from the entry loads");
    let err = result.unwrap_err();
    assert_eq!(err.code(), Some("sync-unsigned"));
    assert_eq!(
        err.to_string(),
        "plugin `rulepack`: the lock has no signature for an OCI plugin; pin its key, then update"
    );
    assert_eq!(w.store_bytes(), before, "the load changed the store");
}

/// After a rotation the policy pins only the new key. A lock edited to name the new key does
/// not make the bytes that the old key signed load: the signer is read from the verified bytes.
#[test]
fn an_entry_whose_lock_names_the_new_key_while_the_old_key_signed_it_is_refused() {
    let (old, new) = (
        signed_fixture::Key::primary(),
        signed_fixture::Key::secondary(),
    );
    let w = World::signed_by(&old);
    let lock = w.lock(&signature_table(&new.public().key_id()));
    let before = w.store_bytes();

    let result = load(&w, &lock, &pinned(&[new.public()]));

    assert_eq!(result.as_ref().ok(), None, "nothing from the entry loads");
    let err = result.unwrap_err();
    assert_eq!(err.code(), Some("signature-invalid"));
    assert_eq!(
        err.to_string(),
        format!(
            "plugin `rulepack`: signature check failed: key {} is not pinned for namespace `acme`, \
             which pins another key; run `slopvac plugins update` for a release that key signs",
            old.public().key_id()
        )
    );
    assert_eq!(w.store_bytes(), before, "the load changed the store");
}
