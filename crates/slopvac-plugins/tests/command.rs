//! The `slopvac plugins` command group through its public `command()` and `run()`.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use std::collections::BTreeSet;
use std::fs;

mod common;

use slopvac_messages::Catalog;
use slopvac_plugins::trust::{DeclSummary, TrustRecord, TrustStore};
use slopvac_plugins::{Dirs, RunError, command, run};

const LOCKED_OCI: &str = "schema_version = 3

[[plugins]]
id = \"acme-docs\"
kind = \"data\"
version = \"1.2.0\"
slopvac_api = 1
source = \"oci://ghcr.io/acme/slopvac-docs:1.2.0\"
digest = \"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"
";

const HASH: &str = "sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
const OTHER_HASH: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

struct Project {
    dir: tempfile::TempDir,
}

impl Project {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn with_lock(text: &str) -> Self {
        let p = Self::new();
        fs::create_dir_all(p.dir.path().join(".slopvac")).unwrap();
        fs::write(p.dir.path().join(".slopvac/plugins.lock"), text).unwrap();
        p
    }

    fn state(&self) -> std::path::PathBuf {
        self.dir.path().join("state")
    }

    /// Run `slopvac plugins <args>`: the result and what it printed.
    fn run(&self, args: &[&str]) -> (Result<u8, RunError>, String) {
        let dirs = Dirs::new(
            self.dir.path(),
            self.dir.path().join("cache"),
            self.dir.path().join("state"),
        );
        let words = std::iter::once("plugins").chain(args.iter().copied());
        let matches = command().try_get_matches_from(words).unwrap();
        let mut out = Vec::new();
        let result = run(&matches, &dirs, &mut out);
        (result, String::from_utf8(out).unwrap())
    }
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
        recorded_at: "2026-11-22T00:00:00Z".into(),
    }
}

#[test]
fn the_group_lists_the_contract_subcommands() {
    let names: BTreeSet<String> = command()
        .get_subcommands()
        .map(|c| c.get_name().to_owned())
        .collect();
    let expected: BTreeSet<String> = [
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

#[test]
fn status_with_no_lock_reports_nothing_and_passes() {
    let p = Project::new();
    let (result, out) = p.run(&["status"]);
    assert_eq!(result.unwrap(), 0);
    assert_eq!(out, "");
}

#[test]
fn status_reports_a_locked_plugin_that_is_not_installed_and_exits_2() {
    let p = Project::with_lock(LOCKED_OCI);
    let (result, out) = p.run(&["status"]);
    assert_eq!(result.unwrap(), 2);
    assert_eq!(
        out,
        "acme-docs 1.2.0: not installed; run `slopvac plugins sync`\n"
    );
}

#[test]
fn status_names_an_unreadable_lock_with_its_code() {
    let p = Project::with_lock("not a lock");
    let err = p.run(&["status"]).0.unwrap_err();
    assert_eq!(err.code(), "plugins-lock-unreadable");
    assert_eq!(err.exit_code(), 2);
}

#[test]
fn untrust_removes_the_trust_recorded_for_a_plugin() {
    let p = Project::new();
    let store = TrustStore::open(p.state());
    store.record(record("fake", HASH)).unwrap();
    store.record(record("other", OTHER_HASH)).unwrap();
    let (result, out) = p.run(&["untrust", "fake"]);
    assert_eq!(result.unwrap(), 0);
    assert_eq!(out, "removed the trust recorded for `fake`\n");
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
    let p = Project::new();
    let err = p.run(&["untrust", "fake"]).0.unwrap_err();
    assert_eq!(err.code(), "plugins-untrust-not-trusted");
    assert_eq!(err.exit_code(), 2);
}

#[rstest::rstest]
#[case::list(&["list"], "list")]
#[case::search(&["search", "style"], "search")]
#[case::sync(&["sync"], "sync")]
#[case::outdated(&["outdated"], "outdated")]
#[case::update(&["update"], "update")]
#[case::trust(&["trust", "fake"], "trust")]
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
    let p = Project::new();
    let (result, out) = p.run(args);
    let err = result.unwrap_err();
    assert_eq!(err.code(), "plugins-command-unavailable");
    assert_eq!(err.exit_code(), 2);
    assert_eq!(
        err.to_string(),
        format!("`slopvac plugins {name}` is not available in this build yet.")
    );
    assert_eq!(out, "");
}
