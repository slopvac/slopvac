//! Synced packs load from the config list, the lock and the store.
//!
//! The store is built by hand in the layout its documentation gives (`trees/tree-<hex>` and
//! `refs/<digest hex>`), so the tests need no registry and no signing.

use std::fs;
use std::path::{Path, PathBuf};

use proptest::prelude::*;
use rstest::rstest;
use slopvac_config::Presets;
use slopvac_model::PackId;
use slopvac_packs::{
    Budget, LimitKind, Limits, LoadedPack, PackError, Packs, Reference, SyncedError, load_synced,
    load_synced_within,
};
use slopvac_plugins::extract::tree_hash;
use slopvac_plugins::lock::Lockfile;
use slopvac_plugins::sync::Store;

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
            "schema_version = 3\nid = \"{}\"\nversion = \"{}\"\nslopvac_api = {}\nkind = \"data\"\n\n[provides]\n{}\n",
            self.id, self.version, self.api, self.provides
        );
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
        self.lock_entry(spec);
        let hash = tree_hash(&tree).unwrap();
        let dest = self
            .tmp
            .path()
            .join("store/trees")
            .join(format!("tree-{}", hash.hex()));
        fs::rename(&tree, &dest).unwrap();
        fs::write(
            self.tmp.path().join("store/refs").join(self.digest_hex()),
            hash.hex(),
        )
        .unwrap();
        dest
    }

    fn write_tree(&self, spec: &Spec) -> PathBuf {
        let tree = self.tmp.path().join("stage").join(spec.id);
        fs::create_dir_all(&tree).unwrap();
        fs::write(tree.join("slopvac-plugin.toml"), spec.manifest()).unwrap();
        for (name, text) in &spec.files {
            let target = tree.join(name);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, text).unwrap();
        }
        tree
    }

    fn digest_hex(&self) -> String {
        format!("{:064x}", self.count)
    }

    /// A lock entry for `spec` with a digest of its own, whether or not the store holds it.
    fn lock_entry(&mut self, spec: &Spec) {
        self.count += 1;
        let requires = spec
            .locked_requires
            .iter()
            .map(|r| format!("\"{r}\""))
            .collect::<Vec<_>>()
            .join(", ");
        self.entries.push(format!(
            "\n[[plugins]]\nid = \"{}\"\nkind = \"data\"\nversion = \"{}\"\nslopvac_api = {}\nsource = \"dist/{}.tar.gz\"\ndigest = \"sha256:{}\"\nrequires = [{requires}]\n",
            spec.id,
            spec.version,
            spec.locked_api,
            spec.id,
            self.digest_hex(),
        ));
    }

    fn lock(&self) -> Lockfile {
        Lockfile::parse(&format!("schema_version = 3\n{}", self.entries.concat())).unwrap()
    }

    fn load(&self, requested: &[&str]) -> Result<Vec<LoadedPack>, SyncedError> {
        load_synced(&ids(requested), &self.lock(), &self.store())
    }
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
fn a_tampered_pack_tree_fails_the_load_with_a_typed_error() {
    let mut w = World::new();
    let tree = w.install(&Spec::new("acme"));
    tamper(&tree);

    let err = w.load(&["acme"]).unwrap_err();

    assert!(matches!(err, SyncedError::Tampered { .. }), "{err}");
    assert_eq!(err.code(), Some("plugin-tampered"));
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

    let err = load_synced(&ids(&["acme"]), &lock, &w.store()).unwrap_err();

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
fn a_synced_pack_registers_no_taggers() {
    let mut w = World::new();
    let mut spec = Spec::new("acme");
    spec.provides = "taggers = [\"taggers/t.yaml\"]".into();
    spec.files.push((
        "taggers/t.yaml",
        "schema_version: 3\nid: t\ntag: procedure\nmatch:\n  - pattern: 'a'\n".to_owned(),
    ));
    w.install(&spec);

    let packs = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store()).unwrap();

    assert_eq!(
        packs.taggers().count(),
        Packs::core().unwrap().taggers().count()
    );
}

