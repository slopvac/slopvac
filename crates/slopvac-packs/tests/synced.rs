//! Synced packs load from the config list, the lock and the store.
//!
//! The store is built by hand in the layout its documentation gives (`trees/tree-<hex>` and
//! `refs/<digest hex>`), so the tests need no registry and no signing.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: a panic is a failed test"
)]

use std::fs;
use std::path::{Path, PathBuf};

use proptest::prelude::*;
use rstest::rstest;
use slopvac_config::Presets;
use slopvac_model::PackId;
use slopvac_packs::{
    ArtifactFault, Budget, Limits, LoadedPack, PackError, Packs, Reference, RuleFault, Source,
    SourceError,
    SyncedError, TooLargePart, Trust, TrustQuery, TrustQueryError, TrustSubject, core_source,
    load_synced, load_synced_within,
};
use slopvac_plugins::extract::{TreeHash, tree_hash};
use slopvac_plugins::lock::Lockfile;
use slopvac_plugins::sync::Store;
use slopvac_rules::PackBudget;

const MIB: usize = 1024 * 1024;

mod trust_conformance;

/// A trust store double: the installation recorded as trusted for each plugin id, as its root
/// and tree hash. A plugin with no record is untrusted, and one recorded at another root or
/// hash changed after trust, as the spawn gate reports a moved tree. A failing store answers
/// nothing.
struct Recorded {
    records: Vec<(String, PathBuf, TreeHash)>,
    fails: bool,
}

impl Recorded {
    const fn none() -> Self {
        Self {
            records: Vec::new(),
            fails: false,
        }
    }

    fn trusting(id: &str, root: &Path, tree: TreeHash) -> Self {
        Self {
            records: vec![(id.to_owned(), root.to_owned(), tree)],
            fails: false,
        }
    }

    const fn failing() -> Self {
        Self {
            records: Vec::new(),
            fails: true,
        }
    }
}

impl TrustQuery for Recorded {
    fn trust(&self, subject: &TrustSubject<'_>) -> Result<Trust, TrustQueryError> {
        if self.fails {
            return Err(TrustQueryError::StoreUnreadable);
        }
        let id = subject.locked.id.as_str();
        Ok(match self.records.iter().find(|(known, ..)| known == id) {
            None => Trust::Untrusted,
            Some((_, root, tree)) if root == subject.root && tree == subject.tree => Trust::Trusted,
            Some(_) => Trust::Changed,
        })
    }
}

/// [`Recorded`] as a store the shared trust cases can fill.
struct RecordedStore;

impl trust_conformance::TrustStore for RecordedStore {
    fn trusting(&self, trusted: &[TrustSubject<'_>]) -> Box<dyn TrustQuery> {
        Box::new(Recorded {
            records: trusted
                .iter()
                .map(|s| (s.locked.id.to_string(), s.root.to_owned(), s.tree.clone()))
                .collect(),
            fails: false,
        })
    }

    fn failing(&self) -> Box<dyn TrustQuery> {
        Box::new(Recorded::failing())
    }
}

#[rstest]
#[case::trusted(trust_conformance::trusted)]
#[case::untrusted(trust_conformance::untrusted)]
#[case::changed(trust_conformance::changed)]
#[case::another_root(trust_conformance::another_root)]
#[case::store_error(trust_conformance::store_error)]
fn the_recorded_double_answers_the_shared_trust_cases(
    #[case] case: fn(&dyn trust_conformance::TrustStore),
) {
    case(&RecordedStore);
}

/// No trust recorded for any plugin. A load of data plugins asks for none.
const NO_TRUST: &Recorded = &Recorded::none();

/// One pack to install: its manifest values and extra files.
struct Spec {
    id: &'static str,
    version: &'static str,
    /// The `slopvac_api` in the installed manifest.
    api: u32,
    /// The `slopvac_api` in the lock entry.
    locked_api: u32,
    /// `(id, version requirement)` in the manifest.
    requires: Vec<(&'static str, &'static str)>,
    /// Ids in the lock entry's `requires`.
    locked_requires: Vec<&'static str>,
    provides: String,
    files: Vec<(&'static str, String)>,
    /// Files whose bytes need not be UTF-8.
    raw_files: Vec<(&'static str, Vec<u8>)>,
    /// The `kind` in the installed manifest.
    kind: &'static str,
    /// The `kind` in the lock entry.
    locked_kind: &'static str,
}

impl Spec {
    fn new(id: &'static str) -> Self {
        Self {
            id,
            version: "1.0.0",
            api: 1,
            locked_api: 1,
            requires: Vec::new(),
            locked_requires: Vec::new(),
            provides: String::new(),
            files: Vec::new(),
            raw_files: Vec::new(),
            kind: "data",
            locked_kind: "data",
        }
    }

    fn requires(mut self, id: &'static str) -> Self {
        self.requires.push((id, ">=1"));
        self.locked_requires.push(id);
        self
    }

    fn genre(mut self, name: &'static str, body: &str) -> Self {
        self.provides = "genres = [\"genres/g.yaml\"]".into();
        self.files.push((
            "genres/g.yaml",
            format!("schema_version: 3\nid: {name}\n{body}"),
        ));
        self
    }

    /// A category file with one pattern rule for each pattern.
    fn rules(mut self, patterns: &[&str]) -> Self {
        let rules: Vec<String> = patterns
            .iter()
            .enumerate()
            .map(|(i, pattern)| {
                format!(
                    r#"{{"id":"r{i}","name":"N","message":"m","match":{{"kind":"pattern","pattern":"{pattern}"}},"citation":{{"source":"s"}}}}"#
                )
            })
            .collect();
        self.provides = "categories = [\"categories/c.json\"]".into();
        self.files.push((
            "categories/c.json",
            format!(
                r#"{{"schema_version":3,"id":"c","title":"T","description":"D","rules":[{}]}}"#,
                rules.join(",")
            ),
        ));
        self
    }

    fn manifest(&self) -> String {
        let mut text = format!(
            "schema_version = 3\nid = \"{}\"\nversion = \"{}\"\nslopvac_api = {}\nkind = \"{}\"\n\n[provides]\n{}\n",
            self.id, self.version, self.api, self.kind, self.provides
        );
        if self.kind == "code" {
            text.push_str(
                "\n[plugin]\ncommand = \"bin/acme\"\nnetwork = false\nhooks = []\nfeatures = { required = [], optional = [] }\n",
            );
        }
        let requires = self
            .requires
            .iter()
            .map(|(id, req)| format!("\n[[requires]]\nid = \"{id}\"\nversion = \"{req}\"\n"));
        text.extend(requires);
        text
    }
}

struct World {
    tmp: tempfile::TempDir,
    entries: Vec<String>,
    count: u32,
}

impl World {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        for dir in ["store/trees", "store/refs", "stage"] {
            fs::create_dir_all(tmp.path().join(dir)).unwrap();
        }
        Self {
            tmp,
            entries: Vec::new(),
            count: 0,
        }
    }

    fn store(&self) -> Store {
        Store::new(self.tmp.path().join("store"))
    }

    /// Install a pack the way sync leaves it, and add its lock entry. Returns the tree.
    fn install(&mut self, spec: &Spec) -> PathBuf {
        let tree = self.write_tree(spec);
        // The lock pins the digest of the archive that sync cached, and the loader derives the
        // tree hash from that archive. The store's record is only a hint.
        let archive = targz_of_dir(&tree);
        let digest = slopvac_plugins::oci::Digest::of(&archive);
        let blobs = self.tmp.path().join("store/blobs");
        fs::create_dir_all(&blobs).unwrap();
        fs::write(blobs.join(digest.hex()), &archive).unwrap();
        self.count += 1;
        self.lock_entry_with(spec, digest.hex());
        let hash = tree_hash(&tree).unwrap();
        let dest = self
            .tmp
            .path()
            .join("store/trees")
            .join(format!("tree-{}", hash.hex()));
        fs::rename(&tree, &dest).unwrap();
        fs::write(
            self.tmp.path().join("store/refs").join(digest.hex()),
            hash.hex(),
        )
        .unwrap();
        dest
    }

    fn write_tree(&self, spec: &Spec) -> PathBuf {
        let tree = self.tmp.path().join("stage").join(spec.id);
        fs::create_dir_all(&tree).unwrap();
        fs::write(tree.join("slopvac-plugin.toml"), spec.manifest()).unwrap();
        let raw = spec.raw_files.iter().map(|(n, b)| (*n, b.as_slice()));
        let text = spec.files.iter().map(|(n, t)| (*n, t.as_bytes()));
        for (name, bytes) in text.chain(raw) {
            let target = tree.join(name);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, bytes).unwrap();
        }
        tree
    }

    fn digest_hex(&self) -> String {
        format!("{:064x}", self.count)
    }

    /// A lock entry for `spec` with a digest of its own, whether or not the store holds it.
    fn lock_entry(&mut self, spec: &Spec) {
        self.count += 1;
        let digest = self.digest_hex();
        self.lock_entry_with(spec, &digest);
    }

    fn lock_entry_with(&mut self, spec: &Spec, digest_hex: &str) {
        let requires = spec
            .locked_requires
            .iter()
            .map(|r| format!("\"{r}\""))
            .collect::<Vec<_>>()
            .join(", ");
        self.entries.push(format!(
            "\n[[plugins]]\nid = \"{}\"\nkind = \"{}\"\nversion = \"{}\"\nslopvac_api = {}\nsource = \"dist/{}.tar.gz\"\ndigest = \"sha256:{}\"\nrequires = [{requires}]\n",
            spec.id,
            spec.locked_kind,
            spec.version,
            spec.locked_api,
            spec.id,
            digest_hex,
        ));
    }

    fn lock(&self) -> Lockfile {
        Lockfile::parse(&format!("schema_version = 3\n{}", self.entries.concat())).unwrap()
    }

    fn load(&self, requested: &[&str]) -> Result<Vec<LoadedPack>, SyncedError> {
        self.load_trusting(requested, NO_TRUST)
    }

    fn load_trusting(
        &self,
        requested: &[&str],
        trust: &dyn TrustQuery,
    ) -> Result<Vec<LoadedPack>, SyncedError> {
        load_synced(&ids(requested), &self.lock(), &self.store(), trust)
    }
}

/// A `.tar.gz` of the files under `root`, in path order.
fn targz_of_dir(root: &Path) -> Vec<u8> {
    use std::io::Write as _;
    fn walk(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                out.push(path.strip_prefix(base).unwrap().to_path_buf());
            }
        }
    }
    let mut files = Vec::new();
    walk(root, root, &mut files);
    files.sort();
    let mut tar = tar::Builder::new(Vec::new());
    for rel in files {
        let bytes = fs::read(root.join(&rel)).unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        tar.append_data(&mut header, &rel, bytes.as_slice())
            .unwrap();
    }
    // Stored, not compressed: the fixtures hold huge repeated files that the ratio limit refuses.
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::none());
    gz.write_all(&tar.into_inner().unwrap()).unwrap();
    gz.finish().unwrap()
}

