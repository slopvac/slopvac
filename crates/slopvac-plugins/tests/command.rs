//! The `slopvac plugins` command group through its public `command()` and `run()`.
//!
//! The network, the revocation source and the plugin loader are the boundaries the project
//! owns, so they are fakes here: a local registry server, a local revocation-list server and a
//! loader over a temporary tree.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

mod common;
mod oci_server;
mod revocations;
mod support;

use std::collections::BTreeMap;
use std::error::Error as _;
use std::fs;
use std::path::{Path, PathBuf};

use oci_server::{Response, Server};
use revocations::{ListServer, Root, Signed, list_json};
use slopvac_messages::Catalog;
use slopvac_plugins::extract::tree_hash;
use slopvac_plugins::lock::{PackId, VersionReq, parse_version_req};
use slopvac_plugins::oci::{Anonymous, Client};
use slopvac_plugins::pyenv::EnvStore;
use slopvac_plugins::revoke::Revocations;
use slopvac_plugins::sign::KeyPolicy;
use slopvac_plugins::sync::{Network, PythonBuild, SyncError};
use slopvac_plugins::trust::{DeclSummary, TrustRecord, TrustStore, VerifiedPlugin};
use slopvac_plugins::update::UpdateError;
use slopvac_plugins::{Dirs, PluginLoader, PluginSpec, RunError, Runner, command, run};

const NOW: &str = "2026-11-22T00:00:00Z";
const HASH: &str = "sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
const OTHER_HASH: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A project, a cache, a state directory and a revocation source, each in its own place.
struct Fixture {
    base: support::Scratch,
    root: Root,
    list: ListServer,
}

/// What `run` printed and returned.
struct Ran {
    result: Result<u8, RunError>,
    out: String,
}

impl Fixture {
    /// A fixture whose machine has fetched an empty revocation list before.
    fn new() -> Self {
        let fixture = Self::uncached();
        fixture.revocations().refresh(&client()).unwrap();
        fixture
    }

    /// A fixture whose machine never fetched a revocation list.
    fn uncached() -> Self {
        let root = Root::new();
        let empty = Signed::by(
            &root,
            list_json("2026-11-01T00:00:00Z", "2026-12-01T00:00:00Z", &[], &[]),
        );
        let fixture = Self {
            base: support::scratch(),
            root,
            list: ListServer::start(Some(empty)),
        };
        fs::create_dir_all(fixture.project().join(".slopvac")).unwrap();
        fixture
    }

    fn project(&self) -> PathBuf {
        self.base.path().join("project")
    }

    fn cache(&self) -> PathBuf {
        self.base.path().join("cache")
    }

    fn state(&self) -> PathBuf {
        self.base.path().join("state")
    }

    fn write_lock(&self, entries: &[String]) {
        let text = format!("schema_version = 3\n{}", entries.concat());
        fs::write(self.project().join(".slopvac/plugins.lock"), text).unwrap();
    }

    fn revocations(&self) -> Revocations {
        Revocations::new(
            self.state(),
            vec![self.root.public()],
            self.list.source(),
            NOW,
        )
        .unwrap()
    }

    /// Serve a list that revokes `digests`, and cache it, as an earlier online run would.
    fn revoke(&self, digests: &[&str]) {
        self.list.serve(Some(Signed::by(
            &self.root,
            list_json("2026-11-10T00:00:00Z", "2026-12-10T00:00:00Z", &[], digests),
        )));
        self.revocations().refresh(&client()).unwrap();
    }

    /// Run `slopvac plugins <args>` with the given network mode.
    fn run<'a>(
        &self,
        args: &[&str],
        online: bool,
        setup: impl FnOnce(Setup<'a>) -> Setup<'a>,
    ) -> Ran {
        let setup = setup(Setup::default());
        let policy = KeyPolicy::default();
        let revocations = self.revocations();
        let c = client();
        let network = if online {
            Network::Online(&c)
        } else {
            Network::Offline
        };
        let dirs = Dirs::new(self.project(), self.cache(), self.state());
        let mut runner = Runner::new(dirs, network, &policy, &revocations, NOW)
            .with_ranges(setup.ranges)
            .with_configured(setup.configured)
            .with_core_version("3.0.0");
        if let Some(loader) = &setup.loader {
            runner = runner.with_loader(loader);
        }
        if let Some(envs) = setup.envs {
            runner = runner.with_envs(envs);
        }
        if let Some(python) = setup.python {
            runner = runner.with_python(python);
        }
        let words = std::iter::once("plugins").chain(args.iter().copied());
        let matches = command().try_get_matches_from(words).unwrap();
        let mut out = Vec::new();
        let result = run(&matches, &runner, &mut out);
        Ran {
            result,
            out: String::from_utf8(out).unwrap(),
        }
    }
}

