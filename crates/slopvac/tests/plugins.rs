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
            "CI",
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
fn sync_of_an_unsigned_oci_plugin_exits_2_with_its_own_code() {
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
        .stderr(predicate::str::starts_with("error[sync-unsigned]: "));
}

/// A lock entry with a signature record, for an OCI plugin that no cache holds.
const SIGNED_UNCACHED: &str = "\n[[plugins]]\nid = \"style\"\nkind = \"data\"\nversion = \"1.0.0\"\n\
    slopvac_api = 1\nsource = \"oci://registry.invalid/acme/style:1.0.0\"\n\
    digest = \"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"\n\
    [plugins.signature]\nkey_id = \"A1B2C3D4E5F60718\"\nnot_before = \"2026-11-01T00:00:00Z\"\n";

/// Offline mode from any source stops the run before the registry: the cache has no copy, so
/// the failure is the offline one. A run that went online would fail another way.
#[rstest::rstest]
#[case::flag(&["--offline", "plugins", "sync"], &[], None)]
#[case::environment(&["plugins", "sync"], &[("SLOPVAC_OFFLINE", "1")], None)]
#[case::project_config(
    &["plugins", "sync"], &[], Some(("slopvac.toml", "schema_version = 3\n[network]\noffline = true\n"))
)]
#[case::extended_config(
    &["plugins", "sync"], &[], Some(("slopvac.toml", "schema_version = 3\nextends = [\"base.toml\"]\n"))
)]
fn sync_offline_from_any_source_reports_the_missing_cache_entry(
    #[case] args: &[&str],
    #[case] vars: &[(&str, &str)],
    #[case] config: Option<(&str, &str)>,
) {
    let sandbox = Sandbox::new();
    sandbox.write_lock(SIGNED_UNCACHED);
    if let Some((name, text)) = config {
        fs::write(sandbox.project().join(name), text).unwrap();
        fs::write(
            sandbox.project().join("base.toml"),
            "schema_version = 3\n[network]\noffline = true\n",
        )
        .unwrap();
    }
    sandbox
        .slopvac()
        .envs(vars.iter().copied())
        .args(args)
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with(
            "error[offline-plugin-missing]: ",
        ));
}

/// A range that does not parse must stop the run and name the plugin, never the value.
#[test]
fn an_invalid_version_range_in_the_config_exits_2_and_names_the_plugin() {
    let sandbox = Sandbox::new();
    fs::write(
        sandbox.project().join("slopvac.toml"),
        "schema_version = 3\n[[plugins]]\nid = \"style\"\nsource = \"oci://registry.invalid/acme/style\"\n\
         version = \"bogus-range\"\n",
    )
    .unwrap();
    sandbox
        .slopvac()
        .args(["--offline", "plugins", "list"])
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[config-invalid-value]: "))
        .stderr(predicate::str::contains("plugin `style`"))
        .stderr(predicate::str::contains("bogus-range").not());
}

#[test]
fn sync_without_a_lock_passes_outside_ci() {
    let sandbox = Sandbox::new();
    sandbox
        .slopvac()
        .args(["--offline", "plugins", "sync"])
        .assert()
        .success();
}

#[test]
fn sync_in_ci_without_a_lock_exits_2() {
    let sandbox = Sandbox::new();
    sandbox
        .slopvac()
        .env("CI", "true")
        .args(["--offline", "plugins", "sync"])
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with(
            "error[plugins-lock-unreadable]: ",
        ));
}

/// The plugin host starts this executable as the plugin launcher. The binary has to run that
/// mode, or no git plugin source can be fetched.
#[test]
fn the_binary_runs_as_the_plugin_launcher() {
    let sandbox = Sandbox::new();
    sandbox
        .slopvac()
        .args([
            "--slopvac-plugin-launcher",
            "cpu=60",
            "nofile=256",
            "fsize=1048576",
            "data=4294967296",
            "sweep=100",
            "--",
            "/bin/echo",
            "launched",
        ])
        .assert()
        .success()
        .stdout("launched\n");
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