fn ids(requested: &[&str]) -> Vec<PackId> {
    requested.iter().map(|r| PackId::new(*r).unwrap()).collect()
}

fn loaded_ids(packs: &[LoadedPack]) -> Vec<String> {
    packs
        .iter()
        .map(|p| p.manifest().id.as_str().to_owned())
        .collect()
}

/// Make a tree writable again and change a file, as an attacker with the user's rights could.
fn tamper(tree: &Path) {
    let file = tree.join("slopvac-plugin.toml");
    let mut text = fs::read_to_string(&file).unwrap();
    text.push_str("# changed\n");
    fs::write(file, text).unwrap();
}

#[test]
fn a_listed_pack_loads_after_the_packs_it_requires() {
    let mut w = World::new();
    w.install(&Spec::new("top").requires("mid"));
    w.install(&Spec::new("mid").requires("base"));
    w.install(&Spec::new("base"));

    let packs = w.load(&["top"]).unwrap();

    assert_eq!(loaded_ids(&packs), ["base", "mid", "top"]);
}

#[test]
fn a_pack_two_packs_require_loads_once() {
    let mut w = World::new();
    w.install(&Spec::new("a").requires("shared"));
    w.install(&Spec::new("b").requires("shared"));
    w.install(&Spec::new("shared"));

    let packs = w.load(&["a", "b"]).unwrap();

    assert_eq!(loaded_ids(&packs), ["shared", "a", "b"]);
}

#[test]
fn a_data_plugin_loads_the_rule_file_its_hash_covered_not_the_one_on_disk() {
    let mut w = World::new();
    let tree = w.install(&Spec::new("acme").rules(&["alpha"]));
    // After the file is hashed, the file on disk gains two rules. A load that opened it a second
    // time would see three rules; the load keeps the one rule the hash covered.
    let swapped = Spec::new("acme").rules(&["alpha", "beta", "gamma"]);
    let replacement = swapped
        .files
        .iter()
        .find(|(name, _)| *name == "categories/c.json")
        .map(|(_, text)| text.clone())
        .unwrap();
    let mut seen = Vec::new();
    let mut swap = |rel: &str, path: &Path| {
        seen.push(rel.to_owned());
        if rel == "categories/c.json" {
            fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
            fs::write(path, &replacement).unwrap();
        }
    };

    let packs = slopvac_packs::load_synced_observed(
        &ids(&["acme"]),
        &w.lock(),
        &w.store(),
        NO_TRUST,
        &mut Budget::default(),
        &mut swap,
    )
    .unwrap();

    assert!(seen.iter().any(|r| r == "categories/c.json"), "{seen:?}");
    assert_eq!(
        fs::read_to_string(tree.join("categories/c.json")).unwrap(),
        replacement
    );
    assert_eq!(packs[0].rules().len(), 1);
}

#[test]
fn a_tampered_pack_tree_fails_the_load_with_a_typed_error() {
    let mut w = World::new();
    let tree = w.install(&Spec::new("acme"));
    tamper(&tree);
    let before = tree_contents(&tree);

    let err = w.load(&["acme"]).unwrap_err();

    assert!(matches!(err, SyncedError::Tampered { .. }), "{err}");
    assert_eq!(err.code(), Some("plugin-tampered"));
    // A load removes nothing: every entry of the tree is still there, byte for byte.
    assert_eq!(tree_contents(&tree), before, "a load changed the tree");
}

/// One entry of a tree, read without following links.
#[derive(Debug, PartialEq, Eq)]
enum Entry {
    File(Vec<u8>),
    Dir,
    Link(PathBuf),
}

/// Every entry under `dir` by relative path. A link is recorded with its target and never
/// followed, so swapping a file for a link to the same bytes shows.
fn tree_contents(dir: &Path) -> std::collections::BTreeMap<PathBuf, Entry> {
    let mut out = std::collections::BTreeMap::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).unwrap() {
            let path = entry.unwrap().path();
            let rel = path.strip_prefix(dir).unwrap().to_path_buf();
            let kind = fs::symlink_metadata(&path).unwrap().file_type();
            if kind.is_symlink() {
                out.insert(rel, Entry::Link(fs::read_link(&path).unwrap()));
            } else if kind.is_dir() {
                out.insert(rel, Entry::Dir);
                pending.push(path);
            } else {
                out.insert(rel, Entry::File(fs::read(&path).unwrap()));
            }
        }
    }
    out
}

/// The one cached archive that `install` left in the store.
fn only_blob(w: &World) -> PathBuf {
    let mut blobs = fs::read_dir(w.tmp.path().join("store/blobs")).unwrap();
    let blob = blobs.next().unwrap().unwrap().path();
    assert!(blobs.next().is_none(), "more than one cached archive");
    blob
}

#[test]
fn a_link_planted_in_an_installed_tree_is_a_store_failure_and_not_tampering() {
    let mut w = World::new();
    let tree = w.install(&Spec::new("acme"));
    std::os::unix::fs::symlink("slopvac-plugin.toml", tree.join("link")).unwrap();

    let err = w.load(&["acme"]).unwrap_err();

    assert!(matches!(err, SyncedError::StoreUnreadable { .. }), "{err}");
    assert_eq!(err.code(), Some("plugin-store-unreadable"));
}

/// A tree over the cap is refused before any file is parsed, and the message names the tree.
#[test]
fn a_data_plugin_tree_over_32_mib_says_the_installed_tree_is_too_large() {
    let mut w = World::new();
    let tree = w.install(&Spec::new("acme"));
    fs::write(tree.join("big.bin"), vec![0_u8; (32 << 20) + 1]).unwrap();

    let err = w.load(&["acme"]).unwrap_err();

    assert_eq!(err.code(), Some("plugin-too-large"));
    assert_eq!(
        err.to_string(),
        "the installed tree of plugin `acme` is over the 33554432 byte limit for a load"
    );
}

#[test]
fn a_swapped_cached_archive_is_tampering_and_a_load_leaves_it_in_place() {
    let mut w = World::new();
    w.install(&Spec::new("acme"));
    let blob = only_blob(&w);
    fs::write(&blob, b"swapped").unwrap();

    let err = w.load(&["acme"]).unwrap_err();

    assert!(matches!(err, SyncedError::Tampered { .. }), "{err}");
    assert_eq!(err.code(), Some("plugin-tampered"));
    assert_eq!(
        fs::read(&blob).unwrap(),
        b"swapped",
        "a load deleted the blob"
    );
}

#[test]
fn an_unreadable_cached_archive_is_a_store_failure_and_not_a_missing_plugin() {
    let mut w = World::new();
    w.install(&Spec::new("acme"));
    let blob = only_blob(&w);
    fs::set_permissions(&blob, std::os::unix::fs::PermissionsExt::from_mode(0o000)).unwrap();
    if fs::File::open(&blob).is_ok() {
        return; // the test runs with rights that ignore modes
    }

    let err = w.load(&["acme"]).unwrap_err();

    assert!(matches!(err, SyncedError::StoreUnreadable { .. }), "{err}");
    assert_eq!(err.code(), Some("plugin-store-unreadable"));
}

#[test]
fn a_tampered_dependency_stops_the_load_of_the_pack_that_requires_it() {
    let mut w = World::new();
    w.install(&Spec::new("top").requires("base"));
    let tree = w.install(&Spec::new("base"));
    tamper(&tree);

    let err = w.load(&["top"]).unwrap_err();

    assert!(
        matches!(err, SyncedError::Tampered { ref id } if id == "base"),
        "{err}"
    );
}

#[rstest]
#[case::listed_pack(&["ghost"])]
#[case::required_pack(&["top"])]
fn a_pack_the_lock_does_not_pin_is_refused(#[case] request: &[&str]) {
    let mut w = World::new();
    w.install(&Spec::new("top").requires("ghost"));

    let err = w.load(request).unwrap_err();

    assert!(
        matches!(err, SyncedError::NotLocked { ref id } if id == "ghost"),
        "{err}"
    );
    assert_eq!(err.code(), Some("plugin-not-locked"));
}

#[test]
fn a_pack_in_the_store_but_not_in_the_lock_is_not_loaded() {
    let mut w = World::new();
    w.install(&Spec::new("acme"));
    let lock = Lockfile::parse("schema_version = 3\nplugins = []\n").unwrap();

    let err = load_synced(&ids(&["acme"]), &lock, &w.store(), NO_TRUST).unwrap_err();

    assert!(matches!(err, SyncedError::NotLocked { .. }), "{err}");
}

#[test]
fn a_locked_pack_with_no_stored_tree_is_not_installed() {
    let mut w = World::new();
    w.lock_entry(&Spec::new("acme"));

    let err = w.load(&["acme"]).unwrap_err();

    assert!(matches!(err, SyncedError::NotInstalled { .. }), "{err}");
    assert_eq!(err.code(), Some("plugin-not-installed"));
}

#[test]
fn a_requires_cycle_is_an_error_that_names_the_cycle() {
    let mut w = World::new();
    w.install(&Spec::new("a").requires("b"));
    w.install(&Spec::new("b").requires("c"));
    w.install(&Spec::new("c").requires("a"));

    let err = w.load(&["a"]).unwrap_err();

    assert!(
        matches!(&err, SyncedError::RequiresCycle { cycle } if cycle == &["a", "b", "c", "a"]),
        "{err}"
    );
    assert!(err.to_string().ends_with("a -> b -> c -> a"), "{err}");
    assert_eq!(err.code(), Some("plugin-requires-cycle"));
}

#[test]
fn a_pack_that_requires_itself_is_a_cycle() {
    let mut w = World::new();
    w.install(&Spec::new("a").requires("a"));

    let err = w.load(&["a"]).unwrap_err();

    assert!(
        matches!(&err, SyncedError::RequiresCycle { cycle } if cycle == &["a", "a"]),
        "{err}"
    );
}

#[rstest]
#[case::locked_entry(2, 1)]
#[case::installed_manifest(1, 2)]
fn a_pack_for_a_newer_api_is_refused(#[case] locked_api: u32, #[case] manifest_api: u32) {
    let mut w = World::new();
    let spec = Spec {
        locked_api,
        api: manifest_api,
        ..Spec::new("acme")
    };
    w.install(&spec);

    let err = w.load(&["acme"]).unwrap_err();

    assert!(matches!(err, SyncedError::ApiUnsupported { .. }), "{err}");
    assert_eq!(err.code(), Some("plugin-api-unsupported"));
}

#[test]
fn a_manifest_that_differs_from_its_lock_entry_is_refused() {
    // The lock pins 1.0.0; the tree behind its digest says 1.1.0.
    let mut w = World::new();
    w.install(&Spec {
        version: "1.1.0",
        ..Spec::new("acme")
    });
    w.entries = w
        .entries
        .iter()
        .map(|e| e.replace("version = \"1.1.0\"", "version = \"1.0.0\""))
        .collect();

    let err = w.load(&["acme"]).unwrap_err();

    assert!(
        matches!(
            err,
            SyncedError::ManifestMismatch {
                field: "version",
                ..
            }
        ),
        "{err}"
    );
}

