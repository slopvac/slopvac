//! Shared helpers for the host tests: a scripted fake plugin and test gates.
#![allow(dead_code, unreachable_pub, missing_docs)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use slopvac_plugins::hooks::Document;
use slopvac_plugins::protocol::{FeatureSet, HookDecl};
use slopvac_plugins::{
    HostConfig, Plugin, PluginDeclaration, PluginError, PluginSpec, SpawnGate, Timeouts,
};

// STUB(slopvac-8l4.9.12): test-only gates stand in for the trust store.
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

pub fn cfg() -> HostConfig {
    HostConfig {
        timeouts: Timeouts {
            initialize: Duration::from_secs(5),
            hook: Duration::from_millis(800),
            provider: Duration::from_secs(2),
            cancel_grace: Duration::from_millis(400),
            shutdown: Duration::from_millis(500),
            exit: Duration::from_millis(500),
        },
        ..HostConfig::default()
    }
}

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

pub fn code<T: std::fmt::Debug>(r: &Result<T, PluginError>) -> String {
    r.as_ref()
        .expect_err("expected an error")
        .slopvac_code()
        .to_owned()
}

pub fn alive(pid: i32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .is_ok_and(|o| o.status.success())
}

pub fn wait_gone(pids: &[i32]) -> bool {
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end {
        if pids.iter().all(|&p| !alive(p)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

pub fn pids_from(file: &Path) -> Vec<i32> {
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end {
        if let Ok(s) = std::fs::read_to_string(file) {
            let v: Vec<i32> = s.lines().filter_map(|l| l.parse().ok()).collect();
            if v.len() == 2 {
                return v;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("pid file not written");
}

pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("slopvac-plugins-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
