//! Shared helpers for the host tests: a scripted fake plugin and test gates.
#![allow(
    dead_code,
    unreachable_pub,
    missing_docs,
    reason = "test support: each test binary uses a subset of the helpers; test support: items are shared across test binaries; test crate: nothing public to document"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use slopvac_plugins::hooks::Document;
use slopvac_plugins::launch::Launcher;
use slopvac_plugins::protocol::{FeatureSet, HookDecl};
use slopvac_plugins::{
    HookKind, HostConfig, Plugin, PluginDeclaration, PluginError, PluginSpec, SpawnGate,
    StartError, Timeouts,
};

// Fixed-answer gates for the host protocol tests; the trust gate is tested in trust.rs.
pub struct Allow;
impl SpawnGate for Allow {
    fn check(&self, _: &PluginSpec) -> Result<(), PluginError> {
        Ok(())
    }
}

pub struct Deny(pub PluginError);
impl SpawnGate for Deny {
    fn check(&self, _: &PluginSpec) -> Result<(), PluginError> {
        Err(self.0.clone())
    }
}

pub fn python() -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let cand = dir.join("python3");
        // The plugin runs with a scrubbed environment, so the interpreter must too.
        let ok = Command::new(&cand)
            .env_clear()
            .args(["-c", "import json, subprocess"])
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            return cand;
        }
    }
    panic!("no usable python3 on PATH");
}

pub fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_plugin.py")
}

pub fn decl() -> PluginDeclaration {
    PluginDeclaration {
        network: false,
        hooks: vec![
            HookDecl {
                kind: "tagger".into(),
                id: "main".into(),
            },
            HookDecl {
                kind: "fixer".into(),
                id: "main".into(),
            },
        ],
        features: FeatureSet::default(),
    }
}

/// Every hook kind with id `main`, declared in the manifest.
pub fn decl_all() -> PluginDeclaration {
    PluginDeclaration {
        network: false,
        hooks: all_hooks(),
        features: FeatureSet::default(),
    }
}

fn all_hooks() -> Vec<HookDecl> {
    HookKind::ALL
        .iter()
        .map(|k| HookDecl {
            kind: k.as_str().into(),
            id: "main".into(),
        })
        .collect()
}

/// Initialize overrides that offer every hook kind with id `main`.
pub fn offer_all() -> Value {
    json!({"capabilities": {"hooks": all_hooks()
        .iter()
        .map(|h| json!({"kind": h.kind, "id": h.id}))
        .collect::<Vec<_>>()}})
}

pub fn spec_with(
    mode: &str,
    overrides: &Value,
    pidfile: Option<&Path>,
    declaration: PluginDeclaration,
) -> PluginSpec {
    let mut args = vec![
        fixture().display().to_string(),
        mode.to_owned(),
        overrides.to_string(),
    ];
    if let Some(p) = pidfile {
        args.push(p.display().to_string());
    }
    PluginSpec {
        name: "fake".into(),
        program: python(),
        args,
        cwd: std::env::temp_dir(),
        declaration,
        path_env: "/usr/bin:/bin".into(),
        env: vec![("ALLOWED_VAR".into(), "1".into())],
    }
}

pub fn spec(mode: &str) -> PluginSpec {
    spec_with(mode, &json!({}), None, decl())
}

/// The launcher binary of this package, which the tests' plugins start through.
pub fn launcher() -> Launcher {
    Launcher::Program(env!("CARGO_BIN_EXE_slopvac-plugin-launcher").into())
}

/// Timers far above anything a healthy test needs, so load cannot fire them.
pub fn cfg() -> HostConfig {
    HostConfig {
        timeouts: Timeouts {
            initialize: Duration::from_secs(30),
            hook: Duration::from_secs(30),
            provider: Duration::from_secs(30),
            cancel_grace: Duration::from_secs(10),
            shutdown: Duration::from_secs(10),
            exit: Duration::from_secs(10),
            ..Timeouts::default()
        },
        launcher: launcher(),
        ..HostConfig::default()
    }
}