#[rstest]
#[case::data_locked_code_installed("data", "code")]
#[case::code_locked_data_installed("code", "data")]
fn a_manifest_kind_that_differs_from_its_lock_entry_is_refused(
    #[case] locked_kind: &'static str,
    #[case] kind: &'static str,
) {
    let mut w = World::new();
    w.install(&Spec {
        kind,
        locked_kind,
        ..Spec::new("acme")
    });

    let err = w.load(&["acme"]).unwrap_err();

    assert!(
        matches!(
            err,
            SyncedError::ManifestMismatch {
                ref id,
                field: "kind"
            } if id == "acme"
        ),
        "{err}"
    );
    assert_eq!(err.code(), Some("plugin-manifest-mismatch"));
}

#[rstest]
#[case::data_locked_code_installed("data", "code")]
#[case::code_locked_data_installed("code", "data")]
fn a_manifest_kind_that_differs_from_its_lock_entry_is_refused_before_any_file_compiles(
    #[case] locked_kind: &'static str,
    #[case] kind: &'static str,
) {
    // The pattern does not compile. A load that compiled the files first would report that.
    let mut w = World::new();
    w.install(&Spec {
        kind,
        locked_kind,
        ..Spec::new("acme").rules(&["("])
    });

    let err = w.load(&["acme"]).unwrap_err();

    assert!(
        matches!(err, SyncedError::ManifestMismatch { field: "kind", .. }),
        "{err}"
    );
}

/// The bytes a load of `spec` reads: its manifest and each file it ships.
fn bytes_of(spec: &Spec) -> u64 {
    let files: usize = spec.files.iter().map(|(_, text)| text.len()).sum();
    (spec.manifest().len() + files) as u64
}

#[rstest]
#[case::rules(|_: &Spec, l: &mut Limits| l.run_rules = 1)]
#[case::bytes(|next: &Spec, l: &mut Limits| l.run_bytes = bytes_of(next))]
fn a_manifest_kind_that_differs_from_its_lock_entry_charges_nothing_to_the_run_budget(
    #[case] fit: fn(&Spec, &mut Limits),
) {
    let mut w = World::new();
    w.install(&Spec {
        kind: "code",
        locked_kind: "data",
        ..Spec::new("acme").rules(&["a"])
    });
    let next = Spec::new("next").rules(&["b"]);
    w.install(&next);
    // The run budget holds `next` alone, so any charge for `acme` leaves too little for it.
    let mut budget = Budget::new(small(|l| fit(&next, l)));

    let refused = load_synced_within(
        &ids(&["acme"]),
        &w.lock(),
        &w.store(),
        NO_TRUST,
        &mut budget,
    );
    let loaded = load_synced_within(
        &ids(&["next"]),
        &w.lock(),
        &w.store(),
        NO_TRUST,
        &mut budget,
    );

    assert!(
        matches!(
            refused,
            Err(SyncedError::ManifestMismatch { field: "kind", .. })
        ),
        "{refused:?}"
    );
    assert_eq!(loaded_ids(&loaded.unwrap()), ["next"]);
}

/// A code plugin `acme` with one rule for each pattern.
fn code_plugin(patterns: &[&str]) -> Spec {
    Spec {
        kind: "code",
        locked_kind: "code",
        ..Spec::new("acme").rules(patterns)
    }
}

/// A hash that no installed tree has.
fn other_hash() -> TreeHash {
    TreeHash::parse(&format!("sha256:{}", "0".repeat(64))).unwrap()
}

/// What the trust store records for `acme`, installed at `root`, in each refusal case.
fn untrusted_record(trust: Trust, root: &Path) -> Recorded {
    match trust {
        Trust::Untrusted => Recorded::none(),
        Trust::Changed => Recorded::trusting("acme", root, other_hash()),
        other => panic!("{other:?} is not a refusal"),
    }
}

fn refused_for_trust(err: &SyncedError, trust: Trust) -> bool {
    match (err, trust) {
        (SyncedError::Untrusted { id }, Trust::Untrusted)
        | (SyncedError::ChangedAfterTrust { id }, Trust::Changed) => id == "acme",
        _ => false,
    }
}

#[rstest]
#[case::untrusted(Trust::Untrusted, "plugin-untrusted")]
#[case::changed_after_trust(Trust::Changed, "plugin-hash-mismatch")]
fn a_code_plugin_without_trust_for_its_tree_contributes_no_rule(
    #[case] trust: Trust,
    #[case] code: &str,
) {
    let mut w = World::new();
    let root = w.install(&code_plugin(&["a"]));

    let err = Packs::with_synced(
        &ids(&["acme"]),
        &w.lock(),
        &w.store(),
        &untrusted_record(trust, &root),
        &[],
    )
    .unwrap_err();

    assert!(refused_for_trust(&err, trust), "{err:?}");
    assert_eq!(err.code(), Some(code));
}

#[rstest]
#[case::untrusted(Trust::Untrusted)]
#[case::changed_after_trust(Trust::Changed)]
fn a_code_plugin_without_trust_is_refused_before_any_file_compiles(#[case] trust: Trust) {
    // The pattern does not compile. A load that compiled the files first would report that.
    let mut w = World::new();
    let root = w.install(&code_plugin(&["("]));

    let err = w
        .load_trusting(&["acme"], &untrusted_record(trust, &root))
        .unwrap_err();

    assert!(refused_for_trust(&err, trust), "{err:?}");
}

#[rstest]
#[case::untrusted_rules(Trust::Untrusted, |_: &Spec, l: &mut Limits| l.run_rules = 1)]
#[case::untrusted_bytes(Trust::Untrusted, |next: &Spec, l: &mut Limits| l.run_bytes = bytes_of(next))]
#[case::changed_rules(Trust::Changed, |_: &Spec, l: &mut Limits| l.run_rules = 1)]
#[case::changed_bytes(Trust::Changed, |next: &Spec, l: &mut Limits| l.run_bytes = bytes_of(next))]
fn a_code_plugin_without_trust_charges_nothing_to_the_run_budget(
    #[case] trust: Trust,
    #[case] fit: fn(&Spec, &mut Limits),
) {
    let mut w = World::new();
    let root = w.install(&code_plugin(&["a"]));
    let next = Spec::new("next").rules(&["b"]);
    w.install(&next);
    let record = untrusted_record(trust, &root);
    // The run budget holds `next` alone, so any charge for `acme` leaves too little for it.
    let mut budget = Budget::new(small(|l| fit(&next, l)));

    let refused = load_synced_within(&ids(&["acme"]), &w.lock(), &w.store(), &record, &mut budget);
    let loaded = load_synced_within(&ids(&["next"]), &w.lock(), &w.store(), &record, &mut budget);

    let err = refused.unwrap_err();
    assert!(refused_for_trust(&err, trust), "{err:?}");
    assert_eq!(loaded_ids(&loaded.unwrap()), ["next"]);
}

#[test]
fn a_code_plugin_trusted_at_its_tree_hash_loads_its_rules() {
    let mut w = World::new();
    let tree = w.install(&code_plugin(&["a", "b"]));
    let record = Recorded::trusting("acme", &tree, tree_hash(&tree).unwrap());

    let sets = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), &record, &[])
        .unwrap()
        .into_rule_sets();

    let got: Vec<(String, usize)> = sets[1..]
        .iter()
        .map(|s| (s.pack().to_string(), s.rules().count()))
        .collect();
    assert_eq!(got, [("acme".to_owned(), 2)]);
}

#[test]
fn trust_for_one_installation_does_not_cover_the_same_content_at_another_root() {
    let mut first = World::new();
    let trusted_root = first.install(&code_plugin(&["a"]));
    let mut second = World::new();
    let other_root = second.install(&code_plugin(&["a"]));
    assert_ne!(trusted_root, other_root);
    let record = Recorded::trusting("acme", &trusted_root, tree_hash(&trusted_root).unwrap());

    let err = second.load_trusting(&["acme"], &record).unwrap_err();

    assert!(refused_for_trust(&err, Trust::Untrusted), "{err:?}");
    assert_eq!(err.code(), Some("plugin-untrusted"));
}

/// A trust query that trusts every installation and keeps the manifest bytes of each subject.
#[derive(Default)]
struct SeenManifests(std::cell::RefCell<Vec<Vec<u8>>>);

impl TrustQuery for SeenManifests {
    fn trust(&self, subject: &TrustSubject<'_>) -> Result<Trust, TrustQueryError> {
        self.0.borrow_mut().push(subject.manifest_bytes.to_vec());
        Ok(Trust::Trusted)
    }
}

#[test]
fn the_trust_query_sees_the_manifest_bytes_the_hash_covered_not_the_ones_on_disk() {
    let mut w = World::new();
    let spec = code_plugin(&["a"]);
    let captured = spec.manifest();
    w.install(&spec);
    // After the manifest is hashed, the file on disk changes. A load that opened it a second
    // time would hand the query the changed bytes.
    let mut swap = |rel: &str, path: &Path| {
        if rel == "slopvac-plugin.toml" {
            fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
            fs::write(path, format!("{captured}# changed\n")).unwrap();
        }
    };
    let seen = SeenManifests::default();

    slopvac_packs::load_synced_observed(
        &ids(&["acme"]),
        &w.lock(),
        &w.store(),
        &seen,
        &mut Budget::default(),
        &mut swap,
    )
    .unwrap();

    assert_eq!(seen.0.into_inner(), [captured.into_bytes()]);
}

fn refused_for_store_failure(err: &SyncedError) -> bool {
    matches!(
        err,
        SyncedError::TrustUnchecked {
            id,
            source: TrustQueryError::StoreUnreadable,
        } if id == "acme"
    )
}

#[test]
fn a_trust_store_that_cannot_answer_stops_the_load_before_any_file_compiles() {
    // The pattern does not compile. A load that compiled the files first would report that.
    let mut w = World::new();
    w.install(&code_plugin(&["("]));

    let err = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), &Recorded::failing(), &[])
        .unwrap_err();

    assert!(refused_for_store_failure(&err), "{err:?}");
    assert_eq!(err.code(), Some("plugins-store-failed"));
}

#[rstest]
#[case::rules(|_: &Spec, l: &mut Limits| l.run_rules = 1)]
#[case::bytes(|next: &Spec, l: &mut Limits| l.run_bytes = bytes_of(next))]
fn a_trust_store_that_cannot_answer_charges_nothing_to_the_run_budget(
    #[case] fit: fn(&Spec, &mut Limits),
) {
    let mut w = World::new();
    w.install(&code_plugin(&["a"]));
    let next = Spec::new("next").rules(&["b"]);
    w.install(&next);
    let failing = Recorded::failing();
    // The run budget holds `next` alone, so any charge for `acme` leaves too little for it.
    let mut budget = Budget::new(small(|l| fit(&next, l)));

    let refused = load_synced_within(
        &ids(&["acme"]),
        &w.lock(),
        &w.store(),
        &failing,
        &mut budget,
    );
    let loaded = load_synced_within(
        &ids(&["next"]),
        &w.lock(),
        &w.store(),
        &failing,
        &mut budget,
    );

    let err = refused.unwrap_err();
    assert!(refused_for_store_failure(&err), "{err:?}");
    assert_eq!(loaded_ids(&loaded.unwrap()), ["next"]);
}

