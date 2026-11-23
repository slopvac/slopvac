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
use slopvac_plugins::extract::tree_hash;
use slopvac_plugins::lock::{PackId, VersionReq, parse_version_req};
use slopvac_plugins::oci::{Anonymous, Client};
use slopvac_plugins::pyenv::EnvStore;
use slopvac_plugins::revoke::Revocations;
use slopvac_plugins::sign::KeyPolicy;
use slopvac_plugins::sync::{Network, PythonBuild};
use slopvac_plugins::trust::{DeclSummary, TrustGate, TrustRecord, TrustStore, VerifiedPlugin};
use slopvac_plugins::{Dirs, PluginLoader, PluginSpec, RunError, Runner, SpawnGate, command, run};

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
        let mut out = Vec::new();
        let result = self.run_into(args, online, setup, &mut out);
        Ran {
            result,
            out: String::from_utf8(out).unwrap(),
        }
    }

    /// [`Fixture::run`] writing to `out`.
    fn run_into<'a>(
        &self,
        args: &[&str],
        online: bool,
        setup: impl FnOnce(Setup<'a>) -> Setup<'a>,
        out: &mut dyn std::io::Write,
    ) -> Result<u8, RunError> {
        let setup = setup(Setup::default());
        let policy = setup.policy.unwrap_or_default();
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
            runner = runner.with_loader(loader.as_ref());
        }
        if let Some(envs) = setup.envs {
            runner = runner.with_envs(envs);
        }
        if let Some(python) = setup.python {
            runner = runner.with_python(python);
        }
        runner = runner
            .with_executables(setup.executables)
            .with_locked(setup.locked);
        let words = std::iter::once("plugins").chain(args.iter().copied());
        let matches = command().try_get_matches_from(words).unwrap();
        run(&matches, &runner, out)
    }
}

#[derive(Default)]
struct Setup<'a> {
    ranges: BTreeMap<PackId, VersionReq>,
    configured: Vec<String>,
    loader: Option<Box<dyn PluginLoader + 'a>>,
    policy: Option<KeyPolicy>,
    executables: BTreeMap<PackId, std::collections::HashSet<String>>,
    envs: Option<&'a EnvStore>,
    python: Option<PythonBuild<'a>>,
    locked: bool,
}

impl<'a> Setup<'a> {
    fn locked(mut self) -> Self {
        self.locked = true;
        self
    }


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

    fn loader(mut self, loader: impl PluginLoader + 'a) -> Self {
        self.loader = Some(Box::new(loader));
        self
    }

    fn executables(
        mut self,
        executables: BTreeMap<PackId, std::collections::HashSet<String>>,
    ) -> Self {
        self.executables = executables;
        self
    }