#[test]
fn a_bare_genre_name_that_a_synced_pack_shares_with_the_core_is_ambiguous() {
    let mut w = World::new();
    w.install(&Spec::new("acme").genre("consumer", "profile: strict\n"));
    let packs = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store()).unwrap();
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
    let packs = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store()).unwrap();
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
    let packs = Packs::with_synced(&ids(&["acme"]), &w.lock(), &w.store()).unwrap();

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
    match load_synced_within(&ids(&["top"]), &w.lock(), &w.store(), &mut budget) {
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
            PackError::RunLimit {
                what: LimitKind::Bytes,
                limit: 10
            }
        ),
        "{e}"
    );
    assert_eq!(e.code(), Some("plugin-run-limit-exceeded"));
}

#[test]
fn the_pack_byte_budget_names_the_pack() {
    let w = base_then_unread_top(&Spec::new("base").genre("memo", ""));

    let e = run_error(&w, small(|l| l.pack_bytes = 10));

    assert!(
        matches!(&e, PackError::Limit { id, what: LimitKind::Bytes, .. } if id == "base"),
        "{e}"
    );
    assert_eq!(e.code(), Some("plugin-limit-exceeded"));
}

#[test]
fn the_run_rule_budget_is_checked_before_a_pattern_compiles() {
    // The second pattern does not compile. A load that compiled first would report that.
    let w = base_then_unread_top(&Spec::new("base").rules(&["a", "("]));

    let e = run_error(&w, small(|l| l.run_rules = 1));

    assert!(
        matches!(
            e,
            PackError::RunLimit {
                what: LimitKind::Rules,
                limit: 1
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
            PackError::RunLimit {
                what: LimitKind::PatternBytes,
                limit: 5
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
        small(|l| l.run_rules = 1),
    )
    .unwrap_err();

    assert!(
        matches!(
            err,
            SyncedError::Pack(PackError::RunLimit {
                what: LimitKind::Rules,
                ..
            })
        ),
        "{err}"
    );
}

#[rstest]
#[case::bytes(|l: &mut Limits| l.run_bytes = 10, LimitKind::Bytes)]
#[case::rules(|l: &mut Limits| l.run_rules = 1, LimitKind::Rules)]
#[case::patterns(|l: &mut Limits| l.run_pattern_bytes = 5, LimitKind::PatternBytes)]
fn registering_packs_applies_the_same_budgets_as_loading_them(
    #[case] set: fn(&mut Limits),
    #[case] what: LimitKind,
) {
    let mut w = World::new();
    w.install(&Spec::new("acme").rules(&["abcdef", "ghi"]));
    // Loaded under the default limits, then registered under small ones.
    let packs = load_synced(&ids(&["acme"]), &w.lock(), &w.store()).unwrap();

    let err = Packs::new_within(packs, small(set)).unwrap_err();

    assert!(
        matches!(err, PackError::RunLimit { what: w, .. } if w == what),
        "{err}"
    );
}

#[rstest]
#[case::pack_error(SyncedError::Pack(PackError::Limit { id: "a".into(), what: LimitKind::Files, limit: 1 }), Some("plugin-limit-exceeded"))]
#[case::pack_run_limit(SyncedError::Pack(PackError::RunLimit { what: LimitKind::Bytes, limit: 1 }), Some("plugin-run-limit-exceeded"))]
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
#[case::api(SyncedError::ApiUnsupported { id: "a".into(), api: 9 }, Some("plugin-api-unsupported"))]
#[case::mismatch(SyncedError::ManifestMismatch { id: "a".into(), field: "id" }, Some("plugin-manifest-mismatch"))]
#[case::cycle(SyncedError::RequiresCycle { cycle: vec!["a".into(), "a".into()] }, Some("plugin-requires-cycle"))]
#[case::unmet(SyncedError::RequirementUnmet { id: "a".into(), needs: "b".into(), requirement: ">=1".into() }, Some("plugin-requirement-unmet"))]
fn every_error_reports_its_catalog_code(#[case] error: SyncedError, #[case] code: Option<&str>) {
    assert_eq!(error.code(), code);
}