/// A code plugin `acme` whose category file is not JSON.
fn code_plugin_with_unparseable_rules() -> Spec {
    let mut spec = code_plugin(&[]);
    spec.files = vec![("categories/c.json", "{ not json".to_owned())];
    spec
}

#[test]
fn an_unparseable_rule_file_in_a_trusted_code_plugin_is_a_file_error() {
    let mut w = World::new();
    let root = w.install(&code_plugin_with_unparseable_rules());
    let record = Recorded::trusting("acme", &root, tree_hash(&root).unwrap());

    let err = w.load_trusting(&["acme"], &record).unwrap_err();

    match err {
        SyncedError::Pack(PackError::Rules { problems }) => {
            assert_eq!(problems.len(), 1, "{problems:?}");
            assert_eq!(problems[0].file, "categories/c.json");
            assert!(matches!(problems[0].fault, RuleFault::Parse), "{problems:?}");
        }
        other => panic!("{other:?}"),
    }
}

#[rstest]
#[case::untrusted(Trust::Untrusted)]
#[case::changed_after_trust(Trust::Changed)]
fn a_code_plugin_without_trust_is_refused_before_its_rule_file_parses(#[case] trust: Trust) {
    let mut w = World::new();
    let root = w.install(&code_plugin_with_unparseable_rules());

    let err = w
        .load_trusting(&["acme"], &untrusted_record(trust, &root))
        .unwrap_err();

    assert!(refused_for_trust(&err, trust), "{err:?}");
}

#[test]
fn a_data_plugin_loads_with_no_trust_recorded() {
    let mut w = World::new();
    w.install(&Spec::new("acme").rules(&["a"]));

    let sets = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST, &[])
        .unwrap()
        .into_rule_sets();

    let got: Vec<(String, usize)> = sets[1..]
        .iter()
        .map(|s| (s.pack().to_string(), s.rules().count()))
        .collect();
    assert_eq!(got, [("acme".to_owned(), 1)]);
}

#[test]
fn a_required_version_the_lock_does_not_provide_is_refused() {
    let mut w = World::new();
    let spec = Spec {
        requires: vec![("base", ">=2")],
        locked_requires: vec!["base"],
        ..Spec::new("top")
    };
    w.install(&spec);
    w.install(&Spec::new("base"));

    let err = w.load(&["top"]).unwrap_err();

    assert!(matches!(err, SyncedError::RequirementUnmet { .. }), "{err}");
    assert_eq!(err.code(), Some("plugin-requirement-unmet"));
}

#[test]
fn a_synced_plugin_registers_its_taggers_after_the_core_ones() {
    let mut w = World::new();
    let mut spec = Spec::new("acme");
    spec.provides = "taggers = [\"taggers/t.yaml\"]".into();
    spec.files.push((
        "taggers/t.yaml",
        "schema_version: 3\nid: t\ntag: acme-tag\nmatch:\n  - pattern: 'a'\n".to_owned(),
    ));
    w.install(&spec);

    let packs = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST, &[]).unwrap();

    let ids: Vec<&str> = packs.taggers().map(|t| t.id.as_str()).collect();
    assert_eq!(ids.len(), Packs::core().unwrap().taggers().count() + 1);
    assert_eq!(ids.last(), Some(&"t"));
}

// Policies section 1.9: an authored tagger pattern over 8,192 bytes is refused with the code
// `tagger-pattern-too-long`, whichever pack supplies it. The text names the length and the cap.
#[test]
fn a_synced_tagger_over_the_pattern_cap_is_refused() {
    let mut w = World::new();
    let mut spec = Spec::new("acme");
    spec.provides = "taggers = [\"taggers/t.yaml\"]".into();
    spec.files.push((
        "taggers/t.yaml",
        format!(
            "schema_version: 3\nid: t\ntag: acme-tag\nmatch:\n  - pattern: '{}'\n",
            "a".repeat(8_193)
        ),
    ));
    w.install(&spec);

    let err = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST, &[]).unwrap_err();

    assert_eq!(err.code(), Some("tagger-pattern-too-long"), "{err}");
    assert!(
        err.to_string()
            .ends_with("pattern is 8193 bytes, over the 8192-byte limit"),
        "{err}"
    );
}

#[test]
fn a_bare_genre_name_that_a_synced_pack_shares_with_the_core_is_ambiguous() {
    let mut w = World::new();
    w.install(&Spec::new("acme").genre("consumer", "profile: strict\n"));
    let packs = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST, &[]).unwrap();
    let at = Reference::new("docs/config.toml", 4, 9);

    let err = packs.resolve_genre("consumer", &at).unwrap_err();

    let shown = err.to_string();
    assert_eq!(err.code(), Some("pack-name-ambiguous"));
    assert!(
        shown.starts_with("pack-name-ambiguous at docs/config.toml:4:9: "),
        "{shown}"
    );
    assert!(
        shown.contains("`acme:consumer`") && shown.contains("`core:consumer`"),
        "{shown}"
    );
}

#[test]
fn a_qualified_genre_name_resolves_to_the_pack_that_supplies_it() {
    let mut w = World::new();
    w.install(&Spec::new("acme").genre("consumer", "profile: strict\n"));
    let packs = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST, &[]).unwrap();
    let at = Reference::new("docs/config.toml", 4, 9);

    let (spec, from) = packs.resolve_genre("acme:consumer", &at).unwrap();

    assert_eq!((spec.id.as_str(), from.pack.as_str()), ("consumer", "acme"));
    let chain = packs.genre_chain("acme:consumer");
    assert!(
        chain
            .iter()
            .flatten()
            .any(|s| s.path == ["profile", "name"] && s.value == "strict"),
        "{chain:?}"
    );
}

#[test]
fn a_bare_genre_name_only_a_synced_pack_defines_resolves() {
    let mut w = World::new();
    w.install(&Spec::new("acme").genre("memo", ""));
    let packs = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST, &[]).unwrap();

    let (_, from) = packs
        .resolve_genre("memo", &Reference::new("c.toml", 1, 1))
        .unwrap();

    assert_eq!(from.pack.as_str(), "acme");
}

proptest! {
    /// Whatever acyclic requirements the lock holds, a pack loads after every pack it requires,
    /// and each pack loads once.
    #[test]
    fn dependencies_always_load_before_their_dependents(
        edges in proptest::collection::vec(proptest::collection::vec(any::<bool>(), 5), 5),
        request in proptest::sample::subsequence(vec![0usize, 1, 2, 3, 4], 1..=5),
        shuffle in any::<proptest::sample::Index>(),
    ) {
        const NAMES: [&str; 5] = ["p0", "p1", "p2", "p3", "p4"];
        let mut w = World::new();
        // Pack i may require pack j only when j < i, so the graph has no cycle.
        let required = |i: usize| -> Vec<&'static str> {
            (0..i).filter(|&j| edges[i][j]).map(|j| NAMES[j]).collect()
        };
        for (i, name) in NAMES.iter().enumerate() {
            let mut spec = Spec::new(name);
            for r in required(i) {
                spec = spec.requires(r);
            }
            w.install(&spec);
        }
        let mut order = request.clone();
        let len = order.len();
        order.rotate_left(shuffle.index(len));
        let wanted: Vec<&str> = order.iter().map(|&i| NAMES[i]).collect();

        let loaded = loaded_ids(&w.load(&wanted).unwrap());

        let mut sorted = loaded.clone();
        sorted.sort();
        sorted.dedup();
        prop_assert_eq!(sorted.len(), loaded.len());
        for (at, id) in loaded.iter().enumerate() {
            let i = NAMES.iter().position(|n| n == id).unwrap();
            for r in required(i) {
                let before = loaded.iter().position(|l| l == r);
                prop_assert!(before.is_some_and(|b| b < at), "{id} before {r}: {loaded:?}");
            }
        }
        for name in wanted {
            prop_assert!(loaded.iter().any(|l| l == name));
        }
    }
}

fn small(limits: impl FnOnce(&mut Limits)) -> Limits {
    let mut l = Limits::default();
    limits(&mut l);
    l
}

/// Two packs: `base` is installed, `top` requires it and is locked but never installed, so a
/// load that read past `base` would fail with `NotInstalled` instead of a budget error.
fn base_then_unread_top(base: &Spec) -> World {
    let mut w = World::new();
    w.lock_entry(&Spec::new("top").requires("base"));
    w.install(base);
    w
}