#[derive(Default)]
struct Setup<'a> {
    ranges: BTreeMap<PackId, VersionReq>,
    configured: Vec<String>,
    loader: Option<TreeLoader>,
    envs: Option<&'a EnvStore>,
    python: Option<PythonBuild<'a>>,
}

impl<'a> Setup<'a> {
    fn envs(mut self, envs: &'a EnvStore) -> Self {
        self.envs = Some(envs);
        self
    }

    fn python(mut self, python: PythonBuild<'a>) -> Self {
        self.python = Some(python);
        self
    }

    fn range(mut self, id: &str, req: &str) -> Self {
        self.ranges
            .insert(id.parse().unwrap(), parse_version_req(req).unwrap());
        self
    }

    fn configured(mut self, ids: &[&str]) -> Self {
        self.configured = ids.iter().map(ToString::to_string).collect();
        self
    }

    fn loader(mut self, loader: TreeLoader) -> Self {
        self.loader = Some(loader);
        self
    }
}

/// A setup step that gives the run a loader for the code plugin `fake`.
fn with_loader<'a>(f: &Fixture) -> impl FnOnce(Setup<'a>) -> Setup<'a> {
    let loader = TreeLoader::new(f.base.path(), "fake");
    move |s| s.loader(loader)
}

fn client() -> Client {
    Client::with_loopback_http_for_tests(Box::new(Anonymous))
}

/// A loader that hands out one plugin whose tree is a temporary directory.
struct TreeLoader {
    id: String,
    root: PathBuf,
    manifest_digest: String,
}

impl TreeLoader {
    fn new(base: &Path, id: &str) -> Self {
        let root = base.join(format!("tree-{id}"));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("plugin.py"), "print('x')\n").unwrap();
        Self {
            id: id.to_owned(),
            root,
            manifest_digest: OTHER_HASH.to_owned(),
        }
    }
}

impl PluginLoader for TreeLoader {
    fn load(&self, id: &PackId) -> Result<(VerifiedPlugin, PluginSpec), String> {
        if id.as_str() != self.id {
            return Err("no such plugin tree".to_owned());
        }
        let vp = VerifiedPlugin {
            name: self.id.clone(),
            version: Some("1.0.0".into()),
            source: "test".into(),
            root: self.root.clone(),
            tree: tree_hash(&self.root).unwrap(),
            manifest_digest: Some(self.manifest_digest.clone()),
            key_fingerprint: None,
            key_id: None,
        };
        let spec = PluginSpec {
            name: self.id.clone(),
            program: self.root.join("plugin.py"),
            args: vec![],
            cwd: self.root.clone(),
            declaration: common::decl(),
            path_env: std::env::temp_dir().to_string_lossy().into_owned(),
            env: vec![],
        };
        Ok((vp, spec))
    }
}

/// A lock entry for an OCI source.
fn oci_entry(id: &str, kind: &str, version: &str, registry: &str, digest: &str) -> String {
    format!(
        "\n[[plugins]]\nid = \"{id}\"\nkind = \"{kind}\"\nversion = \"{version}\"\n\
         slopvac_api = 1\nsource = \"oci://{registry}/acme/{id}:{version}\"\ndigest = \"{digest}\"\n"
    )
}

/// A lock entry for a project directory.
fn path_entry(id: &str, version: &str, dir: &str, digest: &str) -> String {
    format!(
        "\n[[plugins]]\nid = \"{id}\"\nkind = \"data\"\nversion = \"{version}\"\n\
         slopvac_api = 1\nsource = \"{dir}\"\ndigest = \"{digest}\"\n"
    )
}

/// A registry that lists these tags for `acme/style` and has nothing else.
fn tag_registry(tags: &'static [&str]) -> Server {
    Server::start(move |req| {
        if req.path.starts_with("/v2/acme/style/tags/list") {
            Response::new(200).json(&serde_json::json!({ "tags": tags }))
        } else {
            Response::new(404)
        }
    })
}

