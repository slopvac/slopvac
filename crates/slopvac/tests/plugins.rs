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
fn extract_plugin(archive: &[u8], store: &std::path::Path) -> slopvac_plugins::extract::Extracted {
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

/// The line of the debug log that names `code` and the plugin `fmt`. It names nothing else:
/// no refusal sentence and no path.
fn assert_debug_line(stderr: &str, code: &str, sandbox: &Sandbox) {
    let line = stderr
        .lines()
        .find(|l| l.contains(code) && l.contains("fmt"));
    assert!(
        line.is_some(),
        "no debug line with `{code}` and `fmt`:\n{stderr}"
    );
    let line = line.unwrap();
    for text in [
        "differs from",
        "locked but not installed",
        "slopvac plugins sync",
        sandbox.dir.path().to_str().unwrap(),
    ] {
        assert!(!line.contains(text), "`{text}` in the debug line: {line}");
    }
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
    assert_debug_line(&stderr, "plugin-not-installed", &sandbox);
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
    assert_debug_line(&stderr, "plugin-tampered", &sandbox);
    // Checked before the manifest is restored, and in the trust store itself.
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
    let honest_hash =
        slopvac_plugins::extract::TreeHash::from_hex(fs::read_to_string(&record).unwrap().trim())
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
    assert_debug_line(&stderr, "plugin-tampered", &sandbox);
    assert_eq!(trust_records(&sandbox), 0);
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