    fn policy(mut self, policy: KeyPolicy) -> Self {
        self.policy = Some(policy);
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
    manifest_digest: Option<String>,
}

impl TreeLoader {
    fn new(base: &Path, id: &str) -> Self {
        let root = base.join(format!("tree-{id}"));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("plugin.py"), "print('x')\n").unwrap();
        Self {
            id: id.to_owned(),
            root,
            manifest_digest: Some(OTHER_HASH.to_owned()),
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
            manifest_digest: self.manifest_digest.clone(),
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
    assert_eq!(err.code(), "plugin-revoked");
    assert!(err.to_string().contains("revocation list"), "{err}");
}

// ---- lock errors and CI ----------------------------------------------------------------

#[test]
fn a_lock_that_does_not_parse_reports_where_and_never_what_it_said() {
    let f = Fixture::new();
    // A rejected value that a message could echo, under a key that is not a lock key.
    f.write_lock(&[
        "\n[[plugins]]\nid = \"style\"\nkind = \"data\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
         source = \"plugins/style\"\ndigest = \"SECRET-VALUE\"\n"
            .to_owned(),
    ]);
    let err = f.run(&["status"], false, |s| s).result.unwrap_err();
    assert_eq!(err.code(), "plugins-lock-invalid");
    assert_eq!(err.to_string(), "invalid value at /plugins/0/digest");
}

#[test]
fn sync_in_ci_without_a_lock_fails_and_outside_ci_passes() {
    let f = Fixture::new();
    assert_eq!(f.run(&["sync"], false, |s| s).result.unwrap(), 0);
    let err = f.run(&["sync"], false, Setup::locked).result.unwrap_err();
    assert_eq!(err.code(), "plugins-lock-unreadable");
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
fn outdated_names_the_failure_by_its_code_and_never_echoes_the_registry() {
    let f = Fixture::new();
    // Nothing listens on this port, so the listing fails at connect.
    f.write_lock(&[oci_entry("style", "data", "1.2.0", "127.0.0.1:1", OTHER_HASH)]);
    let ran = f.run(&["outdated"], true, |s| s.range("style", "^1.2"));
    assert_eq!(ran.result.unwrap(), 2);
    assert!(ran.out.starts_with("style: cannot check for newer versions: "), "{}", ran.out);
    assert!(!ran.out.contains("127.0.0.1"), "{}", ran.out);
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
    // The line names the code of the retained cause and no free text.
    let reason = ran.out.trim_end().strip_prefix("style: refused: ").unwrap();
    assert!(
        reason.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
        "{reason}"
    );
}

#[test]
fn update_offline_fails_with_the_update_error_as_its_cause() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("style", "data", "1.2.0", "r.example", OTHER_HASH)]);
    let err = f.run(&["update"], false, |s| s).result.unwrap_err();
    assert_eq!(err.code(), "offline-network-needed");
    assert_eq!(err.to_string(), "offline: plugins update needs the network");
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
    assert!(shown.contains("withheld"), "{shown}");
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

    /// Hands out the in-place plugin `py` as its files are on disk.
    struct PyLoader {
        root: PathBuf,
    }

    impl PyLoader {
        fn parts(&self) -> (VerifiedPlugin, PluginSpec) {
            let vp = VerifiedPlugin {
                name: "py".into(),
                version: Some("1.0.0".into()),
                source: "test".into(),
                root: self.root.clone(),
                tree: tree_hash(&self.root).unwrap(),
                manifest_digest: None,
                key_fingerprint: None,
                key_id: None,
            };
            let spec = PluginSpec {
                name: "py".into(),
                program: self.root.join("plugin.py"),
                args: vec![],
                cwd: self.root.clone(),
                declaration: common::decl(),
                path_env: std::env::temp_dir().to_string_lossy().into_owned(),
                env: vec![],
            };
            (vp, spec)
        }
    }

    impl PluginLoader for PyLoader {
        fn load(&self, _: &PackId) -> Result<(VerifiedPlugin, PluginSpec), String> {
            Ok(self.parts())
        }
    }

    #[test]
    fn trust_for_a_managed_python_plugin_is_accepted_by_the_production_spawn_gate() {
        let p = project();
        p.f.run(&["sync"], false, |s| s.python(p.build()))
            .result
            .unwrap();
        let loader = PyLoader {
            root: p.f.project().join("plugins/py"),
        };
        let ran = p.f.run(&["trust", "py"], false, |s| {
            s.envs(&p.envs).loader(PyLoader {
                root: loader.root.clone(),
            })
        });
        assert_eq!(ran.result.unwrap(), 0, "{}", ran.out);
        // The spawn gate the host uses, wired to the managed environment of the plugin.
        let (vp, spec) = loader.parts();
        let gate = TrustGate::new(TrustStore::open(p.f.state()), vec![], vec![vp])
            .with_python_plugins(&p.envs)
            .unwrap()
            .with_revocation_cache(p.f.revocations());
        let (program, args) = gate.python_launch("py").unwrap();
        let spec = PluginSpec {
            program,
            args,
            ..spec
        };
        gate.check(&spec).unwrap();
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
// ---- round 3: trust, revocation, codes, redaction, output -------------------------------

/// A loader that hands out a plugin with a program path that the spec must be refused for.
struct OutsideProgram {
    inner: TreeLoader,
    program: PathBuf,
}

impl PluginLoader for OutsideProgram {
    fn load(&self, id: &PackId) -> Result<(VerifiedPlugin, PluginSpec), String> {
        let (vp, mut spec) = self.inner.load(id)?;
        spec.program.clone_from(&self.program);
        Ok((vp, spec))
    }
}

#[test]
fn trust_refuses_a_locked_digest_that_the_revocation_list_names_even_when_the_loader_has_none() {
    let f = Fixture::new();
    // The loader's plugin carries no manifest digest or key id, as a path or git source does.
    let mut loader = TreeLoader::new(f.base.path(), "fake");
    loader.manifest_digest = None;
    f.write_lock(&[code_path_entry("fake", "plugins/fake", OTHER_HASH)]);
    f.revoke(&[OTHER_HASH]);
    let err = f
        .run(&["trust", "fake"], false, |s| s.loader(loader))
        .result
        .unwrap_err();
    assert_eq!(err.code(), "plugins-trust-revoked-plugin");
    assert_eq!(TrustStore::open(f.state()).records().unwrap().len(), 0);
}

#[test]
fn an_unsigned_oci_entry_stops_sync_with_sync_unsigned() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("style", "data", "1.2.0", "r.example", OTHER_HASH)]);
    let err = f.run(&["sync"], false, |s| s).result.unwrap_err();
    assert_eq!(err.code(), "sync-unsigned");
}