/// Short timers, for tests where a timer must fire against a plugin that hangs.
pub fn tight() -> HostConfig {
    HostConfig {
        timeouts: Timeouts {
            initialize: Duration::from_secs(30),
            hook: Duration::from_millis(800),
            provider: Duration::from_secs(2),
            cancel_grace: Duration::from_millis(400),
            shutdown: Duration::from_millis(500),
            exit: Duration::from_millis(500),
            ..Timeouts::default()
        },
        launcher: launcher(),
        ..HostConfig::default()
    }
}

/// A bound for "the host did not hang": the hanging fixtures sleep 300 s.
pub const NO_HANG: Duration = Duration::from_secs(20);

pub fn start(mode: &str) -> Plugin {
    Plugin::start(&spec(mode), &cfg(), &Allow).expect("plugin starts")
}

pub fn docs(n: usize) -> Vec<Document> {
    (0..n)
        .map(|i| Document {
            id: format!("d{i}"),
            text: format!("text {i}"),
            media_type: None,
            path: None,
        })
        .collect()
}

/// An error with a stable `slopvac_code`.
pub trait Coded: std::fmt::Debug {
    fn coded(&self) -> &str;
}

impl Coded for PluginError {
    fn coded(&self) -> &str {
        self.slopvac_code()
    }
}

impl Coded for StartError {
    fn coded(&self) -> &str {
        self.error.slopvac_code()
    }
}

pub fn code<T: std::fmt::Debug, E: Coded>(r: &Result<T, E>) -> String {
    r.as_ref()
        .expect_err("expected an error")
        .coded()
        .to_owned()
}

pub fn alive(pid: i32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .is_ok_and(|o| o.status.success())
}

pub fn wait_gone(pids: &[i32]) -> bool {
    wait_gone_within(pids, Duration::from_secs(10))
}

pub fn wait_gone_within(pids: &[i32], limit: Duration) -> bool {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if pids.iter().all(|&p| !alive(p)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// Kill processes the host leaves running on purpose, so no test leaks them.
pub fn end(pids: &[i32]) {
    for p in pids {
        let _ = Command::new("kill").args(["-9", &p.to_string()]).status();
    }
}

/// Split a fixture's pids, where `pids[0]` is in the plugin's process group and
/// the rest left it, into those the host kills and those it only reports.
/// Only Linux binds a process outside the group to its id (a pidfd); without
/// one the host never signals it (residual risk R2), so the test ends it.
pub fn reach(pids: &[i32]) -> (&[i32], &[i32]) {
    pids.split_at(if cfg!(target_os = "linux") {
        pids.len()
    } else {
        1
    })
}

pub fn pids_from(file: &Path) -> Vec<i32> {
    pids_n(file, 2)
}

pub fn pids_n(file: &Path, n: usize) -> Vec<i32> {
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        if let Ok(s) = std::fs::read_to_string(file) {
            let v: Vec<i32> = s.lines().filter_map(|l| l.parse().ok()).collect();
            if v.len() == n {
                return v;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("pid file not written");
}

/// Wait for a marker file that a fixture creates.
pub fn wait_for_file(file: &Path) {
    let end = Instant::now() + Duration::from_secs(20);
    while !file.exists() {
        assert!(Instant::now() < end, "{} never appeared", file.display());
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The messages the fixture received, from `<pidfile>.log`.
pub fn received(pidfile: &Path) -> Vec<Value> {
    let log = log_path(pidfile);
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

pub fn log_path(pidfile: &Path) -> PathBuf {
    let mut p = pidfile.as_os_str().to_owned();
    p.push(".log");
    PathBuf::from(p)
}

/// Wait until the fixture has received a message with `method`.
pub fn wait_received(pidfile: &Path, method: &str) {
    let end = Instant::now() + Duration::from_secs(20);
    while !received(pidfile).iter().any(|m| m["method"] == method) {
        assert!(Instant::now() < end, "`{method}` never arrived");
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("slopvac-plugins-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(unix)]
pub mod fake_uv;