fn record(name: &str, hash: &str) -> TrustRecord {
    TrustRecord {
        hash: hash.into(),
        name: name.into(),
        version: None,
        source: "test".into(),
        key_fingerprint: None,
        manifest_digest: None,
        key_id: None,
        trust_key_id: None,
        declaration: DeclSummary::of(&common::decl()),
        recorded_at: NOW.into(),
    }
}

fn names_in_chain(err: &RunError) -> String {
    let mut text = err.to_string();
    let mut next = err.source();
    while let Some(e) = next {
        text.push('\n');
        text.push_str(&e.to_string());
        next = e.source();
    }
    text
}

// ---- the command tree ---------------------------------------------------------------

#[test]
fn the_group_lists_the_contract_subcommands() {
    let names: std::collections::BTreeSet<String> = command()
        .get_subcommands()
        .map(|c| c.get_name().to_owned())
        .collect();
    let expected: std::collections::BTreeSet<String> = [
        "list", "search", "sync", "status", "outdated", "update", "trust", "untrust", "pin",
        "rotate", "new", "validate", "test", "build", "publish",
    ]
    .map(String::from)
    .into();
    assert_eq!(names, expected);
}

#[test]
fn the_help_names_every_subcommand_with_its_text() {
    let help = command().render_help().to_string();
    assert!(
        help.contains("Record trust for the content hash of a code plugin."),
        "{help}"
    );
    assert!(help.contains("untrust"), "{help}");
}

#[test]
fn pin_takes_a_namespace_a_fingerprint_and_an_optional_registry() {
    let m = command()
        .try_get_matches_from([
            "plugins",
            "pin",
            "acme",
            &"a".repeat(64),
            "--registry",
            "r.io",
        ])
        .unwrap();
    let (name, pin) = m.subcommand().unwrap();
    assert_eq!(name, "pin");
    assert_eq!(pin.get_one::<String>("namespace").unwrap(), "acme");
    assert_eq!(pin.get_one::<String>("registry").unwrap(), "r.io");
}

#[rstest::rstest]
#[case::search(&["search", "style"], "search")]
#[case::pin(&["pin", "acme", "00"], "pin")]
#[case::rotate(&["rotate", "acme"], "rotate")]
#[case::new(&["new", "style"], "new")]
#[case::validate(&["validate"], "validate")]
#[case::run_tests(&["test"], "test")]
#[case::build(&["build"], "build")]
#[case::publish(&["publish"], "publish")]
fn a_subcommand_without_an_implementation_refuses_with_exit_2(
    #[case] args: &[&str],
    #[case] name: &str,
) {
    let f = Fixture::new();
    let ran = f.run(args, false, |s| s);
    let err = ran.result.unwrap_err();
    assert_eq!(err.code(), "command-not-wired");
    assert_eq!(err.exit_code(), 2);
    assert_eq!(
        err.to_string(),
        format!("`slopvac plugins {name}` is not available in this build yet.")
    );
    assert_eq!(ran.out, "");
}

// ---- status -------------------------------------------------------------------------

#[test]
fn status_with_no_lock_reports_nothing_and_passes() {
    let f = Fixture::new();
    let ran = f.run(&["status"], false, |s| s);
    assert_eq!(ran.result.unwrap(), 0);
    assert_eq!(ran.out, "");
}

#[test]
fn status_names_the_kind_and_says_a_plugin_is_not_installed() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("style", "data", "1.2.0", "r.example", OTHER_HASH)]);
    let ran = f.run(&["status"], false, |s| s);
    assert_eq!(ran.result.unwrap(), 2);
    assert_eq!(
        ran.out,
        "style 1.2.0 (data): not installed; run `slopvac plugins sync`\n"
    );
}

#[test]
fn status_flags_a_plugin_that_the_revocation_list_names() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("style", "data", "1.2.0", "r.example", OTHER_HASH)]);
    f.revoke(&[OTHER_HASH]);
    let ran = f.run(&["status"], false, |s| s);
    assert_eq!(ran.result.unwrap(), 2);
    assert_eq!(
        ran.out,
        "style 1.2.0 (data): not installed; run `slopvac plugins sync`; revoked\n"
    );
}