#[test]
fn a_revoked_digest_stops_sync_with_plugin_revoked() {
    let f = Fixture::new();
    let digest = in_place_project(&f);
    f.write_lock(&[path_entry("style", "1.0.0", "plugins/style", &digest)]);
    f.revoke(&[&digest]);
    let err = f.run(&["sync"], true, |s| s).result.unwrap_err();
    assert_eq!(err.code(), "plugin-revoked");
}

#[test]
fn an_offline_update_keeps_the_code_of_its_cause() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("style", "data", "1.2.0", "r.example", OTHER_HASH)]);
    let err = f.run(&["update"], false, |s| s).result.unwrap_err();
    assert_eq!(err.code(), "offline-network-needed");
}

#[test]
fn a_damaged_revocation_cache_keeps_its_code_and_names_no_absolute_path() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("style", "data", "1.2.0", "r.example", OTHER_HASH)]);
    fs::write(f.revocations().cache_file(), "not a list").unwrap();
    let err = f.run(&["status"], false, |s| s).result.unwrap_err();
    assert_eq!(err.code(), "revocation-cache-invalid");
    let shown = names_in_chain(&err);
    let base = f.base.path().to_string_lossy().into_owned();
    assert!(!shown.contains(&base), "absolute path leaked: {shown}");
}

#[test]
fn a_refused_program_path_is_withheld_and_never_absolute() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("fake", "code", "1.0.0", "r.example", OTHER_HASH)]);
    let outside = f.base.path().join("elsewhere/prog");
    let loader = OutsideProgram {
        inner: TreeLoader::new(f.base.path(), "fake"),
        program: outside,
    };
    let err = f
        .run(&["trust", "fake"], false, |s| s.loader(loader))
        .result
        .unwrap_err();
    assert_eq!(err.code(), "plugin-usage");
    let shown = names_in_chain(&err);
    let base = f.base.path().to_string_lossy().into_owned();
    assert!(!shown.contains(&base), "absolute path leaked: {shown}");
    assert!(!shown.contains("prog"), "{shown}");
    assert!(shown.contains("withheld"), "{shown}");
}

/// A writer whose every write fails the way a closed pipe does.
struct ClosedPipe;

impl std::io::Write for ClosedPipe {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn an_output_failure_keeps_the_io_error_as_its_source() {
    let f = Fixture::new();
    let err = f
        .run_into(&["list"], false, |s| s, &mut ClosedPipe)
        .unwrap_err();
    assert_eq!(err.code(), "plugins-output-failed");
    let kind = err
        .source()
        .and_then(|e| e.downcast_ref::<std::io::Error>())
        .map(std::io::Error::kind);
    assert_eq!(kind, Some(std::io::ErrorKind::BrokenPipe));
}

#[test]
fn trust_without_a_loader_says_so_in_registered_text() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("fake", "code", "1.0.0", "r.example", OTHER_HASH)]);
    let err = f.run(&["trust", "fake"], false, |s| s).result.unwrap_err();
    assert_eq!(
        err.to_string(),
        "cannot load `fake` to record trust: the plugin files are not loaded"
    );
}

