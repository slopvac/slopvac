//! A model provider plugin run through the installed `slopvac` binary, for end-to-end tests.
//!
//! [`World`] builds a project with two documents and the provider plugin in
//! `fixtures/review_provider.py`. The plugin is installed as a path source, locked by its tree
//! hash, and the project has per-user directories of its own. [`World::trust`] syncs the plugin
//! and trusts it with `slopvac plugins`. [`World::review`] runs `slopvac review`, which starts
//! the plugin through the host and the binary's own launcher mode. The plugin logs each start,
//! the resource limits it runs under, and every `provider/complete` request, so a test sees
//! what started and what it was sent. Nothing in the binary is replaced.
#![allow(
    dead_code,
    unreachable_pub,
    reason = "shared by more than one test binary, and each uses only part of it"
)]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use serde_json::Value;
use snapbox::cmd::{Command, cargo_bin};
use std::fs;
use std::path::{Path, PathBuf};

const FAKE: &str = include_str!("../fixtures/review_provider.py");

/// The manifest of the provider plugin, which runs `bin/provider` with the log directory.
fn manifest(log_dir: &Path, hooks: &str) -> String {
    format!(
        "schema_version = 3\nid = \"provider\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
         kind = \"code\"\n[provides]\n[plugin]\ncommand = \"bin/provider\"\n\
         args = ['{}']\nnetwork = false\nhooks = {hooks}\n\
         features = {{ required = [], optional = [] }}\n",
        log_dir.display()
    )
}

/// The `hooks` value of a manifest that declares one provider hook.
pub(crate) const PROVIDER_HOOK: &str = "[{ kind = \"provider\", id = \"review\" }]";

/// A Python 3 that runs with an empty environment, as a plugin child does.
fn python() -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|dir| dir.join("python3"))
        .find(|cand| {
            std::process::Command::new(cand)
                .env_clear()
                .args(["-c", "import json"])
                .status()
                .is_ok_and(|s| s.success())
        })
        .expect("a python3 on PATH that runs with an empty environment")
}

/// The soft and hard values of one resource limit, as the plugin read them at its start.
/// `None` is unlimited.
pub(crate) type Limit = (Option<u64>, Option<u64>);

/// A project with two documents, a provider plugin locked by its tree hash, a config, and
/// per-user directories of its own.
pub(crate) struct World {
    dir: tempfile::TempDir,
}

impl World {
    /// A world whose plugin declares the one provider hook.
    pub(crate) fn new(docs: (&str, &str), config: &str) -> Self {
        Self::with_hooks(docs, config, PROVIDER_HOOK)
    }

    /// A world whose plugin manifest declares `hooks`.
    pub(crate) fn with_hooks(docs: (&str, &str), config: &str, hooks: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let world = Self {
            dir: tempfile::tempdir().expect("tempdir"),
        };
        let project = world.project();
        let plugin = project.join("plugins/provider");
        fs::create_dir_all(plugin.join("bin")).unwrap();
        fs::create_dir_all(project.join(".slopvac")).unwrap();
        fs::create_dir_all(world.logs()).unwrap();
        fs::write(project.join("old.md"), docs.0).unwrap();
        fs::write(project.join("new.md"), docs.1).unwrap();
        fs::write(project.join("slopvac.toml"), config).unwrap();
        fs::write(
            plugin.join("slopvac-plugin.toml"),
            manifest(&world.logs(), hooks),
        )
        .unwrap();
        let script = plugin.join("bin/provider");
        fs::write(&script, format!("#!{}\n{FAKE}", python().display())).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        // A plugin tree lists every executable file in its executable list.
        fs::write(plugin.join(".slopvac-exec"), "bin/provider\n").unwrap();
        let digest = slopvac_plugins::extract::tree_hash(&plugin).unwrap();
        fs::write(
            project.join(".slopvac/plugins.lock"),
            format!(
                "schema_version = 3\n\n[[plugins]]\nid = \"provider\"\nkind = \"code\"\n\
                 version = \"1.0.0\"\nslopvac_api = 1\nsource = \"plugins/provider\"\n\
                 digest = \"{digest}\"\n"
            ),
        )
        .unwrap();
        world
    }