#[test]
fn status_says_a_code_plugin_is_not_trusted_until_trust_is_recorded() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("fake", "code", "1.0.0", "r.example", OTHER_HASH)]);
    let before = f.run(&["status"], false, with_loader(&f));
    assert_eq!(before.result.unwrap(), 2);
    assert_eq!(
        before.out,
        "fake 1.0.0 (code): not installed; run `slopvac plugins sync`; \
         not trusted; run `slopvac plugins trust fake`\n"
    );
    f.run(&["trust", "fake"], false, with_loader(&f))
        .result
        .unwrap();
    let after = f.run(&["status"], false, with_loader(&f));
    assert_eq!(
        after.out,
        "fake 1.0.0 (code): not installed; run `slopvac plugins sync`; trusted\n"
    );
}

#[test]
fn status_names_an_unreadable_lock_and_keeps_the_cause() {
    let f = Fixture::new();
    f.write_lock(&["not a lock".to_owned()]);
    let err = f.run(&["status"], false, |s| s).result.unwrap_err();
    assert_eq!(err.code(), "plugins-lock-invalid");
    assert_eq!(err.exit_code(), 2);
    assert!(err.source().is_some(), "{err:?}");
}

// ---- list ---------------------------------------------------------------------------

#[test]
fn list_shows_the_core_each_locked_plugin_and_each_configured_one_not_locked() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("style", "data", "1.2.0", "r.example", OTHER_HASH)]);
    let ran = f.run(&["list"], false, |s| s.configured(&["style", "extra"]));
    assert_eq!(ran.result.unwrap(), 0);
    assert_eq!(
        ran.out,
        "core 3.0.0: built in\n\
         style 1.2.0 (data): not installed; run `slopvac plugins sync`\n\
         extra: named in the config, not locked; run `slopvac plugins update`\n"
    );
}

// ---- sync ---------------------------------------------------------------------------

fn in_place_project(f: &Fixture) -> String {
    let dir = f.project().join("plugins/style");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("rules.yml"), "a: b\n").unwrap();
    tree_hash(&dir).unwrap().to_string()
}

#[test]
fn sync_installs_the_lock_and_tells_a_missing_revocation_list() {
    let f = Fixture::uncached();
    let digest = in_place_project(&f);
    f.write_lock(&[path_entry("style", "1.0.0", "plugins/style", &digest)]);
    let ran = f.run(&["sync"], false, |s| s);
    assert_eq!(ran.result.unwrap(), 0);
    assert!(
        ran.out.starts_with("style 1.0.0: in place\n"),
        "{}",
        ran.out
    );
    assert!(
        ran.out.contains("notice[revocation-list-missing]:"),
        "{}",
        ran.out
    );
}

#[test]
fn sync_keeps_its_verification_a_revoked_digest_stops_it_with_the_cause() {
    let f = Fixture::new();
    let digest = in_place_project(&f);
    f.write_lock(&[path_entry("style", "1.0.0", "plugins/style", &digest)]);
    f.revoke(&[&digest]);
    let err = f.run(&["sync"], true, |s| s).result.unwrap_err();
    assert_eq!(err.code(), "plugins-sync-failed");
    assert!(
        matches!(
            err.source()
                .and_then(|e| e.downcast_ref::<Box<SyncError>>())
                .map(|b| &**b),
            Some(SyncError::Revoked { .. })
        ),
        "{err:?}"
    );
}

// ---- outdated and update ------------------------------------------------------------

#[test]
fn outdated_lists_a_newer_version_from_the_registry() {
    let f = Fixture::new();
    let server = tag_registry(&["1.2.0", "1.3.0"]);
    f.write_lock(&[oci_entry(
        "style",
        "data",
        "1.2.0",
        &server.registry(),
        OTHER_HASH,
    )]);
    let ran = f.run(&["outdated"], true, |s| s.range("style", "^1.2"));
    assert_eq!(ran.result.unwrap(), 0);
    assert!(
        ran.out.contains("style") && ran.out.contains("1.3.0"),
        "{}",
        ran.out
    );
}

#[test]
fn outdated_offline_needs_the_network_and_says_so_with_the_shared_code() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("style", "data", "1.2.0", "r.example", OTHER_HASH)]);
    let err = f.run(&["outdated"], false, |s| s).result.unwrap_err();
    assert_eq!(err.code(), "offline-network-needed");
    assert_eq!(
        err.to_string(),
        "offline: plugins outdated needs the network"
    );
}