fn run_error(w: &World, limits: Limits) -> PackError {
    let mut budget = Budget::new(limits);
    match load_synced_within(&ids(&["top"]), &w.lock(), &w.store(), NO_TRUST, &mut budget) {
        Err(SyncedError::Pack(e)) => e,
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_run_byte_budget_stops_the_load_before_the_next_pack_is_read() {
    let w = base_then_unread_top(&Spec::new("base").genre("memo", ""));

    let e = run_error(&w, small(|l| l.run_bytes = 10));

    assert!(
        matches!(
            e,
            PackError::Budget {
                budget: PackBudget::RunBytes,
                limit: 10,
                ..
            }
        ),
        "{e}"
    );
    assert_eq!(e.code(), Some("rules-budget-exceeded"));
}

#[test]
fn the_pack_byte_budget_names_the_pack() {
    let w = base_then_unread_top(&Spec::new("base").genre("memo", ""));

    let e = run_error(&w, small(|l| l.pack_bytes = 10));

    assert!(
        matches!(
            &e,
            PackError::Budget {
                budget: PackBudget::Bytes,
                limit: 10,
                ..
            }
        ),
        "{e}"
    );
    assert_eq!(e.code(), Some("rules-budget-exceeded"));
}

#[test]
fn the_run_rule_budget_is_checked_before_a_pattern_compiles() {
    // The second pattern does not compile. A load that compiled first would report that.
    let w = base_then_unread_top(&Spec::new("base").rules(&["a", "("]));

    let e = run_error(&w, small(|l| l.run_rules = 1));

    assert!(
        matches!(
            e,
            PackError::Budget {
                budget: PackBudget::RunRules,
                limit: 1,
                ..
            }
        ),
        "{e}"
    );
}

#[test]
fn the_compiled_pattern_budget_stops_the_load_after_a_pack_compiles() {
    let w = base_then_unread_top(&Spec::new("base").rules(&["abcdef"]));

    let e = run_error(&w, small(|l| l.run_pattern_bytes = 5));

    assert!(
        matches!(
            e,
            PackError::Budget {
                budget: PackBudget::RunPatternBytes,
                limit: 5,
                ..
            }
        ),
        "{e}"
    );
}

#[test]
fn the_core_pack_counts_toward_the_run_budget() {
    let mut w = World::new();
    w.install(&Spec::new("acme"));

    let err = Packs::with_synced_within(
        &ids(&["acme"]),
        &w.lock(),
        &w.store(),
        NO_TRUST,
        small(|l| l.run_rules = 1),
        &[],
    )
    .unwrap_err();

    assert!(
        matches!(
            err,
            SyncedError::Pack(PackError::Budget {
                budget: PackBudget::RunRules,
                ..
            })
        ),
        "{err}"
    );
}

#[rstest]
#[case::bytes(|l: &mut Limits| l.run_bytes = 10, PackBudget::RunBytes)]
#[case::rules(|l: &mut Limits| l.run_rules = 1, PackBudget::RunRules)]
#[case::patterns(|l: &mut Limits| l.run_pattern_bytes = 5, PackBudget::RunPatternBytes)]
fn registering_packs_applies_the_same_budgets_as_loading_them(
    #[case] set: fn(&mut Limits),
    #[case] what: PackBudget,
) {
    let mut w = World::new();
    w.install(&Spec::new("acme").rules(&["abcdef", "ghi"]));
    // Loaded under the default limits, then registered under small ones.
    let packs = load_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST).unwrap();

    let err = Packs::new_within(packs, small(set), &[]).unwrap_err();

    assert!(
        matches!(err, PackError::Budget { budget, .. } if budget == what),
        "{err}"
    );
}

#[test]
fn manifests_alone_exhaust_the_run_byte_budget() {
    // Neither pack lists a file, so only the manifests are counted.
    let mut w = World::new();
    w.install(&Spec::new("top").requires("base"));
    w.install(&Spec::new("base"));
    // One manifest fits and two do not.
    let manifest_len = Spec::new("base").manifest().len() as u64;

    let e = run_error(&w, small(|l| l.run_bytes = manifest_len));

    assert!(
        matches!(
            e,
            PackError::Budget {
                budget: PackBudget::RunBytes,
                ..
            }
        ),
        "{e}"
    );
}

#[test]
fn a_manifest_counts_toward_the_pack_byte_budget() {
    let w = base_then_unread_top(&Spec::new("base"));

    let e = run_error(&w, small(|l| l.pack_bytes = 10));

    assert!(
        matches!(
            e,
            PackError::Budget {
                budget: PackBudget::Bytes,
                ..
            }
        ),
        "{e}"
    );
}

#[test]
fn a_pattern_is_charged_its_compiled_size_not_its_source_size() {
    // The source is about twenty bytes and passes a limit of 500, but ab repeated a thousand
    // times compiles to several kilobytes.
    let w = base_then_unread_top(&Spec::new("base").rules(&["(?:ab){1000}"]));

    let e = run_error(&w, small(|l| l.run_pattern_bytes = 500));

    assert!(
        matches!(
            e,
            PackError::Budget {
                budget: PackBudget::RunPatternBytes,
                limit: 500,
                ..
            }
        ),
        "{e}"
    );
}

#[test]
fn an_exhausted_pattern_budget_compiles_nothing() {
    // The pattern does not compile. A load that compiled it would report a rules problem.
    let w = base_then_unread_top(&Spec::new("base").rules(&["("]));

    let e = run_error(&w, small(|l| l.run_pattern_bytes = 0));

    assert!(
        matches!(
            e,
            PackError::Budget {
                budget: PackBudget::RunPatternBytes,
                limit: 0,
                ..
            }
        ),
        "{e}"
    );
}

#[rstest]
#[case::files(PackBudget::Files, "plugin-files")]
#[case::bytes(PackBudget::Bytes, "plugin-bytes")]
#[case::run_bytes(PackBudget::RunBytes, "plugin-run-bytes")]
#[case::run_rules(PackBudget::RunRules, "plugin-run-rules")]
#[case::run_pattern_bytes(PackBudget::RunPatternBytes, "plugin-run-pattern-bytes")]
fn every_budget_refusal_has_one_code_and_names_its_budget(
    #[case] budget: PackBudget,
    #[case] name: &str,
) {
    let error = PackError::Budget {
        budget,
        limit: 1,
        plugin: None,
    };

    assert_eq!(error.code(), Some("rules-budget-exceeded"));
    assert_eq!(budget.as_str(), name);
    assert!(error.to_string().contains(name), "{error}");
}

#[rstest]
#[case::files(PackBudget::Files, 256, "plugin-files")]
#[case::bytes(PackBudget::Bytes, 33_554_432, "plugin-bytes")]
#[case::run_bytes(PackBudget::RunBytes, 134_217_728, "plugin-run-bytes")]
#[case::run_rules(PackBudget::RunRules, 20_000, "plugin-run-rules")]
#[case::run_pattern_bytes(PackBudget::RunPatternBytes, 67_108_864, "plugin-run-pattern-bytes")]
fn a_budget_refusal_exposes_its_budget_name_and_limit(
    #[case] budget: PackBudget,
    #[case] limit: u64,
    #[case] name: &str,
) {
    let pack = PackError::Budget {
        budget,
        limit,
        plugin: None,
    };
    let synced = SyncedError::Pack(PackError::Budget {
        budget,
        limit,
        plugin: None,
    });

    for (got, code) in [
        (pack.budget(), pack.code()),
        (synced.budget(), synced.code()),
    ] {
        let (b, l) = got.unwrap();
        assert_eq!((b.as_str(), l), (name, limit));
        assert_eq!(code, Some("rules-budget-exceeded"));
    }
    assert!(PackError::NotCore.budget().is_none());
}

#[rstest]
#[case::pack_error(SyncedError::Pack(PackError::Budget { budget: PackBudget::Files, limit: 1, plugin: None }), Some("rules-budget-exceeded"))]
#[case::pack_glob(
    SyncedError::Pack(PackError::GlobTooComplex),
    Some("plugin-glob-too-complex")
)]
#[case::pack_link(SyncedError::Pack(PackError::Source(slopvac_packs::SourceError::Link { path: "l".into() })), Some("plugin-file-link"))]
#[case::pack_entries(
    SyncedError::Pack(PackError::Source(slopvac_packs::SourceError::TooManyEntries)),
    Some("plugin-too-many-entries")
)]
#[case::pack_without_code(SyncedError::Pack(PackError::NotCore), None)]
#[case::not_locked(SyncedError::NotLocked { id: "a".into() }, Some("plugin-not-locked"))]
#[case::not_installed(SyncedError::NotInstalled { id: "a".into() }, Some("plugin-not-installed"))]
#[case::tampered(SyncedError::Tampered { id: "a".into() }, Some("plugin-tampered"))]
#[case::too_large(SyncedError::TooLarge { id: "a".into(), part: TooLargePart::Tree, cap: 7 }, Some("plugin-too-large"))]
#[case::api(SyncedError::ApiUnsupported { id: "a".into(), api: 9 }, Some("plugin-api-unsupported"))]
#[case::mismatch(SyncedError::ManifestMismatch { id: "a".into(), field: "id" }, Some("plugin-manifest-mismatch"))]
#[case::cycle(SyncedError::RequiresCycle { cycle: vec!["a".into(), "a".into()] }, Some("plugin-requires-cycle"))]
#[case::unmet(SyncedError::RequirementUnmet { id: "a".into(), needs: "b".into(), requirement: ">=1".into() }, Some("plugin-requirement-unmet"))]
#[case::untrusted(SyncedError::Untrusted { id: "a".into() }, Some("plugin-untrusted"))]
#[case::changed_after_trust(SyncedError::ChangedAfterTrust { id: "a".into() }, Some("plugin-hash-mismatch"))]
#[case::trust_unchecked(SyncedError::TrustUnchecked { id: "a".into(), source: TrustQueryError::StoreUnreadable }, Some("plugins-store-failed"))]
fn every_error_reports_its_catalog_code(#[case] error: SyncedError, #[case] code: Option<&str>) {
    assert_eq!(error.code(), code);
}

/// A pack that supplies one tagger file.
fn with_tagger(id: &'static str, tagger: &str) -> Spec {
    let mut spec = Spec::new(id);
    spec.provides = "taggers = [\"taggers/t.yaml\"]".into();
    spec.files.push(("taggers/t.yaml", tagger.to_owned()));
    spec
}

fn core_tagger() -> (String, String) {
    let packs = Packs::core().unwrap();
    let t = packs.taggers().next().unwrap();
    (t.id.clone(), t.tag.clone())
}

#[test]
fn owned_rule_sets_list_the_core_first_then_each_plugin_in_load_order() {
    let mut w = World::new();
    w.install(&Spec::new("base").rules(&["a"]));
    w.install(&Spec::new("top").requires("base").rules(&["b", "c"]));

    let sets = Packs::with_synced(&ids(&["top"]), &w.lock(), &w.store(), NO_TRUST, &[])
        .unwrap()
        .into_rule_sets();

    let got: Vec<(String, usize)> = sets
        .iter()
        .map(|s| (s.pack().to_string(), s.rules().count()))
        .collect();
    assert_eq!(got[1..], [("base".to_owned(), 1), ("top".to_owned(), 2)]);
    assert_eq!(got[0].0, "core");
}

#[test]
fn a_synced_tagger_that_takes_a_core_tagger_id_is_refused() {
    let (id, _) = core_tagger();
    let mut w = World::new();
    w.install(&with_tagger(
        "acme",
        &format!("schema_version: 3\nid: {id}\ntag: acme-tag\nmatch:\n  - pattern: 'a'\n"),
    ));

    let err = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST, &[]).unwrap_err();

    assert!(err.to_string().contains("taggers/t.yaml"), "{err}");
}

#[test]
fn a_synced_tagger_that_emits_a_core_tag_is_refused() {
    let (_, tag) = core_tagger();
    let mut w = World::new();
    w.install(&with_tagger(
        "acme",
        &format!("schema_version: 3\nid: acme-t\ntag: {tag}\nmatch:\n  - pattern: 'a'\n"),
    ));

    let err = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST, &[]).unwrap_err();

    assert!(err.to_string().contains("taggers/t.yaml"), "{err}");
}