    /// The project directory, where every command runs.
    pub(crate) fn project(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    /// The directory the plugin logs to.
    pub(crate) fn logs(&self) -> PathBuf {
        self.dir.path().join("logs")
    }

    /// The installed binary, run in the project with per-user directories of the world's own.
    pub(crate) fn slopvac(&self) -> Command {
        let home = self.dir.path().join("home");
        let mut cmd = Command::new(cargo_bin!("slopvac"))
            .current_dir(self.project())
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
            cmd = cmd.env_remove(var);
        }
        cmd
    }

    /// Sync the plugin in place and record the user's trust in it.
    pub(crate) fn trust(&self) {
        self.slopvac()
            .args(["--offline", "plugins", "sync"])
            .assert()
            .success();
        self.slopvac()
            .args(["--offline", "plugins", "trust", "provider"])
            .assert()
            .success();
    }

    /// Replace the project config.
    pub(crate) fn write_config(&self, text: &str) {
        fs::write(self.project().join("slopvac.toml"), text).unwrap();
    }

    /// How the plugin answers its next request.
    pub(crate) fn script(&self, script: &Value) {
        fs::write(self.logs().join("script.json"), script.to_string()).unwrap();
    }

    /// How many times the plugin process started.
    pub(crate) fn spawns(&self) -> usize {
        fs::read_to_string(self.logs().join("spawns")).map_or(0, |s| s.lines().count())
    }

    /// The resource limits of the last start, by name: `cpu`, `core`, `nofile` and `fsize`.
    pub(crate) fn limits(&self, name: &str) -> Limit {
        let text = fs::read_to_string(self.logs().join("limits")).expect("the plugin started");
        let last = text.lines().next_back().expect("one line per start");
        let all: Value = serde_json::from_str(last).unwrap();
        let pair = all[name].as_array().expect("a soft and a hard value");
        (pair[0].as_u64(), pair[1].as_u64())
    }

    /// The params of every `provider/complete` request the plugin received.
    pub(crate) fn requests(&self) -> Vec<Value> {
        fs::read_to_string(self.logs().join("requests"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// The body of each `provider/complete` request, the bytes the plugin received.
    pub(crate) fn raw_requests(&self) -> Vec<String> {
        fs::read_to_string(self.logs().join("requests.raw"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// `slopvac review new.md --original old.md`, with `extra` arguments.
    pub(crate) fn review(&self, extra: &[&str]) -> std::process::Output {
        let mut args = vec!["review", "new.md", "--original", "old.md"];
        args.extend_from_slice(extra);
        self.slopvac().args(args).output().expect("run")
    }
}

/// A project config with the `[review]` table `review`.
pub(crate) fn config(review: &str) -> String {
    format!("schema_version = 3\n\n[review]\n{review}")
}

/// A config that names the provider and the reviewer model.
pub(crate) fn configured() -> String {
    config("provider = \"provider\"\nmodel = \"reviewer-model\"\n")
}

/// The standard error of a run, as text.
pub(crate) fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The review plan a run printed on standard output.
pub(crate) fn plan(out: &std::process::Output) -> Value {
    let parsed = serde_json::from_slice::<Value>(&out.stdout);
    assert!(
        parsed.is_ok(),
        "a JSON plan on stdout: {parsed:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        stderr(out)
    );
    parsed.unwrap()
}

/// The run exited 2 with an `error[<code>]:` line.
pub(crate) fn assert_refused(out: &std::process::Output, code: &str) {
    let err = stderr(out);
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(
        err.lines()
            .any(|l| l.starts_with(&format!("error[{code}]: "))),
        "want error[{code}]:\n{err}"
    );
}