#[test]
fn update_with_nothing_newer_leaves_the_lock_and_passes() {
    let f = Fixture::new();
    let server = tag_registry(&["1.2.0"]);
    f.write_lock(&[oci_entry(
        "style",
        "data",
        "1.2.0",
        &server.registry(),
        OTHER_HASH,
    )]);
    let ran = f.run(&["update"], true, |s| s.range("style", "^1.2"));
    assert_eq!(ran.result.unwrap(), 0);
    assert_eq!(ran.out, "style: unchanged at 1.2.0\n");
}

#[test]
fn update_refuses_a_newer_version_it_cannot_fetch_and_exits_2() {
    let f = Fixture::new();
    let server = tag_registry(&["1.2.0", "1.3.0"]);
    f.write_lock(&[oci_entry(
        "style",
        "data",
        "1.2.0",
        &server.registry(),
        OTHER_HASH,
    )]);
    let ran = f.run(&["update"], true, |s| s.range("style", "^1.2"));
    assert_eq!(ran.result.unwrap(), 2);
    assert!(ran.out.starts_with("style: refused: "), "{}", ran.out);
}

#[test]
fn update_offline_fails_with_the_update_error_as_its_cause() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("style", "data", "1.2.0", "r.example", OTHER_HASH)]);
    let err = f.run(&["update"], false, |s| s).result.unwrap_err();
    assert_eq!(err.code(), "plugins-update-failed");
    assert!(
        matches!(
            err.source()
                .and_then(|e| e.downcast_ref::<Box<UpdateError>>())
                .map(|b| &**b),
            Some(UpdateError::Offline)
        ),
        "{err:?}"
    );
}

// ---- trust and untrust --------------------------------------------------------------

#[test]
fn trust_records_the_content_hash_and_says_trust_is_not_containment() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("fake", "code", "1.0.0", "r.example", OTHER_HASH)]);
    let ran = f.run(&["trust", "fake"], false, |s| {
        s.loader(TreeLoader::new(f.base.path(), "fake"))
    });
    assert_eq!(ran.result.unwrap(), 0);
    let records = TrustStore::open(f.state()).records().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        ran.out,
        format!(
            "trusted `fake` with content hash {}\n\
             Trust is not containment: a trusted plugin runs with your rights, and slopvac has no sandbox.\n",
            records[0].hash
        )
    );
}

#[test]
fn trust_of_a_data_plugin_exits_2_with_plugin_not_code_and_records_nothing() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("style", "data", "1.2.0", "r.example", OTHER_HASH)]);
    let err = f
        .run(&["trust", "style"], false, |s| {
            s.loader(TreeLoader::new(f.base.path(), "style"))
        })
        .result
        .unwrap_err();
    assert_eq!(err.code(), "plugin-not-code");
    assert_eq!(err.exit_code(), 2);
    assert_eq!(TrustStore::open(f.state()).records().unwrap().len(), 0);
}

#[test]
fn trust_refuses_a_plugin_that_the_revocation_list_names() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("fake", "code", "1.0.0", "r.example", OTHER_HASH)]);
    f.revoke(&[OTHER_HASH]);
    let err = f
        .run(&["trust", "fake"], false, |s| {
            s.loader(TreeLoader::new(f.base.path(), "fake"))
        })
        .result
        .unwrap_err();
    assert_eq!(err.code(), "plugins-trust-revoked-plugin");
    assert_eq!(TrustStore::open(f.state()).records().unwrap().len(), 0);
}

#[test]
fn trust_of_a_plugin_that_is_not_locked_says_so() {
    let f = Fixture::new();
    let err = f.run(&["trust", "fake"], false, |s| s).result.unwrap_err();
    assert_eq!(err.code(), "plugins-unknown-plugin");
}

#[test]
fn untrust_removes_the_trust_recorded_for_a_plugin() {
    let f = Fixture::new();
    let store = TrustStore::open(f.state());
    store.record(record("fake", HASH)).unwrap();
    store.record(record("other", OTHER_HASH)).unwrap();
    let ran = f.run(&["untrust", "fake"], false, |s| s);
    assert_eq!(ran.result.unwrap(), 0);
    assert_eq!(ran.out, "removed the trust recorded for `fake`\n");
    let left: Vec<String> = store
        .records()
        .unwrap()
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(left, ["other"]);
}