// A tagger id and an emitted tag are two namespaces. A built-in tagger id reserves only ids and
// a built-in emitted tag reserves only tags, whether or not the core is loaded: `procedure-heading`
// is the id of a core tagger that emits `procedure`, and `change-note` is emitted by the lead-in
// tagger under the id `change-note-list-lead-in`.
#[rstest]
#[case::id_of_a_built_in_tagger("requirement", "acme-tag", true)]
#[case::tag_of_a_built_in_tagger("acme-t", "requirement", true)]
#[case::tag_a_rust_tagger_emits_under_another_id("acme-t", "change-note", true)]
#[case::tag_a_declarative_tagger_emits("acme-t", "procedure", true)]
#[case::id_of_a_declarative_tagger_used_as_a_tag("acme-t", "procedure-heading", false)]
#[case::fresh_id_and_tag("acme-t", "acme-tag", false)]
#[case::tag_of_a_declarative_tagger_used_as_an_id("procedure", "acme-tag", false)]
#[case::id_of_a_declarative_tagger("procedure-heading", "acme-tag", true)]
#[case::id_of_the_declared_context_tagger("declared-contexts", "acme-tag", true)]
#[case::tag_a_built_in_genre_declares("acme-t", "security", true)]
fn a_plugin_tagger_is_checked_against_built_in_ids_and_tags_separately(
    #[case] id: &str,
    #[case] tag: &str,
    #[case] refused: bool,
) {
    let mut w = World::new();
    w.install(&with_tagger(
        "acme",
        &format!("schema_version: 3\nid: {id}\ntag: {tag}\nmatch:\n  - pattern: 'a'\n"),
    ));
    // The plugin loads on its own, so the refusal below is the assembly check.
    let loaded = w.load(&["acme"]).unwrap();

    let assembled = Packs::new(loaded, &[]);

    match (refused, assembled) {
        (false, Ok(_)) => {}
        (true, Err(PackError::Artifact { path, fault })) => {
            assert_eq!(path, "taggers/t.yaml");
            assert!(matches!(fault, ArtifactFault::Invalid), "{fault:?}");
        }
        (_, other) => panic!("refused={refused}: {:?}", other.err()),
    }
}

// The project's config can declare context tags for a genre. A plugin tagger must not emit one,
// for the same reason it must not emit a built-in genre's tag. The refusal is the typed
// `PackError::Artifact` with `ArtifactFault::Invalid`, which has no stable code.
#[rstest]
#[case::tag_the_config_declares("config-ctx", true)]
#[case::tag_the_config_does_not_declare("acme-tag", false)]
fn a_plugin_tagger_is_checked_against_the_contexts_a_config_declares(
    #[case] tag: &str,
    #[case] refused: bool,
) {
    let genres = slopvac_context::builtin_genres().unwrap();
    let genre = genres.ids()[0].to_owned();
    let genres = genres.with_contexts(&genre, &["config-ctx"]).unwrap();
    let reserved = genres.declared_context_tags().unwrap();
    let mut w = World::new();
    w.install(&with_tagger(
        "acme",
        &format!("schema_version: 3\nid: acme-t\ntag: {tag}\nmatch:\n  - pattern: 'a'\n"),
    ));
    let loaded = w.load(&["acme"]).unwrap();

    let assembled = Packs::new(loaded, &reserved);

    match (refused, assembled) {
        (false, Ok(_)) => {}
        (true, Err(PackError::Artifact { path, fault })) => {
            assert_eq!(path, "taggers/t.yaml");
            assert!(matches!(fault, ArtifactFault::Invalid), "{fault:?}");
        }
        (_, other) => panic!("refused={refused}: {:?}", other.err()),
    }
}

// The synced constructors take the reserved set too, so a caller cannot omit it: a synced plugin
// tagger that emits a context the project's config declares is refused, with the core loaded.
#[rstest]
#[case::tag_the_config_declares("config-ctx", true)]
#[case::tag_the_config_does_not_declare("acme-tag", false)]
fn a_synced_plugin_tagger_is_checked_against_the_contexts_a_config_declares(
    #[case] tag: &str,
    #[case] refused: bool,
) {
    let reserved = vec!["config-ctx".to_owned()];
    let mut w = World::new();
    w.install(&with_tagger(
        "acme",
        &format!("schema_version: 3\nid: acme-t\ntag: {tag}\nmatch:\n  - pattern: 'a'\n"),
    ));

    let synced = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST, &reserved);

    match (refused, synced) {
        (false, Ok(_)) => {}
        (true, Err(SyncedError::Pack(PackError::Artifact { path, fault }))) => {
            assert_eq!(path, "taggers/t.yaml");
            assert!(matches!(fault, ArtifactFault::Invalid), "{fault:?}");
        }
        (_, other) => panic!("refused={refused}: {:?}", other.err()),
    }
}

// A tagger pattern counts against the run's compiled-pattern budget (X3), like a rule pattern.
#[test]
fn a_tagger_pattern_is_charged_to_the_compiled_pattern_budget() {
    let w = base_then_unread_top(&with_tagger(
        "base",
        "schema_version: 3\nid: acme-t\ntag: acme-tag\nmatch:\n  - pattern: 'a'\n",
    ));

    let e = run_error(&w, small(|l| l.run_pattern_bytes = 1));

    assert!(
        matches!(
            e,
            PackError::Budget {
                budget: PackBudget::RunPatternBytes,
                limit: 1,
                ..
            }
        ),
        "{e}"
    );
}

// Policies section 1.9: the cap of a rule or category file is 2 MiB, so a 1.5 MiB file reads
// and a file over the cap is refused with `rules-file-too-large`.
#[rstest]
#[case::one_and_a_half_mebibytes(1_536 * 1024, None)]
#[case::at_the_cap(2 * 1024 * 1024, None)]
#[case::one_byte_over(2 * 1024 * 1024 + 1, Some("rules-file-too-large"))]
fn the_directory_source_caps_a_file_at_two_mebibytes(
    #[case] len: usize,
    #[case] code: Option<&str>,
) {
    use slopvac_packs::{DirSource, PackSource};
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("slopvac-plugin.toml"), "x".repeat(len)).unwrap();

    let read = DirSource::new(dir.path()).manifest_text();

    assert_eq!(
        read.as_ref()
            .err()
            .and_then(slopvac_packs::SourceError::code),
        code,
        "{read:?}"
    );
}

/// A plugin `acme` whose one style file `styles/s.toml` is `len` bytes: valid UTF-8 text, or
/// bytes that are not UTF-8.
fn plugin_with_style_of(len: usize, utf8: bool) -> World {
    let header = "schema_version = 3\ndescription = \"d\"\n";
    let mut spec = Spec::new("acme");
    spec.provides = "styles = [\"styles/s.toml\"]".into();
    if utf8 {
        // A comment line pads the header to exactly `len` bytes.
        let pad = len - header.len() - "# \n".len();
        spec.files
            .push(("styles/s.toml", format!("{header}# {}\n", "x".repeat(pad))));
    } else {
        spec.raw_files.push(("styles/s.toml", vec![0xff; len]));
    }
    let mut w = World::new();
    w.install(&spec);
    w
}

// Policies section 1.9: a style file is capped at 1 MiB, and a file of exactly the cap loads.
#[test]
fn a_plugin_style_of_exactly_one_mebibyte_loads() {
    let w = plugin_with_style_of(MIB, true);

    let loaded = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST, &[]);

    assert!(loaded.is_ok(), "{:?}", loaded.err());
}

// A file over the cap is refused with `style-too-large` where it is read, before its bytes are
// decoded as UTF-8, so a file that is not UTF-8 gets the size refusal, not a decode error. A
// rule or category file has the 2 MiB cap, so a 1.5 MiB style is over its own cap only.
#[rstest]
#[case::one_byte_over(MIB + 1, true)]
#[case::one_and_a_half_mebibytes(MIB * 3 / 2, true)]
#[case::not_utf8_one_byte_over(MIB + 1, false)]
fn a_plugin_style_over_one_mebibyte_is_refused_as_too_large(
    #[case] len: usize,
    #[case] utf8: bool,
) {
    let w = plugin_with_style_of(len, utf8);

    let err = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST, &[]).unwrap_err();

    let SyncedError::Pack(PackError::Source(SourceError::StyleTooLarge { style, cap })) = &err
    else {
        panic!("{err:?}");
    };
    assert_eq!((style.as_str(), *cap), ("s", 1_048_576));
    assert_eq!(err.code(), Some("style-too-large"));
}

#[test]
fn a_one_and_a_half_mebibyte_category_still_loads() {
    let mut spec = Spec::new("acme").rules(&["a"]);
    let (_, text) = spec.files.pop().unwrap();
    let padded = text.replacen(
        r#""rules":["#,
        &format!("{}\"rules\":[", " ".repeat(1_536 * 1024)),
        1,
    );
    spec.files.push(("categories/c.json", padded));
    let mut w = World::new();
    w.install(&spec);

    let loaded = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST, &[]);

    assert!(loaded.is_ok(), "{:?}", loaded.err());
}

// The 2 MiB cap applies before the bytes are decoded, so a category over the cap that is not
// valid UTF-8 gets the size refusal and not a decode error.
#[test]
fn an_oversized_category_that_is_not_utf8_is_refused_as_too_large() {
    let mut spec = Spec::new("acme");
    spec.provides = "categories = [\"categories/c.json\"]".into();
    spec.raw_files
        .push(("categories/c.json", vec![0xff; 2 * MIB + 1]));
    let mut w = World::new();
    w.install(&spec);

    let err = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST, &[]).unwrap_err();

    let SyncedError::Pack(PackError::Source(SourceError::TooLarge { cap })) = &err else {
        panic!("{err:?}");
    };
    assert_eq!(*cap, 2_097_152);
    assert_eq!(err.code(), Some("rules-file-too-large"));
}

// Rule ids are qualified by plugin (docs/contract/plugins.md, section 2), so two plugins that
// share a bare rule id are two rules: `base:c.r0` and `top:c.r0`.
#[test]
fn two_plugins_that_share_a_bare_rule_id_both_load() {
    let mut w = World::new();
    w.install(&Spec::new("base").rules(&["a"]));
    w.install(&Spec::new("top").requires("base").rules(&["b"]));

    let packs = Packs::with_synced(&ids(&["top"]), &w.lock(), &w.store(), NO_TRUST, &[]).unwrap();

    let ids: Vec<String> = packs
        .rule_sets()
        .flat_map(|s| s.rules().map(|r| r.id().to_string()))
        .filter(|id| id.ends_with(":c.r0"))
        .collect();
    assert_eq!(ids, ["base:c.r0", "top:c.r0"]);
}

/// A plugin `acme` whose one rule has the id of the first rule of the core pack, and that id.
fn colliding_with_core() -> (Spec, String) {
    colliding_with_core_as("acme")
}

/// As [`colliding_with_core`], for the plugin `id`.
fn colliding_with_core_as(id: &'static str) -> (Spec, String) {
    let packs = Packs::core().unwrap();
    let core_rule = packs.rule_sets().next().unwrap().rules().next().unwrap();
    colliding_with(id, core_rule.id().local())
}

/// A plugin `id` whose one rule has the local id `local` of a core rule, and that id.
fn colliding_with(id: &'static str, local: &str) -> (Spec, String) {
    let local = local.to_owned();
    let (category, rule) = local.split_once('.').unwrap();
    let text = format!(
        r#"{{"schema_version":3,"id":"{category}","title":"T","description":"D","rules":[{{"id":"{rule}","name":"N","message":"m","match":{{"kind":"pattern","pattern":"a"}},"citation":{{"source":"s"}}}}]}}"#
    );
    let mut spec = Spec::new(id);
    spec.provides = "categories = [\"categories/c.json\"]".into();
    spec.files.push(("categories/c.json", text));
    (spec, local)
}