/// A lock entry for a code plugin in a project directory.
fn code_path_entry(id: &str, dir: &str, digest: &str) -> String {
    path_entry(id, "1.0.0", dir, digest).replace("kind = \"data\"", "kind = \"code\"")
}
// ---- round 4: catalog codes, redacted causes, trust notices ----------------------------

/// The code through the trait, as a generic caller and as a trait object see it.
fn trait_codes(err: &RunError) -> [&'static str; 3] {
    fn generic<C: slopvac_messages::Catalog>(c: &C) -> &'static str {
        c.code()
    }
    fn dynamic(c: &dyn slopvac_messages::Catalog) -> &'static str {
        c.code()
    }
    [
        slopvac_messages::Catalog::code(err),
        generic(err),
        dynamic(err),
    ]
}

#[test]
fn a_failed_sync_reports_the_code_of_its_cause_through_every_lookup() {
    let f = Fixture::new();
    let digest = in_place_project(&f);
    f.write_lock(&[path_entry("style", "1.0.0", "plugins/style", &digest)]);
    f.revoke(&[&digest]);
    let err = f.run(&["sync"], true, |s| s).result.unwrap_err();
    assert_eq!(err.code(), "plugin-revoked");
    assert_eq!(trait_codes(&err), ["plugin-revoked"; 3]);
}

#[test]
fn a_failed_update_reports_the_code_of_its_cause_through_every_lookup() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("style", "data", "1.2.0", "r.example", OTHER_HASH)]);
    let err = f.run(&["update"], false, |s| s).result.unwrap_err();
    assert_eq!(trait_codes(&err), ["offline-network-needed"; 3]);
}

#[test]
fn a_failed_revocation_read_reports_its_code_through_every_lookup() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("style", "data", "1.2.0", "r.example", OTHER_HASH)]);
    fs::write(f.revocations().cache_file(), "not a list").unwrap();
    let err = f.run(&["status"], false, |s| s).result.unwrap_err();
    assert_eq!(trait_codes(&err), ["revocation-cache-invalid"; 3]);
}

#[test]
fn a_failed_sync_keeps_no_absolute_path_anywhere_in_its_source_chain() {
    let f = Fixture::new();
    let digest = in_place_project(&f);
    f.write_lock(&[path_entry("style", "1.0.0", "plugins/style", &digest)]);
    fs::write(f.revocations().cache_file(), "not a list").unwrap();
    let err = f.run(&["sync"], false, |s| s).result.unwrap_err();
    assert_eq!(err.code(), "revocation-cache-invalid");
    let shown = names_in_chain(&err);
    let base = f.base.path().to_string_lossy().into_owned();
    assert!(!shown.contains(&base), "absolute path leaked: {shown}");
}

/// A loader whose failure text names a local path.
struct FailingLoader;

impl PluginLoader for FailingLoader {
    fn load(&self, _: &PackId) -> Result<(VerifiedPlugin, PluginSpec), String> {
        Err("cannot read /home/someone/secret-dir/plugin.toml".to_owned())
    }
}

#[test]
fn loader_text_is_never_echoed_into_the_failure() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("fake", "code", "1.0.0", "r.example", OTHER_HASH)]);
    let err = f
        .run(&["trust", "fake"], false, |s| s.loader(FailingLoader))
        .result
        .unwrap_err();
    assert_eq!(err.code(), "plugins-trust-inputs-failed");
    let shown = names_in_chain(&err);
    assert!(!shown.contains("secret-dir"), "{shown}");
    assert!(!shown.contains("cannot read"), "{shown}");
    assert_eq!(
        err.to_string(),
        "cannot load `fake` to record trust: plugins-trust-inputs-failed: details are withheld because they can name paths outside the project"
    );
}
#[test]
fn status_names_the_code_beside_a_withheld_loader_failure() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("fake", "code", "1.0.0", "r.example", OTHER_HASH)]);
    let ran = f.run(&["status"], false, |s| s.loader(FailingLoader));
    assert!(
        ran.out.contains(
            "trust state unknown: plugins-part-trust-unknown: details are withheld because they can name paths outside the project"
        ),
        "{}",
        ran.out
    );
}

