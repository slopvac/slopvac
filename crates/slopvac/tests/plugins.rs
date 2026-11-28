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
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_DATA_HOME", home.join("data"))
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

mod conformance {
    use super::*;
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

    /// Conformance: N4
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

    /// The plugin store is the per-user data directory (doctor.md, "Directories"), never the cache.
    #[test]
    fn sync_writes_the_plugin_store_to_the_data_directory_and_not_the_cache() {
        let sandbox = Sandbox::new();
        sandbox
            .slopvac()
            .args(["--offline", "plugins", "sync"])
            .assert()
            .success();
        let home = sandbox.dir.path().join("home");
        assert!(
            home.join("data/slopvac/trees").is_dir(),
            "no store in the data directory"
        );
        assert!(
            !home.join("cache/slopvac").exists(),
            "the cache directory holds a store"
        );
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
    ///
    /// Conformance: N2
    #[rstest::rstest]
    #[case::flag(&["--offline", "plugins", "sync"], &[], None)]
    #[case::environment(&["plugins", "sync"], &[("SLOPVAC_OFFLINE", "1")], None)]
    #[case::project_config(
        &["plugins", "sync"], &[], Some(("slopvac.toml", "schema_version = 3\n[network]\noffline = true\n"))
    )]
    #[case::extended_config(
        &["plugins", "sync"], &[], Some(("slopvac.toml", "schema_version = 3\nextends = [\"base.toml\"]\n"))
    )]
    #[case::extended_true_then_project_false(
        &["plugins", "sync"], &[], Some((
            "slopvac.toml",
            "schema_version = 3\nextends = [\"base.toml\"]\n[network]\noffline = false\n"
        ))
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

    /// The network boundary of a run: a local proxy that counts every connection the binary opens.
    /// The registry and revocation clients read the proxy settings of the environment, so any
    /// request, the revocation fetch included, reaches it. It answers nothing.
    struct NetworkTally {
        port: u16,
        seen: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
    }

    impl NetworkTally {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            listener.set_nonblocking(true).unwrap();
            let port = listener.local_addr().unwrap().port();
            let (seen, stop) = (
                Arc::new(AtomicUsize::new(0)),
                Arc::new(AtomicBool::new(false)),
            );
            let (count, done) = (Arc::clone(&seen), Arc::clone(&stop));
            std::thread::spawn(move || {
                while !done.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok(_) => {
                            count.fetch_add(1, Ordering::AcqRel);
                        }
                        Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
                    }
                }
            });
            Self { port, seen, stop }
        }

        fn requests(&self) -> usize {
            self.seen.load(Ordering::Acquire)
        }

        /// Route every request of `cmd` through this proxy.
        fn route(&self, cmd: &mut Command) {
            let url = format!("http://127.0.0.1:{}", self.port);
            for var in [
                "HTTP_PROXY",
                "HTTPS_PROXY",
                "ALL_PROXY",
                "http_proxy",
                "https_proxy",
                "all_proxy",
            ] {
                cmd.env(var, &url);
            }
            for var in ["NO_PROXY", "no_proxy"] {
                cmd.env_remove(var);
            }
        }
    }

    impl Drop for NetworkTally {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
        }
    }

    /// Sticky offline: the extended file turns it on, the project turns it off, and the run opens no
    /// connection at all, the revocation fetch included.
    #[test]
    fn sticky_offline_opens_no_connection_when_a_later_layer_says_false() {
        let sandbox = Sandbox::new();
        sandbox.write_lock(SIGNED_UNCACHED);
        fs::write(
            sandbox.project().join("slopvac.toml"),
            "schema_version = 3\nextends = [\"base.toml\"]\n[network]\noffline = false\n",
        )
        .unwrap();
        fs::write(
            sandbox.project().join("base.toml"),
            "schema_version = 3\n[network]\noffline = true\n",
        )
        .unwrap();
        let tally = NetworkTally::start();
        let mut cmd = sandbox.slopvac();
        tally.route(&mut cmd);
        cmd.args(["plugins", "sync"])
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with(
                "error[offline-plugin-missing]: ",
            ));
        assert_eq!(tally.requests(), 0, "an offline run opened a connection");
    }

    /// `SLOPVAC_OFFLINE=1` in the environment with `[network] offline = false` in the config stays
    /// offline: the sync stops at the missing cache entry and never reaches the registry.
    ///
    /// Conformance: N3
    #[test]
    fn the_environment_stays_offline_when_the_config_says_false() {
        let sandbox = Sandbox::new();
        sandbox.write_lock(SIGNED_UNCACHED);
        fs::write(
            sandbox.project().join("slopvac.toml"),
            "schema_version = 3\n[network]\noffline = false\n",
        )
        .unwrap();
        sandbox
            .slopvac()
            .env("SLOPVAC_OFFLINE", "1")
            .args(["plugins", "sync"])
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with(
                "error[offline-plugin-missing]: ",
            ));
    }

    /// The control for the test above: an online run does reach the tally, so a count of zero
    /// there means the run stayed off the network.
    #[test]
    fn an_online_sync_reaches_the_network_tally() {
        let sandbox = Sandbox::new();
        sandbox.write_lock(SIGNED_UNCACHED);
        let tally = NetworkTally::start();
        let mut cmd = sandbox.slopvac();
        tally.route(&mut cmd);
        cmd.args(["plugins", "sync"]).assert().code(2);
        assert!(
            tally.requests() > 0,
            "the tally saw no connection from an online run"
        );
    }

    /// A config id longer than the limit passes the config check but is not a valid plugin id. It
    /// stops the run, and the message names the entry and never the value. (The config check itself
    /// refuses an id with bad characters.) The code is `config-invalid-value`, as contract ruled.
    #[test]
    fn a_too_long_plugin_id_in_the_config_exits_2_and_names_no_value() {
        let id = "a".repeat(5000);
        let sandbox = Sandbox::new();
        sandbox.write_lock(SIGNED_UNCACHED);
        fs::write(
            sandbox.project().join("slopvac.toml"),
            format!(
                "schema_version = 3\n[[plugins]]\nid = \"{id}\"\nsource = \"oci://registry.invalid/acme/x\"\n"
            ),
        )
        .unwrap();
        sandbox
            .slopvac()
            .env("CI", "true")
            .args(["--offline", "plugins", "sync"])
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with("error[config-invalid-value]: "))
            .stderr(predicate::str::contains("plugin entry 1"))
            .stderr(predicate::str::contains(&id).not());
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

    /// Conformance: N7
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

    const STYLE_CONFIG: &str = "schema_version = 3\n[[plugins]]\nid = \"style\"\n\
        source = \"oci://registry.invalid/acme/style\"\n";

    #[test]
    fn sync_in_ci_exits_2_when_a_configured_plugin_is_not_in_the_lock() {
        let sandbox = Sandbox::new();
        // A lock needs at least one entry to be valid, so it locks another plugin that the config
        // also drops. The `unlocked` line is the one under test.
        sandbox.write_lock(&SIGNED_UNCACHED.replace("\"style\"", "\"other\""));
        fs::write(sandbox.project().join("slopvac.toml"), STYLE_CONFIG).unwrap();
        sandbox
            .slopvac()
            .env("CI", "true")
            .args(["--offline", "plugins", "sync"])
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with("error[lock-mismatch]: "))
            .stderr(predicate::str::contains(
                "the lock does not match the configured plugins:",
            ))
            .stderr(predicate::str::contains("unlocked: style"));
    }

    #[test]
    fn sync_in_ci_exits_2_when_a_locked_plugin_is_no_longer_configured() {
        let sandbox = Sandbox::new();
        sandbox.write_lock(SIGNED_UNCACHED);
        fs::write(
            sandbox.project().join("slopvac.toml"),
            "schema_version = 3\n",
        )
        .unwrap();
        sandbox
            .slopvac()
            .env("CI", "true")
            .args(["--offline", "plugins", "sync"])
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with("error[lock-mismatch]: "))
            .stderr(predicate::str::contains("removed: style"));
    }

    /// The lock bytes, to show that a refused CI sync leaves the file as it found it.
    fn lock_bytes(sandbox: &Sandbox) -> Vec<u8> {
        fs::read(sandbox.project().join(".slopvac/plugins.lock")).unwrap()
    }

    /// Writes each `(path, text)` under the project. A path may climb out of it with `..`.
    fn write_files(sandbox: &Sandbox, files: &[(&str, &str)]) {
        for (name, text) in files {
            let path = sandbox.project().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
    }

    /// A config entry for `style` with `source` and an optional version requirement.
    fn style_entry(source: &str, version: Option<&str>) -> String {
        let version = version.map_or_else(String::new, |v| format!("version = \"{v}\"\n"));
        format!("schema_version = 3\n[[plugins]]\nid = \"style\"\nsource = \"{source}\"\n{version}")
    }

    /// In CI, a configured source that names another source than the lock, or a locked version
    /// that the configured requirement does not allow, fails before any fetch. The line names the
    /// plugin, and the lockfile is unchanged.
    ///
    /// Conformance: N7
    #[rstest::rstest]
    #[case::other_repository(
        style_entry("oci://registry.invalid/acme/other", None),
        "source: style is configured with a different source from its lock entry"
    )]
    #[case::other_registry_port(
        style_entry("oci://registry.invalid:5000/acme/style", None),
        "source: style is configured with a different source from its lock entry"
    )]
    #[case::path_against_oci(
        style_entry("plugins/style", None),
        "source: style is configured with a different source from its lock entry"
    )]
    #[case::out_of_range(
        style_entry("oci://registry.invalid/acme/style", Some("^2")),
        "out of range: style is locked at 1.0.0, which the configured requirement ^2 does not allow"
    )]
    fn sync_in_ci_exits_2_on_config_drift_and_leaves_the_lock(
        #[case] config: String,
        #[case] line: &str,
    ) {
        let sandbox = Sandbox::new();
        sandbox.write_lock(SIGNED_UNCACHED);
        fs::write(sandbox.project().join("slopvac.toml"), config).unwrap();
        let before = lock_bytes(&sandbox);
        sandbox
            .slopvac()
            .env("CI", "true")
            .args(["--offline", "plugins", "sync"])
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with("error[lock-mismatch]: "))
            .stderr(predicate::str::contains(line));
        assert_eq!(lock_bytes(&sandbox), before);
    }

    /// The documented config form names an OCI repository with no tag, and the lock pins the tag
    /// of the version. They match, so the run goes on to the cache, which here has no copy.
    #[rstest::rstest]
    #[case::untagged(style_entry("oci://registry.invalid/acme/style", None))]
    #[case::untagged_with_a_requirement(style_entry(
        "oci://registry.invalid/acme/style",
        Some("1.0.0")
    ))]
    #[case::other_tag(style_entry("oci://registry.invalid/acme/style:latest", None))]
    fn sync_in_ci_matches_an_untagged_oci_source_with_the_locked_tag(#[case] config: String) {
        let sandbox = Sandbox::new();
        sandbox.write_lock(SIGNED_UNCACHED);
        fs::write(sandbox.project().join("slopvac.toml"), config).unwrap();
        sandbox
            .slopvac()
            .env("CI", "true")
            .args(["--offline", "plugins", "sync"])
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with(
                "error[offline-plugin-missing]: ",
            ));
    }

    /// Outside CI, sync does not compare the config with the lock.
    #[test]
    fn sync_outside_ci_ignores_a_configured_source_that_differs() {
        let sandbox = Sandbox::new();
        sandbox.write_lock(SIGNED_UNCACHED);
        fs::write(
            sandbox.project().join("slopvac.toml"),
            style_entry("oci://registry.invalid/acme/other", Some("^2")),
        )
        .unwrap();
        sandbox
            .slopvac()
            .args(["--offline", "plugins", "sync"])
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with(
                "error[offline-plugin-missing]: ",
            ));
    }

    /// A project plugin at `plugins/style`, locked by its tree hash at that root-relative path.
    fn lock_path_plugin(sandbox: &Sandbox) {
        let plugin = sandbox.project().join("plugins/style");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("rules.yml"), "a: b\n").unwrap();
        let digest = slopvac_plugins::extract::tree_hash(&plugin).unwrap();
        sandbox.write_lock(&format!(
            "\n[[plugins]]\nid = \"style\"\nkind = \"data\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
             source = \"plugins/style\"\ndigest = \"{digest}\"\n"
        ));
    }

    /// A config path is relative to the file that declares it. Read that way, each of these names
    /// the locked `plugins/style`, so CI sync installs it in place.
    #[rstest::rstest]
    #[case::root_config(&[("slopvac.toml", "schema_version = 3\n[[plugins]]\nid = \"style\"\nsource = \"plugins/style\"\n")])]
    #[case::extends_from_a_subdirectory(&[
        ("slopvac.toml", "schema_version = 3\nextends = [\"config/base.toml\"]\n"),
        ("config/base.toml", "schema_version = 3\n[[plugins]]\nid = \"style\"\nsource = \"../plugins/style\"\n"),
    ])]
    #[case::extends_from_outside_the_project(&[
        ("slopvac.toml", "schema_version = 3\nextends = [\"../shared/base.toml\"]\n"),
        ("../shared/base.toml", "schema_version = 3\n[[plugins]]\nid = \"style\"\nsource = \"../project/plugins/style\"\n"),
    ])]
    fn sync_in_ci_resolves_a_path_source_against_its_config_file(#[case] files: &[(&str, &str)]) {
        let sandbox = Sandbox::new();
        lock_path_plugin(&sandbox);
        write_files(&sandbox, files);
        sandbox
            .slopvac()
            .env("CI", "true")
            .args(["--offline", "plugins", "sync"])
            .assert()
            .success()
            .stdout(predicate::str::starts_with("style 1.0.0: in place\n"));
    }

    /// A path that resolves to another directory than the locked one is drift.
    #[rstest::rstest]
    #[case::other_directory(&[("slopvac.toml", "schema_version = 3\n[[plugins]]\nid = \"style\"\nsource = \"plugins/other\"\n")])]
    #[case::root_path_in_a_subdirectory_file(&[
        ("slopvac.toml", "schema_version = 3\nextends = [\"config/base.toml\"]\n"),
        ("config/base.toml", "schema_version = 3\n[[plugins]]\nid = \"style\"\nsource = \"plugins/style\"\n"),
    ])]
    fn sync_in_ci_exits_2_when_a_path_source_resolves_elsewhere(#[case] files: &[(&str, &str)]) {
        let sandbox = Sandbox::new();
        lock_path_plugin(&sandbox);
        write_files(&sandbox, files);
        let before = lock_bytes(&sandbox);
        sandbox
            .slopvac()
            .env("CI", "true")
            .args(["--offline", "plugins", "sync"])
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with("error[lock-mismatch]: "))
            .stderr(predicate::str::contains(
                "source: style is configured with a different source from its lock entry",
            ));
        assert_eq!(lock_bytes(&sandbox), before);
    }

    /// A source that the lock would refuse makes the config invalid for every `plugins` command,
    /// in CI or not. The message names the plugin, never the value.
    #[rstest::rstest]
    #[case::sync_in_ci(&["--offline", "plugins", "sync"], true)]
    #[case::sync(&["--offline", "plugins", "sync"], false)]
    #[case::status(&["--offline", "plugins", "status"], false)]
    #[case::list(&["--offline", "plugins", "list"], false)]
    fn an_invalid_source_in_the_config_fails_every_plugins_command(
        #[case] args: &[&str],
        #[case] ci: bool,
    ) {
        let sandbox = Sandbox::new();
        sandbox.write_lock(SIGNED_UNCACHED);
        fs::write(
            sandbox.project().join("slopvac.toml"),
            style_entry("http://registry.invalid/acme/style.tar.gz", None),
        )
        .unwrap();
        let mut cmd = sandbox.slopvac();
        if ci {
            cmd.env("CI", "true");
        }
        cmd.args(args)
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with("error[config-invalid-value]: "))
            .stderr(predicate::str::contains("plugin `style`"))
            .stderr(predicate::str::contains("registry.invalid").not());
    }

    /// An `oci://` source with no repository, or with an empty tag, is malformed. The config may
    /// leave out the tag or digest, but not get one wrong.
    #[rstest::rstest]
    #[case::host_only("oci://registry.invalid")]
    #[case::empty_tag("oci://registry.invalid/acme/style:")]
    #[case::empty_tag_and_a_digest(
        "oci://registry.invalid/acme/style:@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    )]
    fn a_malformed_oci_source_in_the_config_is_invalid(#[case] source: &str) {
        let sandbox = Sandbox::new();
        sandbox.write_lock(SIGNED_UNCACHED);
        fs::write(
            sandbox.project().join("slopvac.toml"),
            style_entry(source, None),
        )
        .unwrap();
        sandbox
            .slopvac()
            .env("CI", "true")
            .args(["--offline", "plugins", "sync"])
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with("error[config-invalid-value]: "))
            .stderr(predicate::str::contains("plugin `style`"));
    }

    /// An online CI sync refuses drift before it opens any connection, the revocation fetch
    /// included, and leaves the lock as it found it. The control above shows that an online sync
    /// does reach the tally.
    #[test]
    fn an_online_sync_in_ci_refuses_drift_before_any_request() {
        let sandbox = Sandbox::new();
        sandbox.write_lock(SIGNED_UNCACHED);
        fs::write(
            sandbox.project().join("slopvac.toml"),
            style_entry("oci://registry.invalid/acme/other", None),
        )
        .unwrap();
        let before = lock_bytes(&sandbox);
        let tally = NetworkTally::start();
        let mut cmd = sandbox.slopvac();
        tally.route(&mut cmd);
        cmd.env("CI", "true")
            .args(["plugins", "sync"])
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with("error[lock-mismatch]: "));
        assert_eq!(tally.requests(), 0, "a refused sync opened a connection");
        assert_eq!(lock_bytes(&sandbox), before);
    }

    /// A configured source can carry a credential, as user-info (an invalid source) or in a query
    /// (a valid one that differs from the lock). Neither output stream repeats it.
    #[rstest::rstest]
    #[case::user_info(
        "https://reader:hunter2-secret@registry.invalid/style.tar.gz",
        "error[config-invalid-value]: "
    )]
    #[case::query_token(
        "https://registry.invalid/style.tar.gz?token=hunter2-secret",
        "error[lock-mismatch]: "
    )]
    fn sync_in_ci_never_prints_a_configured_credential(#[case] source: &str, #[case] start: &str) {
        let sandbox = Sandbox::new();
        sandbox.write_lock(&SIGNED_UNCACHED.replace(
            "oci://registry.invalid/acme/style:1.0.0",
            "https://registry.invalid/style.tar.gz",
        ));
        fs::write(
            sandbox.project().join("slopvac.toml"),
            style_entry(source, None),
        )
        .unwrap();
        sandbox
            .slopvac()
            .env("CI", "true")
            .args(["--offline", "plugins", "sync"])
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with(start))
            .stderr(predicate::str::contains("hunter2-secret").not())
            .stdout(predicate::str::contains("hunter2-secret").not());
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

    /// A launcher call typed by hand with a part missing is a usage error, not a plugin failure.
    #[test]
    fn a_malformed_launcher_invocation_exits_2_with_the_usage_line() {
        let sandbox = Sandbox::new();
        sandbox
            .slopvac()
            .args(["--slopvac-plugin-launcher", "cpu=60"])
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with("slopvac plugin launcher: "))
            .stderr(predicate::str::contains("usage: slopvac-plugin-launcher"));
    }

    /// A valid launcher call hands back the status of the program it ran.
    #[test]
    fn a_valid_launcher_invocation_relays_the_exit_status_of_the_program() {
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
                "/bin/sh",
                "-c",
                "exit 3",
            ])
            .assert()
            .code(3);
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

    /// A code plugin in the project at `plugins/fmt`, locked by its tree hash.
    fn lock_code_plugin(sandbox: &Sandbox) {
        let plugin = sandbox.project().join("plugins/fmt");
        fs::create_dir_all(plugin.join("bin")).unwrap();
        fs::write(
            plugin.join("slopvac-plugin.toml"),
            "schema_version = 3\nid = \"fmt\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"code\"\n\
             [provides]\n[plugin]\ncommand = \"bin/fmt\"\nnetwork = false\nhooks = []\n\
             features = { required = [], optional = [] }\n",
        )
        .unwrap();
        fs::write(plugin.join("bin/fmt"), "#!/bin/sh\nexit 0\n").unwrap();
        let digest = slopvac_plugins::extract::tree_hash(&plugin).unwrap();
        sandbox.write_lock(&format!(
            "\n[[plugins]]\nid = \"fmt\"\nkind = \"code\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
             source = \"plugins/fmt\"\ndigest = \"{digest}\"\n"
        ));
    }

    /// The trust store that the sandbox's binary writes to.
    fn trust_records(sandbox: &Sandbox) -> usize {
        slopvac_plugins::trust::TrustStore::open(sandbox.dir.path().join("home/state/slopvac"))
            .records()
            .unwrap()
            .len()
    }

    /// The plugin store of the sandbox, which `plugins sync` fills.
    fn plugin_store(sandbox: &Sandbox) -> std::path::PathBuf {
        sandbox.dir.path().join("home/data/slopvac")
    }

    const PLUGIN_MANIFEST: &str = "schema_version = 3\nid = \"fmt\"\nversion = \"1.0.0\"\n\
        slopvac_api = 1\nkind = \"code\"\n[provides]\n[plugin]\ncommand = \"bin/fmt\"\n\
        args = [\"--serve\"]\nnetwork = false\nhooks = []\n\
        features = { required = [], optional = [] }\n";

    /// A `.tar.gz` of a code plugin with this manifest and an empty script.
    fn plugin_targz(manifest: &str) -> Vec<u8> {
        use std::io::Write as _;
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

    /// Extract `archive` into a new directory under `scratch` and return its tree hash and place.
    fn extract_plugin(
        archive: &[u8],
        store: &std::path::Path,
    ) -> slopvac_plugins::extract::Extracted {
        slopvac_plugins::extract::extract_targz(
            archive,
            store,
            &std::collections::HashSet::<String>::new(),
            slopvac_plugins::extract::Limits::default(),
        )
        .unwrap()
    }

    /// Move the extracted tree `done` to the place that sync keeps the tree `name`.
    fn move_tree_to(
        done: &slopvac_plugins::extract::Extracted,
        trees: &std::path::Path,
        name: &slopvac_plugins::extract::TreeHash,
    ) {
        use std::os::unix::fs::PermissionsExt;
        // A rename to another directory needs write access to the moved directory.
        fs::set_permissions(&done.root, fs::Permissions::from_mode(0o755)).unwrap();
        fs::rename(&done.root, trees.join(format!("tree-{}", name.hex()))).unwrap();
    }

    /// The debug event of a refused load, after its timestamp and level: the fixed message and
    /// exactly two fields, the stable `code` of the refusal and the plugin id `fmt`, once each. Any
    /// other text, such as a path or a refusal sentence, fails.
    fn assert_debug_line(stderr: &str, code: &str) {
        let events: Vec<&str> = stderr
            .lines()
            .filter(|l| l.contains("plugin loader refused"))
            .collect();
        assert_eq!(
            events.len(),
            1,
            "one `plugin loader refused` event:\n{stderr}"
        );
        let split = events[0].split_once(" DEBUG ");
        assert!(split.is_some(), "no DEBUG level: {:?}", events[0]);
        let (timestamp, event) = split.unwrap();
        assert!(!timestamp.contains(' '), "{:?}", events[0]);
        assert_eq!(
            event.trim_start(),
            format!("plugin loader refused code=\"{code}\" plugin=fmt"),
            "{:?}",
            events[0]
        );
    }

    /// One entry of a tree, read without following links.
    #[derive(Debug, PartialEq, Eq)]
    enum Entry {
        File(Vec<u8>),
        Dir,
        Link(std::path::PathBuf),
    }

    /// Every entry under `dir`, by relative path. A link is recorded with its target and never
    /// followed, so swapping a file for a link to the same bytes shows.
    fn snapshot(dir: &std::path::Path) -> std::collections::BTreeMap<String, Entry> {
        fn walk(
            base: &std::path::Path,
            dir: &std::path::Path,
            out: &mut std::collections::BTreeMap<String, Entry>,
        ) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                let rel = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let kind = fs::symlink_metadata(&path).unwrap().file_type();
                if kind.is_symlink() {
                    out.insert(rel, Entry::Link(fs::read_link(&path).unwrap()));
                } else if kind.is_dir() {
                    out.insert(rel, Entry::Dir);
                    walk(base, &path, out);
                } else {
                    out.insert(rel, Entry::File(fs::read(&path).unwrap()));
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        walk(dir, dir, &mut out);
        out
    }

    /// Once sync has installed a code plugin, `plugins trust` records trust from the installed
    /// files, and `plugins status` then reports the plugin as trusted.
    #[test]
    fn trust_of_a_synced_code_plugin_records_trust_and_exits_0() {
        let sandbox = Sandbox::new();
        lock_code_plugin(&sandbox);
        sandbox
            .slopvac()
            .args(["--offline", "plugins", "sync"])
            .assert()
            .success();
        sandbox
            .slopvac()
            .args(["--offline", "plugins", "trust", "fmt"])
            .assert()
            .success()
            .stdout(predicate::str::contains(
                "trusted `fmt` with content hash sha256:",
            ));
        assert_eq!(trust_records(&sandbox), 1);
        sandbox
            .slopvac()
            .args(["--offline", "plugins", "status"])
            .assert()
            .success()
            .stdout(predicate::function(|out: &str| {
                out.lines().any(|line| {
                    line.starts_with("fmt 1.0.0 (code): ")
                        && line.ends_with("; trusted")
                        && !line.contains("not trusted")
                })
            }));
    }

    /// A locked code plugin with no installed tree fails for that reason, not for a missing loader.
    /// The debug log names the stable text id of the refusal and the plugin, since the command
    /// withholds the loader's own text.
    #[test]
    fn trust_of_a_locked_code_plugin_that_is_not_synced_exits_2() {
        let sandbox = Sandbox::new();
        sandbox.write_lock(
            "\n[[plugins]]\nid = \"fmt\"\nkind = \"code\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
             source = \"oci://registry.invalid/acme/fmt:1.0.0\"\n\
             digest = \"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"\n",
        );
        let out = sandbox
            .slopvac()
            .env("SLOPVAC_LOG", "debug")
            .args(["--offline", "plugins", "trust", "fmt"])
            .assert()
            .code(2)
            .stderr(predicate::str::contains(
                "error[plugins-trust-inputs-failed]: ",
            ))
            .stderr(predicate::str::contains("the plugin files are not loaded").not());
        let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
        assert_debug_line(&stderr, "plugin-not-installed");
    }

    /// Trust is pinned to the synced tree: a file changed after sync is refused, and nothing is
    /// recorded. The debug log names the stable text id of the refusal.
    #[test]
    fn trust_of_a_code_plugin_changed_since_sync_exits_2_and_records_nothing() {
        let sandbox = Sandbox::new();
        lock_code_plugin(&sandbox);
        sandbox
            .slopvac()
            .args(["--offline", "plugins", "sync"])
            .assert()
            .success();
        let manifest = sandbox.project().join("plugins/fmt/slopvac-plugin.toml");
        let original = fs::read_to_string(&manifest).unwrap();
        fs::write(
            &manifest,
            original.replace("hooks = []", "args = [\"--extra\"]\nhooks = []"),
        )
        .unwrap();
        let out = sandbox
            .slopvac()
            .env("SLOPVAC_LOG", "debug")
            .args(["--offline", "plugins", "trust", "fmt"])
            .assert()
            .code(2)
            .stderr(predicate::str::contains(
                "error[plugins-trust-inputs-failed]: ",
            ));
        let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
        assert_debug_line(&stderr, "plugin-tampered");
        // Checked before the manifest is restored, and in the trust store itself.
        assert_eq!(trust_records(&sandbox), 0);
    }

    /// A manifest over 1 MiB stops `plugins trust` before it parses anything. The command reports
    /// its own code, the debug log names `plugin-too-large`, and nothing is recorded.
    #[test]
    fn trust_of_a_code_plugin_with_a_manifest_over_1_mib_exits_2_and_records_nothing() {
        let sandbox = Sandbox::new();
        lock_code_plugin(&sandbox);
        sandbox
            .slopvac()
            .args(["--offline", "plugins", "sync"])
            .assert()
            .success();
        let manifest = sandbox.project().join("plugins/fmt/slopvac-plugin.toml");
        let mut text = fs::read_to_string(&manifest).unwrap();
        text.push_str(&"#".repeat(1 << 20));
        fs::write(&manifest, text).unwrap();
        let out = sandbox
            .slopvac()
            .env("SLOPVAC_LOG", "debug")
            .args(["--offline", "plugins", "trust", "fmt"])
            .assert()
            .code(2)
            .stderr(predicate::str::contains(
                "error[plugins-trust-inputs-failed]: ",
            ));
        let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
        assert_debug_line(&stderr, "plugin-too-large");
        assert_eq!(trust_records(&sandbox), 0);
    }

    /// An attacker who writes the cache can plant a tree and a record that points at it. The
    /// trusted hash comes from the lock and the cached archive, so the plant is refused.
    #[test]
    fn trust_of_an_archive_plugin_with_a_swapped_tree_and_a_forged_record_exits_2() {
        let sandbox = Sandbox::new();
        let honest = plugin_targz(PLUGIN_MANIFEST);
        fs::create_dir_all(sandbox.project().join("plugins")).unwrap();
        fs::write(sandbox.project().join("plugins/fmt.tar.gz"), &honest).unwrap();
        let digest = slopvac_plugins::oci::Digest::of(&honest);
        sandbox.write_lock(&format!(
            "\n[[plugins]]\nid = \"fmt\"\nkind = \"code\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
             source = \"plugins/fmt.tar.gz\"\ndigest = \"{digest}\"\n"
        ));
        sandbox
            .slopvac()
            .args(["--offline", "plugins", "sync"])
            .assert()
            .success();

        let store = plugin_store(&sandbox);
        let trees = store.join("trees");
        let record = store.join("refs").join(digest.hex());
        let honest_hash = slopvac_plugins::extract::TreeHash::from_hex(
            fs::read_to_string(&record).unwrap().trim(),
        )
        .unwrap();
        // Put the evil tree where the honest one was, and point the record at a second copy.
        let evil = plugin_targz(&PLUGIN_MANIFEST.replace("--serve", "--evil"));
        let aside = sandbox.dir.path().join("aside");
        let old = trees.join(format!("tree-{}", honest_hash.hex()));
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&old, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::create_dir_all(&aside).unwrap();
        fs::rename(&old, aside.join("honest")).unwrap();
        let planted = extract_plugin(&evil, &aside.join("scratch"));
        move_tree_to(&planted, &trees, &honest_hash);
        let forged = extract_plugin(&evil, &trees);
        fs::write(&record, forged.hash.hex()).unwrap();

        let before = snapshot(&trees);
        let out = sandbox
            .slopvac()
            .env("SLOPVAC_LOG", "debug")
            .args(["--offline", "plugins", "trust", "fmt"])
            .assert()
            .code(2)
            .stderr(predicate::str::contains(
                "error[plugins-trust-inputs-failed]: ",
            ));
        let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
        assert_debug_line(&stderr, "plugin-tampered");
        assert_eq!(trust_records(&sandbox), 0);
        // A read path removes nothing: the trees are byte for byte what they were.
        assert_eq!(snapshot(&trees), before, "trust changed the store");
    }

    /// A lock edited to name a planted artifact has no signature that sync ever checked. `trust` is
    /// the point where a person decides, so it checks the cached signature again.
    #[test]
    fn trust_of_a_planted_registry_plugin_without_a_valid_signature_exits_2() {
        use slopvac_plugins::oci::{Descriptor, Digest, MEDIA_EMPTY, MEDIA_MANIFEST, Manifest};
        use slopvac_plugins::sync::{ARTIFACT_TYPE_PACK, MEDIA_PACK_LAYER};
        let sandbox = Sandbox::new();
        let archive = plugin_targz(PLUGIN_MANIFEST);
        let manifest = Manifest {
            schema_version: 2,
            media_type: Some(MEDIA_MANIFEST.to_owned()),
            artifact_type: Some(ARTIFACT_TYPE_PACK.to_owned()),
            config: Descriptor::for_bytes(MEDIA_EMPTY, b"{}"),
            layers: vec![Descriptor::for_bytes(MEDIA_PACK_LAYER, &archive)],
            subject: None,
            annotations: std::collections::BTreeMap::new(),
        };
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let digest = Digest::of(&manifest_bytes);
        sandbox.write_lock(&format!(
            "\n[[plugins]]\nid = \"fmt\"\nkind = \"code\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
             source = \"oci://registry.invalid/acme/fmt:1.0.0\"\ndigest = \"{digest}\"\n\
             [plugins.signature]\nkey_id = \"A1B2C3D4E5F60718\"\nnot_before = \"2026-11-01T00:00:00Z\"\n"
        ));
        // The cache as a successful sync leaves it, without a signature layer.
        let store = plugin_store(&sandbox);
        fs::create_dir_all(store.join("blobs")).unwrap();
        fs::write(store.join("blobs").join(digest.hex()), &manifest_bytes).unwrap();
        fs::write(
            store.join("blobs").join(Digest::of(&archive).hex()),
            &archive,
        )
        .unwrap();
        let done = extract_plugin(&archive, &store.join("trees"));
        fs::create_dir_all(store.join("refs")).unwrap();
        fs::write(store.join("refs").join(digest.hex()), done.hash.hex()).unwrap();

        sandbox
            .slopvac()
            .args(["--offline", "plugins", "trust", "fmt"])
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with("error[signature-invalid]: "));
        assert_eq!(trust_records(&sandbox), 0);
    }

    /// A cached signature file that is not what its name says is an input `trust` cannot use, so it
    /// is reported under the code of the command, as a tree that differs is. The command leaves
    /// the cache as it found it: it repairs nothing.
    #[test]
    fn trust_of_a_registry_plugin_with_a_swapped_signature_file_exits_2_and_changes_nothing() {
        use slopvac_plugins::oci::{Descriptor, Digest, MEDIA_EMPTY, MEDIA_MANIFEST, Manifest};
        use slopvac_plugins::sign::{MEDIA_MINISIG, MEDIA_STATEMENT};
        use slopvac_plugins::sync::{ARTIFACT_TYPE_PACK, MEDIA_PACK_LAYER};
        let sandbox = Sandbox::new();
        let archive = plugin_targz(PLUGIN_MANIFEST);
        let statement = b"{}".to_vec();
        let minisig = b"untrusted comment: x\n".to_vec();
        let manifest = Manifest {
            schema_version: 2,
            media_type: Some(MEDIA_MANIFEST.to_owned()),
            artifact_type: Some(ARTIFACT_TYPE_PACK.to_owned()),
            config: Descriptor::for_bytes(MEDIA_EMPTY, b"{}"),
            layers: vec![
                Descriptor::for_bytes(MEDIA_PACK_LAYER, &archive),
                Descriptor::for_bytes(MEDIA_STATEMENT, &statement),
                Descriptor::for_bytes(MEDIA_MINISIG, &minisig),
            ],
            subject: None,
            annotations: std::collections::BTreeMap::new(),
        };
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let digest = Digest::of(&manifest_bytes);
        sandbox.write_lock(&format!(
            "\n[[plugins]]\nid = \"fmt\"\nkind = \"code\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
             source = \"oci://registry.invalid/acme/fmt:1.0.0\"\ndigest = \"{digest}\"\n\
             [plugins.signature]\nkey_id = \"A1B2C3D4E5F60718\"\nnot_before = \"2026-11-01T00:00:00Z\"\n"
        ));
        let store = plugin_store(&sandbox);
        let blobs = store.join("blobs");
        fs::create_dir_all(&blobs).unwrap();
        fs::write(blobs.join(digest.hex()), &manifest_bytes).unwrap();
        fs::write(blobs.join(Digest::of(&archive).hex()), &archive).unwrap();
        // The statement file holds other bytes than its name says.
        fs::write(blobs.join(Digest::of(&statement).hex()), b"swapped").unwrap();
        fs::write(blobs.join(Digest::of(&minisig).hex()), &minisig).unwrap();
        let done = extract_plugin(&archive, &store.join("trees"));
        fs::create_dir_all(store.join("refs")).unwrap();
        fs::write(store.join("refs").join(digest.hex()), done.hash.hex()).unwrap();
        let before = cached_files(&blobs);

        sandbox
            .slopvac()
            .args(["--offline", "plugins", "trust", "fmt"])
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with(
                "error[plugins-trust-inputs-failed]: ",
            ));

        assert_eq!(cached_files(&blobs), before, "trust changed the cache");
        assert_eq!(trust_records(&sandbox), 0);
    }

    /// Every file in `dir`, by name, with its bytes.
    fn cached_files(dir: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap())
            .filter(|e| e.path().is_file())
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    fs::read(e.path()).unwrap(),
                )
            })
            .collect()
    }

    /// With the update check off, each command that has no network path finishes while a proxy
    /// that counts and answers nothing stands in for the network, and it sees no connection. The
    /// run has 60 seconds, so a command that waits on a refused connection fails the test.
    ///
    /// Conformance: N1
    #[rstest::rstest]
    #[case::lint(&["lint", "doc.md"])]
    #[case::validate(&["validate", "--config", "slopvac.toml"])]
    #[case::rules(&["rules"])]
    #[case::explain(&["explain", "core:ai-residue.chat-leakage"])]
    #[case::prime(&["prime"])]
    #[case::reference(&["reference"])]
    #[case::claims(&["claims", "doc.md", "--original", "doc.md"])]
    fn a_command_with_no_network_path_finishes_when_every_connection_is_refused(
        #[case] args: &[&str],
    ) {
        let sandbox = Sandbox::new();
        fs::write(
            sandbox.project().join("slopvac.toml"),
            "schema_version = 3\n",
        )
        .unwrap();
        fs::write(
            sandbox.project().join("doc.md"),
            "# Title\n\nPlain words here.\n",
        )
        .unwrap();
        let tally = NetworkTally::start();
        let mut cmd = sandbox.slopvac();
        tally.route(&mut cmd);
        let out = cmd
            .args(args)
            .timeout(std::time::Duration::from_secs(60))
            .output()
            .expect("the command finishes within 60 seconds");
        assert!(
            matches!(out.status.code(), Some(0..=2)),
            "the command did not exit normally: {:?}",
            out.status
        );
        assert_eq!(tally.requests(), 0, "the command opened a connection");
    }

    /// The long flags that one help page lists.
    fn long_flags(help: &str) -> Vec<String> {
        help.split_whitespace()
            .filter(|word| word.starts_with("--"))
            .map(|word| {
                word.trim_end_matches(|c: char| !c.is_ascii_alphanumeric())
                    .split('=')
                    .next()
                    .unwrap_or_default()
                    .to_string()
            })
            .collect()
    }

    /// `--offline` is on every help page, and no page lists a flag whose name turns the network
    /// on. Only `--offline` itself is a network flag.
    ///
    /// Conformance: N3
    #[rstest::rstest]
    #[case::root(&["--help"])]
    #[case::lint(&["lint", "--help"])]
    #[case::plugins_sync(&["plugins", "sync", "--help"])]
    #[case::review(&["review", "--help"])]
    fn the_help_lists_no_flag_that_turns_offline_mode_off(#[case] args: &[&str]) {
        let sandbox = Sandbox::new();
        let out = sandbox.slopvac().args(args).assert().success();
        let help = String::from_utf8(out.get_output().stdout.clone()).unwrap();
        let flags = long_flags(&help);
        assert!(
            flags.iter().any(|f| f == "--offline"),
            "--offline is missing:\n{help}"
        );
        let turn_on: Vec<&String> = flags
            .iter()
            .filter(|f| f.contains("online") || f.contains("network") || f.contains("no-offline"))
            .collect();
        assert!(turn_on.is_empty(), "{turn_on:?} in:\n{help}");
    }

    /// `plugins sync` installs a code plugin and records no trust for it.
    ///
    /// Conformance: N8
    #[test]
    fn sync_never_records_trust_for_a_code_plugin() {
        let sandbox = Sandbox::new();
        lock_code_plugin(&sandbox);
        sandbox
            .slopvac()
            .args(["--offline", "plugins", "sync"])
            .assert()
            .success();
        assert_eq!(trust_records(&sandbox), 0);
    }

    /// `plugins update` leaves the trust store as it found it, with a code plugin installed.
    ///
    /// Conformance: N8
    #[test]
    fn update_never_records_trust_for_a_code_plugin() {
        let sandbox = Sandbox::new();
        lock_code_plugin(&sandbox);
        sandbox
            .slopvac()
            .args(["--offline", "plugins", "sync"])
            .assert()
            .success();
        sandbox
            .slopvac()
            .args(["--offline", "plugins", "update"])
            .assert()
            .code(2);
        assert_eq!(trust_records(&sandbox), 0);
    }

    /// Neither command takes `--yes`, and each refuses it as a usage error.
    ///
    /// Conformance: N8
    #[rstest::rstest]
    #[case::sync(&["--offline", "plugins", "sync", "--yes"])]
    #[case::update(&["--offline", "plugins", "update", "--yes"])]
    fn sync_and_update_refuse_a_yes_flag(#[case] args: &[&str]) {
        let sandbox = Sandbox::new();
        lock_code_plugin(&sandbox);
        sandbox
            .slopvac()
            .args(args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("--yes"));
    }

    /// In CI, a locked path plugin whose files no longer hash to the lock's digest stops the run
    /// with exit 2, and the lock stays as it was.
    ///
    /// Conformance: N7
    #[test]
    fn sync_in_ci_exits_2_on_a_digest_mismatch_and_leaves_the_lock() {
        let sandbox = Sandbox::new();
        lock_path_plugin(&sandbox);
        fs::write(
            sandbox.project().join("slopvac.toml"),
            "schema_version = 3\n[[plugins]]\nid = \"style\"\nsource = \"plugins/style\"\n",
        )
        .unwrap();
        fs::write(sandbox.project().join("plugins/style/rules.yml"), "a: c\n").unwrap();
        let before = lock_bytes(&sandbox);
        sandbox
            .slopvac()
            .env("CI", "true")
            .args(["--offline", "plugins", "sync"])
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with("error[digest-mismatch]: "));
        assert_eq!(lock_bytes(&sandbox), before);
    }
}