#[test]
fn a_plugin_rule_with_a_core_rule_id_is_refused_with_its_own_code() {
    let (spec, local) = colliding_with_core();
    let mut w = World::new();
    w.install(&spec);

    let err = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST, &[]).unwrap_err();

    assert_eq!(err.code(), Some("plugin-rule-id-collision"), "{err}");
    assert_eq!(
        err.to_string(),
        format!(
            "plugin `acme` is refused: its rule `{local}` has the id of a rule that another plugin supplies, and nothing from the plugin loads"
        )
    );
    assert!(err.to_string().contains(&format!("`{local}`")), "{err}");
}

/// A plugin `acme` whose category has the id of a core category and whose one rule has a fresh
/// id, and that category id.
fn colliding_category_with_a_fresh_rule() -> (Spec, String) {
    let packs = Packs::core().unwrap();
    let core_rule = packs.rule_sets().next().unwrap().rules().next().unwrap();
    let category = core_rule.id().local().split_once('.').unwrap().0.to_owned();
    let text = format!(
        r#"{{"schema_version":3,"id":"{category}","title":"T","description":"D","rules":[{{"id":"zz-fresh-rule","name":"N","message":"m","match":{{"kind":"pattern","pattern":"a"}},"citation":{{"source":"s"}}}}]}}"#
    );
    let mut spec = Spec::new("acme");
    spec.provides = "categories = [\"categories/c.json\"]".into();
    spec.files.push(("categories/c.json", text));
    (spec, category)
}

// Categories are keyed by plugin and id, so a plugin category that shares only an id with a core
// category is legal. Both load, each with its own metadata.
#[test]
fn a_plugin_category_that_shares_an_id_with_a_core_category_loads_beside_it() {
    let (spec, category) = colliding_category_with_a_fresh_rule();
    let mut w = World::new();
    w.install(&spec);

    let packs = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST, &[]).unwrap();

    let shared: Vec<(String, String)> = packs
        .rule_sets()
        .flat_map(|s| {
            s.categories()
                .iter()
                .filter(|c| c.id().as_str() == category)
                .map(|c| (s.pack().to_string(), c.title().as_str().to_owned()))
                .collect::<Vec<_>>()
        })
        .collect();
    let plugins: Vec<&str> = shared.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(plugins, ["core", "acme"]);
    assert_eq!(shared[1].1, "T");
    assert_ne!(shared[0].1, "T");
}

// One plugin supplies a category that shares only an id with a core category, and a category
// that holds a rule with the id of a core rule. The rule clash decides the refusal, whichever
// category the plugin lists first.
#[rstest]
#[case::clash_first(true)]
#[case::clash_last(false)]
fn a_plugin_with_a_category_only_clash_and_a_rule_clash_is_refused_for_the_rule_in_either_order(
    #[case] rule_clash_first: bool,
) {
    let core = Packs::core().unwrap();
    let locals: Vec<String> = core
        .rule_sets()
        .flat_map(|s| s.rules().map(|r| r.id().local().to_owned()))
        .collect();
    let (shared_rule_category, shared_rule) = locals[0].split_once('.').unwrap();
    let (category_only, _) = locals
        .iter()
        .map(|l| l.split_once('.').unwrap())
        .find(|(c, _)| *c != shared_rule_category)
        .unwrap();
    let category = |id: &str, rule: &str| {
        format!(
            r#"{{"schema_version":3,"id":"{id}","title":"T","description":"D","rules":[{{"id":"{rule}","name":"N","message":"m","match":{{"kind":"pattern","pattern":"a"}},"citation":{{"source":"s"}}}}]}}"#
        )
    };
    let clash = (
        "categories/rule-clash.json",
        category(shared_rule_category, shared_rule),
    );
    let only = (
        "categories/category-only.json",
        category(category_only, "zz-fresh-rule"),
    );
    let (first, second) = if rule_clash_first {
        (clash, only)
    } else {
        (only, clash)
    };
    let mut spec = Spec::new("acme");
    spec.provides = format!("categories = [\"{}\", \"{}\"]", first.0, second.0);
    spec.files.push(first);
    spec.files.push(second);
    let mut w = World::new();
    w.install(&spec);

    let err = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST, &[]).unwrap_err();

    assert!(
        matches!(
            &err,
            SyncedError::Pack(PackError::RuleIdCollision { plugin, rule })
                if plugin == "acme" && *rule == locals[0]
        ),
        "{err:?}"
    );
    assert_eq!(err.code(), Some("plugin-rule-id-collision"), "{err}");
}

// A repeated plugin listed before a different plugin that clashes with the core: the core clash
// is checked first, so the refusal names the clashing plugin and its rule. Both refusals carry
// one code, so the plugin and the rule tell them apart.
#[test]
fn a_core_clash_is_reported_before_a_repeated_plugin_listed_ahead_of_it() {
    let mut w = World::new();
    w.install(&Spec::new("acme").rules(&["a"]));
    let (clash, local) = colliding_with_core_as("other");
    w.install(&clash);
    let mut loaded = load_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST).unwrap();
    loaded.extend(load_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST).unwrap());
    loaded.extend(load_synced(&ids(&["other"]), &w.lock(), &w.store(), NO_TRUST).unwrap());
    loaded.push(LoadedPack::load(&core_source(), Source::Core).unwrap());

    let err = Packs::new(loaded, &[]).unwrap_err();

    assert!(
        matches!(
            &err,
            PackError::RuleIdCollision { plugin, rule } if plugin == "other" && *rule == local
        ),
        "{err:?}"
    );
}

// Two categories of one plugin both clash with the core. The refusal names the lowest of the
// two rule ids, whichever category the plugin lists first.
#[rstest]
#[case::first_listed_is_lowest(true)]
#[case::first_listed_is_highest(false)]
fn two_core_clashes_in_one_plugin_name_the_lowest_rule_in_either_order(#[case] lowest_first: bool) {
    let core = Packs::core().unwrap();
    let locals: Vec<String> = core
        .rule_sets()
        .flat_map(|s| s.rules().map(|r| r.id().local().to_owned()))
        .collect();
    let first = locals[0].clone();
    let second = locals
        .iter()
        .find(|l| l.split_once('.').unwrap().0 != first.split_once('.').unwrap().0)
        .unwrap()
        .clone();
    let (low, high) = if first < second {
        (first, second)
    } else {
        (second, first)
    };
    let category = |local: &str| {
        let (id, rule) = local.split_once('.').unwrap();
        format!(
            r#"{{"schema_version":3,"id":"{id}","title":"T","description":"D","rules":[{{"id":"{rule}","name":"N","message":"m","match":{{"kind":"pattern","pattern":"a"}},"citation":{{"source":"s"}}}}]}}"#
        )
    };
    let (a, b) = if lowest_first {
        (&low, &high)
    } else {
        (&high, &low)
    };
    let mut spec = Spec::new("acme");
    spec.provides = "categories = [\"categories/a.json\", \"categories/b.json\"]".into();
    spec.files.push(("categories/a.json", category(a)));
    spec.files.push(("categories/b.json", category(b)));
    let mut w = World::new();
    w.install(&spec);

    let err = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST, &[]).unwrap_err();

    assert!(
        matches!(
            &err,
            SyncedError::Pack(PackError::RuleIdCollision { plugin, rule })
                if plugin == "acme" && *rule == low
        ),
        "{err:?}"
    );
}

// The refusal is a function of the set of plugins, not of the order they are listed in. Three
// plugins clash with three different core rules and `beta` is listed twice. The lowest qualified
// id, `alpha:<category>.<rule>`, decides which plugin and rule the error names.
#[rstest]
#[case::alpha_first(&[0, 1, 2, 1])]
#[case::alpha_last(&[1, 2, 1, 0])]
#[case::beta_first(&[1, 1, 2, 0])]
#[case::gamma_first(&[2, 0, 1, 1])]
fn core_clashes_across_plugins_name_the_lowest_qualified_id_in_any_order(#[case] order: &[usize]) {
    let core = Packs::core().unwrap();
    let locals: Vec<String> = core
        .rule_sets()
        .flat_map(|s| s.rules().map(|r| r.id().local().to_owned()))
        .collect();
    let mut w = World::new();
    let mut expected = String::new();
    for (name, local) in [
        ("alpha", &locals[2]),
        ("beta", &locals[0]),
        ("gamma", &locals[1]),
    ] {
        let (spec, local) = colliding_with(name, local);
        if name == "alpha" {
            expected = local;
        }
        w.install(&spec);
    }
    let mut loaded = Vec::new();
    for &i in order {
        let name = ["alpha", "beta", "gamma"][i];
        loaded.extend(load_synced(&ids(&[name]), &w.lock(), &w.store(), NO_TRUST).unwrap());
    }
    loaded.push(LoadedPack::load(&core_source(), Source::Core).unwrap());

    let err = Packs::new(loaded, &[]).unwrap_err();

    assert!(
        matches!(
            &err,
            PackError::RuleIdCollision { plugin, rule } if plugin == "alpha" && *rule == expected
        ),
        "{err:?}"
    );
}

// With no core clash, the lowest duplicated qualified id decides. `acme:c.r0`, `beta:c.r0` and
// `beta:c.r1` are each supplied twice, in every listing order.
#[rstest]
#[case::acme_first(&["acme", "beta", "acme", "beta"])]
#[case::beta_first(&["beta", "beta", "acme", "acme"])]
#[case::interleaved(&["beta", "acme", "beta", "acme"])]
#[case::acme_last(&["acme", "beta", "beta", "acme"])]
fn duplicate_ids_name_the_lowest_qualified_id_in_any_order(#[case] order: &[&'static str]) {
    let mut w = World::new();
    w.install(&Spec::new("acme").rules(&["a"]));
    w.install(&Spec::new("beta").rules(&["a", "b"]));
    let mut loaded = Vec::new();
    for name in order {
        loaded.extend(load_synced(&ids(&[name]), &w.lock(), &w.store(), NO_TRUST).unwrap());
    }

    let err = Packs::new(loaded, &[]).unwrap_err();

    assert!(
        matches!(
            &err,
            PackError::RuleIdCollision { plugin, rule } if plugin == "acme" && rule == "c.r0"
        ),
        "{err:?}"
    );
}

// The order is byte order on the whole qualified id, not a comparison of plugin, category and
// rule in turn. `acme-extra:...` sorts before `acme:...` because `-` (0x2d) is below `:` (0x3a),
// so the plugin `acme-extra` is named although `acme` is the shorter id.
#[rstest]
#[case::prefix_first(&["acme", "acme-extra", "acme", "acme-extra"])]
#[case::prefix_last(&["acme-extra", "acme", "acme-extra", "acme"])]
fn core_clashes_of_a_prefix_plugin_pair_follow_byte_order_of_the_whole_id(
    #[case] order: &[&'static str],
) {
    let core = Packs::core().unwrap();
    let locals: Vec<String> = core
        .rule_sets()
        .flat_map(|s| s.rules().map(|r| r.id().local().to_owned()))
        .collect();
    let mut w = World::new();
    let (acme, _) = colliding_with("acme", &locals[0]);
    let (extra, expected) = colliding_with("acme-extra", &locals[1]);
    w.install(&acme);
    w.install(&extra);
    let mut loaded = Vec::new();
    for name in order {
        loaded.extend(load_synced(&ids(&[name]), &w.lock(), &w.store(), NO_TRUST).unwrap());
    }
    loaded.push(LoadedPack::load(&core_source(), Source::Core).unwrap());

    let err = Packs::new(loaded, &[]).unwrap_err();

    assert!(
        matches!(
            &err,
            PackError::RuleIdCollision { plugin, rule }
                if plugin == "acme-extra" && *rule == expected
        ),
        "{err:?}"
    );
}