const STORE_WITHHELD: &str =
    "plugins-store-failed: details are withheld because they can name paths outside the project";

#[test]
fn trust_names_the_code_beside_a_withheld_damaged_store() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("fake", "code", "1.0.0", "r.example", OTHER_HASH)]);
    fs::create_dir_all(f.state()).unwrap();
    fs::write(f.state().join("trust.json"), "not json").unwrap();
    let err = f
        .run(&["trust", "fake"], false, with_loader(&f))
        .result
        .unwrap_err();
    assert_eq!(err.code(), "plugins-store-failed");
    assert_eq!(err.to_string(), STORE_WITHHELD);
}

#[test]
fn status_names_the_code_beside_a_withheld_damaged_store() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("fake", "code", "1.0.0", "r.example", OTHER_HASH)]);
    fs::create_dir_all(f.state()).unwrap();
    fs::write(f.state().join("trust.json"), "not json").unwrap();
    let ran = f.run(&["status"], false, with_loader(&f));
    assert!(
        ran.out
            .contains(&format!("trust state unknown: {STORE_WITHHELD}")),
        "{}",
        ran.out
    );
}

#[test]
fn trust_tells_a_missing_revocation_list_and_still_records_trust_offline() {
    let f = Fixture::uncached();
    f.write_lock(&[oci_entry("fake", "code", "1.0.0", "r.example", OTHER_HASH)]);
    let ran = f.run(&["trust", "fake"], false, with_loader(&f));
    assert_eq!(ran.result.unwrap(), 0);
    assert!(
        ran.out.contains("notice[revocation-list-missing]:"),
        "{}",
        ran.out
    );
    assert_eq!(TrustStore::open(f.state()).records().unwrap().len(), 1);
}

#[test]
fn trust_tells_a_stale_revocation_list_and_still_records_trust() {
    let f = Fixture::uncached();
    f.list.serve(Some(Signed::by(
        &f.root,
        list_json("2026-11-01T00:00:00Z", "2026-11-10T00:00:00Z", &[], &[]),
    )));
    Revocations::new(
        f.state(),
        vec![f.root.public()],
        f.list.source(),
        "2026-11-05T00:00:00Z",
    )
    .unwrap()
    .refresh(&client())
    .unwrap();
    f.write_lock(&[oci_entry("fake", "code", "1.0.0", "r.example", OTHER_HASH)]);
    let ran = f.run(&["trust", "fake"], false, with_loader(&f));
    assert_eq!(ran.result.unwrap(), 0);
    assert!(
        ran.out.contains("notice[revocation-list-stale]:"),
        "{}",
        ran.out
    );
}

#[test]
fn a_malformed_revocation_cache_shows_no_parser_text_in_the_chain() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("style", "data", "1.2.0", "r.example", OTHER_HASH)]);
    fs::write(
        f.revocations().cache_file(),
        r#"{"SENSITIVE_MARKER": 1, "schema_version": "SENSITIVE_MARKER"}"#,
    )
    .unwrap();
    let err = f.run(&["status"], false, |s| s).result.unwrap_err();
    assert_eq!(err.code(), "revocation-cache-invalid");
    let shown = names_in_chain(&err);
    assert!(!shown.contains("SENSITIVE_MARKER"), "{shown}");
}

#[test]
fn a_malformed_trust_store_shows_no_parser_text_in_the_chain() {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("fake", "code", "1.0.0", "r.example", OTHER_HASH)]);
    fs::create_dir_all(f.state()).unwrap();
    fs::write(
        f.state().join("trust.json"),
        r#"{"SENSITIVE_MARKER": 1, "version": "SENSITIVE_MARKER"}"#,
    )
    .unwrap();
    let err = f
        .run(&["untrust", "fake"], false, |s| s)
        .result
        .unwrap_err();
    let shown = names_in_chain(&err);
    assert!(!shown.contains("SENSITIVE_MARKER"), "{shown}");
}

