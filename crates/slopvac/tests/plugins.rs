//! The `slopvac plugins` group through the installed binary: the command tree, exit codes and
//! the codes a failure reports (policies.md, 3.5).
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

/// A project directory and per-user directories that belong to one test.
struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("project/.slopvac")).unwrap();
        Self { dir }
    }

    fn project(&self) -> std::path::PathBuf {
        self.dir.path().join("project")
    }

    fn write_lock(&self, entries: &str) {
        fs::write(
            self.project().join(".slopvac/plugins.lock"),
            format!("schema_version = 3\n{entries}"),
        )
        .unwrap();
    }

    fn slopvac(&self) -> Command {
        let home = self.dir.path().join("home");
        let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
        cmd.current_dir(self.project())
            .env("SLOPVAC_NO_UPDATE_CHECK", "1")
            .env("HOME", &home)
            .env("XDG_CACHE_HOME", home.join("cache"))
            .env("XDG_STATE_HOME", home.join("state"))
            .env("NO_COLOR", "1");
        for var in [
            "SLOPVAC_OFFLINE",
            "SLOPVAC_LOG",
            "SLOPVAC_CACHE_DIR",
            "SLOPVAC_TRUST",
            "FORCE_COLOR",
            "CLICOLOR_FORCE",
        ] {
            cmd.env_remove(var);
        }
        cmd
    }
}

#[test]
fn plugins_help_lists_the_group_subcommands() {
    let sandbox = Sandbox::new();
    let assert = sandbox
        .slopvac()
        .args(["plugins", "--help"])
        .assert()
        .success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    for name in [
        "list", "search", "sync", "status", "outdated", "update", "trust", "untrust", "pin",
        "rotate", "new", "validate", "test", "build", "publish",
    ] {
        assert!(
            out.lines().any(|l| l.trim_start().starts_with(name)),
            "{name} missing:\n{out}"
        );
    }
}

#[test]
fn packs_is_no_longer_a_command() {
    let sandbox = Sandbox::new();
    sandbox
        .slopvac()
        .args(["packs"])
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[usage]: "));
}

#[test]
fn status_in_a_project_with_no_lockfile_passes() {
    let sandbox = Sandbox::new();
    sandbox
        .slopvac()
        .args(["plugins", "status"])
        .assert()
        .success();
}

#[test]
fn sync_exits_0_when_the_lock_is_in_place() {
    let sandbox = Sandbox::new();
    let plugin = sandbox.project().join("plugins/style");
    fs::create_dir_all(&plugin).unwrap();
    fs::write(plugin.join("rules.yml"), "a: b\n").unwrap();
    let digest = slopvac_plugins::extract::tree_hash(&plugin).unwrap();
    sandbox.write_lock(&format!(
        "\n[[plugins]]\nid = \"style\"\nkind = \"data\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
         source = \"plugins/style\"\ndigest = \"{digest}\"\n"
    ));
    sandbox
        .slopvac()
        .args(["--offline", "plugins", "sync"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("style 1.0.0: in place\n"));
}

#[test]
fn sync_offline_with_an_uncached_plugin_exits_2_with_its_own_code() {
    let sandbox = Sandbox::new();
    sandbox.write_lock(
        "\n[[plugins]]\nid = \"style\"\nkind = \"data\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
         source = \"oci://registry.invalid/acme/style:1.0.0\"\n\
         digest = \"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"\n",
    );
    sandbox
        .slopvac()
        .args(["--offline", "plugins", "sync"])
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with(
            "error[offline-plugin-missing]: ",
        ));
}

#[rstest::rstest]
#[case::search(&["search", "style"])]
#[case::pin(&["pin", "acme", "abc"])]
#[case::rotate(&["rotate", "acme"])]
#[case::new(&["new", "style"])]
#[case::publish(&["publish"])]
fn a_command_not_wired_yet_reports_its_code_and_exits_2(#[case] args: &[&str]) {
    let sandbox = Sandbox::new();
    sandbox
        .slopvac()
        .arg("plugins")
        .args(args)
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[command-not-wired]: "));
}

/// Pins today's behaviour: the binary gives `plugins trust` no loader for code plugin files,
/// so trust is refused with its own code. Change this test when the loader is wired.
#[test]
fn trust_of_a_locked_code_plugin_exits_2_because_no_loader_is_wired() {
    let sandbox = Sandbox::new();
    sandbox.write_lock(
        "\n[[plugins]]\nid = \"fmt\"\nkind = \"code\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
         source = \"oci://registry.invalid/acme/fmt:1.0.0\"\n\
         digest = \"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"\n",
    );
    sandbox
        .slopvac()
        .args(["--offline", "plugins", "trust", "fmt"])
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with(
            "error[plugins-trust-inputs-failed]: ",
        ));
}