#[rstest]
#[case::prefix_first(&["acme", "acme-extra", "acme", "acme-extra"])]
#[case::prefix_last(&["acme-extra", "acme", "acme-extra", "acme"])]
fn duplicates_of_a_prefix_plugin_pair_follow_byte_order_of_the_whole_id(
    #[case] order: &[&'static str],
) {
    let mut w = World::new();
    w.install(&Spec::new("acme").rules(&["a"]));
    w.install(&Spec::new("acme-extra").rules(&["a"]));
    let mut loaded = Vec::new();
    for name in order {
        loaded.extend(load_synced(&ids(&[name]), &w.lock(), &w.store(), NO_TRUST).unwrap());
    }

    let err = Packs::new(loaded, &[]).unwrap_err();

    assert!(
        matches!(
            &err,
            PackError::RuleIdCollision { plugin, rule }
                if plugin == "acme-extra" && rule == "c.r0"
        ),
        "{err:?}"
    );
}

#[test]
fn the_same_qualified_rule_id_supplied_twice_is_refused() {
    let mut w = World::new();
    w.install(&Spec::new("acme").rules(&["a"]));
    let mut loaded = load_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST).unwrap();
    loaded.extend(load_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST).unwrap());

    let err = Packs::new(loaded, &[]).unwrap_err();

    assert_eq!(err.code(), Some("plugin-rule-id-collision"), "{err}");
}

// The refusal does not depend on where the core sits in the list.
#[test]
fn a_plugin_listed_before_the_core_is_still_checked_against_the_core_ids() {
    let (spec, _) = colliding_with_core();
    let mut w = World::new();
    w.install(&spec);
    let mut loaded = load_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST).unwrap();
    loaded.push(LoadedPack::load(&core_source(), Source::Core).unwrap());

    let err = Packs::new(loaded, &[]).unwrap_err();

    assert_eq!(err.code(), Some("plugin-rule-id-collision"), "{err}");
}

// X3: a tagger stops compiling at the first pattern that crosses the run budget. The first
// pattern compiles to more bytes than the whole file holds, and the second does not compile, so
// a load that went on would report the second pattern, not the budget.
#[test]
fn a_tagger_stops_compiling_at_the_first_pattern_that_crosses_the_budget() {
    let text = format!(
        "schema_version: 3\nid: t\ntag: acme-tag\nmatch:\n  - pattern: '{}'\n  - pattern: '('\n",
        "a".repeat(200)
    );
    let w = base_then_unread_top(&with_tagger("base", &text));

    let e = run_error(&w, small(|l| l.run_pattern_bytes = text.len() as u64));

    assert!(
        matches!(
            e,
            PackError::Budget {
                budget: PackBudget::RunPatternBytes,
                ..
            }
        ),
        "{e}"
    );
}

// X3: one rule may list at most 4,096 tokens (`rule-entries`). A rule with 4,097 is refused through
// the loader, and the refusal names the plugin whose load crossed it.
#[test]
fn a_rule_with_4097_tokens_is_refused_with_the_rule_entries_budget() {
    let tokens: Vec<String> = (0..4097).map(|i| format!("\"t{i}\"")).collect();
    let text = format!(
        r#"{{"schema_version":3,"id":"c","title":"T","description":"D","rules":[{{"id":"r","name":"N","message":"m","match":{{"kind":"tokens","tokens":[{}]}},"citation":{{"source":"s"}}}}]}}"#,
        tokens.join(",")
    );
    let mut spec = Spec::new("acme");
    spec.provides = "categories = [\"categories/c.json\"]".into();
    spec.files.push(("categories/c.json", text));
    let mut w = World::new();
    w.install(&spec);

    let err = load_synced(&ids(&["acme"]), &w.lock(), &w.store(), NO_TRUST).unwrap_err();

    assert_eq!(err.code(), Some("rules-budget-exceeded"), "{err}");
    assert_eq!(err.budget(), Some((PackBudget::RuleEntries, 4096)), "{err}");
    let SyncedError::Pack(plugin) = &err else {
        panic!("{err}");
    };
    assert_eq!(plugin.budget_plugin(), Some("acme"), "{err}");
}

#[test]
fn a_budget_refusal_names_the_plugin_whose_load_crossed_it() {
    let w = base_then_unread_top(&Spec::new("base").rules(&["a", "b"]));

    let e = run_error(&w, small(|l| l.run_rules = 1));

    assert_eq!(e.budget_plugin(), Some("base"), "{e}");
}

// The 2 MiB per-file cap is that of a rule or category file. A genre, tagger or config file from
// a source that has no file cap of its own is bounded by the byte budgets only.
#[test]
fn a_three_mebibyte_config_from_an_embedded_source_loads() {
    use slopvac_packs::EmbeddedSource;
    let config: &'static str = Box::leak(format!("# {}\n", "x".repeat(3 * MIB)).into_boxed_str());
    let files: &'static [(&'static str, &'static str)] =
        Box::leak(Box::new([("c/big.toml", config)]));
    let tables: &'static [&'static [(&'static str, &'static str)]] = Box::leak(Box::new([files]));
    let manifest = "schema_version = 3\nid = \"acme\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"data\"\n\n[provides]\nconfigs = [\"c/big.toml\"]\n";

    let loaded = LoadedPack::load(&EmbeddedSource::new(manifest, tables), Source::Pack);

    assert!(loaded.is_ok(), "{:?}", loaded.err());
}

#[rstest]
#[case::toml_that_does_not_parse("schema_version = 3\nid = \"acme\nversion = [\n")]
#[case::a_key_the_contract_does_not_know(
    "schema_version = 3\nid = \"acme\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"data\"\nextra = 1\n\n[provides]\n"
)]
fn a_malformed_manifest_over_the_run_byte_allowance_is_a_run_byte_refusal(
    #[case] manifest: &'static str,
) {
    use slopvac_packs::EmbeddedSource;
    // The manifest fits the per-plugin cap and crosses the 10-byte run allowance.
    let mut budget = Budget::new(small(|l| l.run_bytes = 10));

    let e = LoadedPack::load_within(
        &EmbeddedSource::new(manifest, &[]),
        Source::Pack,
        &mut budget,
    )
    .unwrap_err();

    assert!(
        matches!(
            e,
            PackError::Budget {
                budget: PackBudget::RunBytes,
                limit: 10,
                plugin: None
            }
        ),
        "{e:?}"
    );
}

#[test]
fn a_malformed_manifest_keeps_its_charge_so_the_next_plugin_crosses_the_run_byte_allowance() {
    use slopvac_packs::{EmbeddedSource, ManifestError};
    let malformed = "schema_version = 3\nid = \"acme\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"data\"\nextra = 1\n\n[provides]\n";
    let valid = "schema_version = 3\nid = \"next\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"data\"\n\n[provides]\n";
    // Each manifest fits alone. Both together cross the allowance by one byte.
    let limit = (malformed.len() + valid.len() - 1) as u64;
    let mut budget = Budget::new(small(|l| l.run_bytes = limit));

    let first = LoadedPack::load_within(
        &EmbeddedSource::new(malformed, &[]),
        Source::Pack,
        &mut budget,
    );
    let second =
        LoadedPack::load_within(&EmbeddedSource::new(valid, &[]), Source::Pack, &mut budget);

    assert!(
        matches!(
            &first,
            Err(PackError::Manifest(ManifestError::Invalid { pointer, message }))
                if pointer.is_empty() && message == "unknown key"
        ),
        "{first:?}"
    );
    assert!(
        matches!(
            second,
            Err(PackError::Budget {
                budget: PackBudget::RunBytes,
                limit: l,
                plugin: None
            }) if l == limit
        ),
        "{second:?}"
    );
}
/// A `.tar.gz` of one file.
fn targz_of(name: &str, bytes: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut tar = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    tar.append_data(&mut header, name, bytes).unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&tar.into_inner().unwrap()).unwrap();
    gz.finish().unwrap()
}

/// The cache is untrusted: an attacker who writes a tree and a record that points at it must not
/// get that tree loaded. The tree to load is the one the lock digest names, through the cached
/// archive, so the swapped tree is refused.
#[test]
fn a_forged_record_and_a_swapped_tree_do_not_load_a_data_plugin() {
    let w = World::new();
    let honest_dir = w.write_tree(&Spec::new("acme"));
    let honest_manifest = fs::read(honest_dir.join("slopvac-plugin.toml")).unwrap();
    let honest_hash = tree_hash(&honest_dir).unwrap();
    let archive = targz_of("slopvac-plugin.toml", &honest_manifest);
    let digest = slopvac_plugins::oci::Digest::of(&archive);

    // The same plugin plus a comment, installed twice: where the honest tree belongs, and where
    // the forged record points.
    let mut evil_manifest = honest_manifest;
    evil_manifest.extend_from_slice(b"# evil\n");
    let trees = w.tmp.path().join("store/trees");
    let mut evil_hash = None;
    for (i, name) in ["swapped", "forged"].into_iter().enumerate() {
        let dir = w.tmp.path().join("stage").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("slopvac-plugin.toml"), &evil_manifest).unwrap();
        let hash = tree_hash(&dir).unwrap();
        let place = if i == 0 { &honest_hash } else { &hash };
        fs::rename(&dir, trees.join(format!("tree-{}", place.hex()))).unwrap();
        evil_hash = Some(hash);
    }
    let blobs = w.tmp.path().join("store/blobs");
    fs::create_dir_all(&blobs).unwrap();
    fs::write(blobs.join(digest.hex()), &archive).unwrap();
    fs::write(
        w.tmp.path().join("store/refs").join(digest.hex()),
        evil_hash.unwrap().hex(),
    )
    .unwrap();
    let lock = Lockfile::parse(&format!(
        "schema_version = 3\n\n[[plugins]]\nid = \"acme\"\nkind = \"data\"\nversion = \"1.0.0\"\n\
         slopvac_api = 1\nsource = \"dist/acme.tar.gz\"\ndigest = \"{digest}\"\n"
    ))
    .unwrap();

    let err = load_synced(&ids(&["acme"]), &lock, &w.store(), NO_TRUST).unwrap_err();

    assert!(matches!(err, SyncedError::Tampered { .. }), "{err}");
    assert_eq!(err.code(), Some("plugin-tampered"));
}