// ---- round 4d: every withheld sentence shows the code of its failure ------------------------

const WITHHELD: &str = "details are withheld because they can name paths outside the project";

/// The manifest of a managed Python plugin.
const PY_MANIFEST: &str = "schema_version = 3\nid = \"fake\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"code\"\n\n[plugin]\ncommand = \"plugin.py\"\nruntime = \"python\"\npython = \"3.12\"\nnetwork = false\nhooks = []\nfeatures = { required = [], optional = [] }\n";

/// What the user sees of a run: the failure, or the report.
fn seen(ran: &Ran) -> String {
    match &ran.result {
        Ok(_) => ran.out.clone(),
        Err(e) => e.to_string(),
    }
}

fn code_fixture() -> Fixture {
    let f = Fixture::new();
    f.write_lock(&[oci_entry("fake", "code", "1.0.0", "r.example", OTHER_HASH)]);
    f
}

fn broken_python_manifest(f: &Fixture) {
    let loader = TreeLoader::new(f.base.path(), "fake");
    fs::write(loader.root.join("slopvac-plugin.toml"), "not = [toml").unwrap();
}

fn corrupt_store(f: &Fixture) {
    fs::create_dir_all(f.state()).unwrap();
    fs::write(f.state().join("trust.json"), "not json").unwrap();
}

/// The arguments of `command`: `trust` also names the plugin.
fn command_args(command: &str) -> Vec<&str> {
    if command == "trust" {
        vec![command, "fake"]
    } else {
        vec![command]
    }
}

fn run_with_missing_env(f: &Fixture, command: &str) -> String {
    let loader = TreeLoader::new(f.base.path(), "fake");
    fs::write(loader.root.join("slopvac-plugin.toml"), PY_MANIFEST).unwrap();
    let envs = EnvStore::open(f.base.path().join("envs"));
    seen(&f.run(&command_args(command), false, |s| {
        s.loader(loader).envs(&envs)
    }))
}

fn run_with_program_outside(f: &Fixture, command: &str) -> String {
    let loader = OutsideProgram {
        inner: TreeLoader::new(f.base.path(), "fake"),
        program: f.base.path().join("elsewhere/prog"),
    };
    seen(&f.run(&command_args(command), false, |s| s.loader(loader)))
}

/// A site that renders the withheld sentence: how to reach it through `run()`, and the code
/// that must stand beside the sentence.
struct Site {
    name: &'static str,
    code: &'static str,
    reach: fn() -> String,
}

fn python_and_loader_sites() -> Vec<Site> {
    vec![
        Site {
            name: "python manifest, trust",
            code: "plugins-trust-inputs-failed",
            reach: || {
                let f = code_fixture();
                broken_python_manifest(&f);
                seen(&f.run(&["trust", "fake"], false, with_loader(&f)))
            },
        },
        Site {
            name: "python manifest, status",
            code: "plugins-part-trust-unknown",
            reach: || {
                let f = code_fixture();
                broken_python_manifest(&f);
                seen(&f.run(&["status"], false, with_loader(&f)))
            },
        },
        Site {
            name: "python environment, trust",
            code: "plugins-trust-inputs-failed",
            reach: || run_with_missing_env(&code_fixture(), "trust"),
        },
        Site {
            name: "python environment, status",
            code: "plugins-part-trust-unknown",
            reach: || run_with_missing_env(&code_fixture(), "status"),
        },
        Site {
            name: "loader, status",
            code: "plugins-part-trust-unknown",
            reach: || seen(&code_fixture().run(&["status"], false, |s| s.loader(FailingLoader))),
        },
        Site {
            name: "loader, trust",
            code: "plugins-trust-inputs-failed",
            reach: || {
                seen(&code_fixture().run(&["trust", "fake"], false, |s| s.loader(FailingLoader)))
            },
        },
    ]
}