#[test]
fn untrust_of_a_plugin_with_no_record_exits_2_with_its_code() {
    let f = Fixture::new();
    let err = f
        .run(&["untrust", "fake"], false, |s| s)
        .result
        .unwrap_err();
    assert_eq!(err.code(), "plugins-untrust-not-trusted");
    assert_eq!(err.exit_code(), 2);
}

#[test]
fn untrust_of_a_data_plugin_exits_2_with_plugin_not_code_and_changes_nothing() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("style", "data", "1.2.0", "r.example", OTHER_HASH)]);
    let store = TrustStore::open(f.state());
    store.record(record("style", HASH)).unwrap();
    let err = f
        .run(&["untrust", "style"], false, |s| s)
        .result
        .unwrap_err();
    assert_eq!(err.code(), "plugin-not-code");
    assert_eq!(store.records().unwrap().len(), 1);
}

#[test]
fn a_corrupt_trust_store_is_reported_without_the_absolute_state_path() {
    let f = Fixture::new();
    fs::create_dir_all(f.state()).unwrap();
    fs::write(f.state().join("trust.json"), "not json").unwrap();
    let err = f
        .run(&["untrust", "fake"], false, |s| s)
        .result
        .unwrap_err();
    assert_eq!(err.code(), "plugins-store-failed");
    let shown = names_in_chain(&err);
    let base = f.base.path().to_string_lossy().into_owned();
    assert!(!shown.contains(&base), "absolute path leaked: {shown}");
    assert!(shown.contains("<external>/trust.json"), "{shown}");
}

// ---- status: managed Python environments ----------------------------------------------

#[cfg(unix)]
mod python_env {
    use super::*;
    use crate::common::fake_uv::{Fake, fake_uv};

    const MANIFEST: &str = "schema_version = 3\nid = \"py\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"code\"\n\n[plugin]\ncommand = \"plugin.py\"\nruntime = \"python\"\npython = \"3.12\"\nnetwork = false\nhooks = []\nfeatures = { required = [], optional = [] }\n";
    const PYLOCK: &str = "lock-version = \"1.0\"\ncreated-by = \"test\"\n";

    /// A project with one in-place Python plugin `py`, a fake uv and an environment store.
    struct Project {
        f: Fixture,
        uv: PathBuf,
        envs: EnvStore,
    }

    fn project() -> Project {
        let f = Fixture::new();
        let dir = f.project().join("plugins/py");
        fs::create_dir_all(&dir).unwrap();
        for (name, text) in [
            ("slopvac-plugin.toml", MANIFEST),
            ("plugin.py", "print('plugin')\n"),
            ("pylock.toml", PYLOCK),
        ] {
            fs::write(dir.join(name), text).unwrap();
        }
        let digest = tree_hash(&dir).unwrap();
        f.write_lock(&[format!(
            "\n[[plugins]]\nid = \"py\"\nkind = \"code\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
             source = \"plugins/py\"\ndigest = \"{digest}\"\n"
        )]);
        let uv = fake_uv(&f.base.path().join("uv-bin"), &Fake::real());
        let envs = EnvStore::open(f.base.path().join("envs"));
        Project { f, uv, envs }
    }

    impl Project {
        fn build(&self) -> PythonBuild<'_> {
            PythonBuild {
                envs: &self.envs,
                uv: &self.uv,
                python_install_dir: Path::new("/nonexistent/uv-python"),
            }
        }

        fn status(&self) -> Ran {
            self.f.run(&["status"], false, |s| s.envs(&self.envs))
        }
    }

    #[test]
    fn status_reports_a_built_environment_as_verified() {
        let p = project();
        p.f.run(&["sync"], false, |s| s.python(p.build()))
            .result
            .unwrap();
        let ran = p.status();
        assert_eq!(ran.result.unwrap(), 0);
        assert_eq!(
            ran.out,
            "py 1.0.0 (code): installed; trust state unknown: the plugin files are not loaded; \
             python environment verified\n"
        );
    }

    #[test]
    fn status_reports_an_environment_that_sync_never_built_as_missing_and_exits_2() {
        let p = project();
        let ran = p.status();
        assert_eq!(ran.result.unwrap(), 2);
        assert_eq!(
            ran.out,
            "py 1.0.0 (code): installed; trust state unknown: the plugin files are not loaded; \
             python environment missing; run `slopvac plugins sync`\n"
        );
    }
}