fn store_sites() -> Vec<Site> {
    vec![
        Site {
            name: "damaged store, status",
            code: "plugins-store-failed",
            reach: || {
                let f = code_fixture();
                corrupt_store(&f);
                seen(&f.run(&["status"], false, with_loader(&f)))
            },
        },
        Site {
            name: "damaged store, trust",
            code: "plugins-store-failed",
            reach: || {
                let f = code_fixture();
                corrupt_store(&f);
                seen(&f.run(&["trust", "fake"], false, with_loader(&f)))
            },
        },
        Site {
            name: "damaged store, untrust",
            code: "plugins-store-failed",
            reach: || {
                let f = code_fixture();
                corrupt_store(&f);
                seen(&f.run(&["untrust", "fake"], false, |s| s))
            },
        },
        Site {
            name: "refused program, trust",
            code: "plugin-usage",
            reach: || run_with_program_outside(&code_fixture(), "trust"),
        },
        Site {
            name: "refused program, status",
            code: "plugin-usage",
            reach: || run_with_program_outside(&code_fixture(), "status"),
        },
    ]
}

fn library_sites() -> Vec<Site> {
    vec![
        Site {
            name: "revocation list unavailable, sync",
            code: "revocation-list-unavailable",
            reach: || {
                let f = Fixture::new();
                let digest = in_place_project(&f);
                f.write_lock(&[path_entry("style", "1.0.0", "plugins/style", &digest)]);
                f.list.serve(None);
                seen(&f.run(&["sync"], true, |s| s))
            },
        },
        Site {
            name: "sync source, sync",
            code: "sync-failed",
            reach: || {
                let f = Fixture::new();
                f.write_lock(&[path_entry("style", "1.0.0", "plugins/missing", OTHER_HASH)]);
                seen(&f.run(&["sync"], false, |s| s))
            },
        },
        Site {
            name: "update lock, update",
            code: "update-failed",
            reach: || {
                let f = Fixture::new();
                f.write_lock(&["not a lock".to_owned()]);
                seen(&f.run(&["update"], true, |s| s))
            },
        },
    ]
}

fn sites() -> Vec<Site> {
    [python_and_loader_sites(), store_sites(), library_sites()]
        .into_iter()
        .flatten()
        .collect()
}

#[test]
fn every_withheld_sentence_shows_the_code_of_its_failure() {
    for site in sites() {
        let shown = (site.reach)();
        let expected = format!("{}: {WITHHELD}", site.code);
        assert!(
            shown.contains(&expected),
            "{}: expected `{expected}` in `{shown}`",
            site.name
        );
    }
}

// ---- sync of a signed native archive -------------------------------------------------------

#[cfg(unix)]
mod native_archive {
    use std::collections::{HashMap, HashSet};
    use std::io::{Cursor, Write};
    use std::os::unix::fs::PermissionsExt;

    use flate2::Compression;
    use flate2::write::GzEncoder;
    use slopvac_plugins::oci::{Descriptor, Digest, MEDIA_MANIFEST, Manifest};
    use slopvac_plugins::sign::{
        ANNOTATION_ID, ANNOTATION_VERSION, MEDIA_MINISIG, MEDIA_STATEMENT, PublicKey, Statement,
    };
    use slopvac_plugins::sync::{ARTIFACT_TYPE_PACK, MEDIA_PACK_LAYER};

    use super::*;

    const FLOOR: &str = "2026-11-01T00:00:00Z";

    /// A signed `acme/style:1.0.0` artifact with one program in it, ready to serve.
    struct Artifact {
        manifest: Vec<u8>,
        digest: String,
        key_id: String,
        blobs: HashMap<String, Vec<u8>>,
        public: PublicKey,
    }

    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, data) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            builder.append_data(&mut header, name, *data).unwrap();
        }
        let raw = builder.into_inner().unwrap();
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&raw).unwrap();
        gz.finish().unwrap()
    }

    fn artifact() -> Artifact {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let public = PublicKey::from_base64(&pair.pk.to_base64()).unwrap();
        let layer = archive(&[("plugin", b"#!/bin/sh\n")]);
        let mut manifest = Manifest {
            schema_version: 2,
            media_type: Some(MEDIA_MANIFEST.into()),
            artifact_type: Some(ARTIFACT_TYPE_PACK.into()),
            config: Descriptor::for_bytes("application/vnd.oci.empty.v1+json", b"{}"),
            layers: vec![Descriptor::for_bytes(MEDIA_PACK_LAYER, &layer)],
            subject: None,
            annotations: BTreeMap::from([
                (ANNOTATION_ID.to_owned(), "style".to_owned()),
                (ANNOTATION_VERSION.to_owned(), "1.0.0".to_owned()),
            ]),
        };
        let key_id = public.key_id();
        let statement =
            Statement::for_manifest("acme", "style", "1.0.0", &key_id, FLOOR, &manifest)
                .canonical_bytes();
        let minisig = minisign::sign(
            Some(&pair.pk),
            &pair.sk,
            Cursor::new(&statement),
            None,
            None,
        )
        .unwrap()
        .to_string();
        manifest
            .layers
            .push(Descriptor::for_bytes(MEDIA_STATEMENT, &statement));
        manifest
            .layers
            .push(Descriptor::for_bytes(MEDIA_MINISIG, minisig.as_bytes()));
        let body = serde_json::to_vec(&manifest).unwrap();
        let mut blobs = HashMap::new();
        blobs.insert(Digest::of(&layer).to_string(), layer);
        blobs.insert(Digest::of(&statement).to_string(), statement);
        blobs.insert(
            Digest::of(minisig.as_bytes()).to_string(),
            minisig.into_bytes(),
        );
        blobs.insert(Digest::of(b"{}").to_string(), b"{}".to_vec());
        Artifact {
            digest: Digest::of(&body).to_string(),
            manifest: body,
            key_id,
            blobs,
            public,
        }
    }

    fn serve(a: &Artifact) -> Server {
        let (manifest, digest, blobs) = (a.manifest.clone(), a.digest.clone(), a.blobs.clone());
        Server::start(move |req| {
            let Some(rest) = req.path.strip_prefix("/v2/acme/style/") else {
                return Response::new(404);
            };
            if let Some(reference) = rest.strip_prefix("manifests/") {
                return if reference == "1.0.0" || reference == digest {
                    Response::new(200)
                        .header("Content-Type", MEDIA_MANIFEST)
                        .body(manifest.clone())
                } else {
                    Response::new(404)
                };
            }
            let blob = rest.rsplit('/').next().unwrap_or_default();
            blobs.get(blob).map_or_else(
                || Response::new(404),
                |b| Response::new(200).body(b.clone()),
            )
        })
    }

    #[test]
    fn sync_installs_a_signed_native_archive_with_its_program_executable() {
        let f = Fixture::new();
        let a = artifact();
        let server = serve(&a);
        f.write_lock(&[format!(
            "\n[[plugins]]\nid = \"style\"\nkind = \"code\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
             source = \"oci://{}/acme/style:1.0.0\"\ndigest = \"{}\"\n\n\
             [plugins.signature]\nkey_id = \"{}\"\nnot_before = \"{FLOOR}\"\n",
            server.registry(),
            a.digest,
            a.key_id
        )]);
        let policy = KeyPolicy {
            pinned: BTreeMap::from([("acme".to_owned(), vec![a.public.clone()])]),
            ..KeyPolicy::default()
        };
        let executables = BTreeMap::from([(
            "style".parse::<PackId>().unwrap(),
            HashSet::from(["plugin".to_owned()]),
        )]);
        let ran = f.run(&["sync"], true, |s| {
            s.policy(policy).executables(executables)
        });
        assert_eq!(ran.result.unwrap(), 0, "{}", ran.out);
        assert!(
            ran.out.starts_with("style 1.0.0: installed\n"),
            "{}",
            ran.out
        );
        let tree = fs::read_dir(f.cache().join("trees"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.join("plugin").exists())
            .expect("an installed tree holds the program");
        let mode = fs::metadata(tree.join("plugin"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "mode {mode:o}");
    }
}
