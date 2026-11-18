//! The `slopvac` binary is the plugin launcher: the host re-runs it with the launcher flag, and
//! `main` must hand over before clap sees the arguments.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use assert_cmd::cargo::cargo_bin;
use serde_json::json;
use slopvac_plugins::launch::{FLAG, Launcher};
use slopvac_plugins::protocol::{FeatureSet, HookDecl};
use slopvac_plugins::{
    HostConfig, Plugin, PluginDeclaration, PluginError, PluginSpec, SpawnGate, Timeouts,
};

struct Allow;

impl SpawnGate for Allow {
    fn check(&self, _: &PluginSpec) -> Result<(), PluginError> {
        Ok(())
    }
}

fn slopvac() -> PathBuf {
    cargo_bin("slopvac")
}

/// The plugin fixture of the plugins package, a scripted fake that speaks the protocol.
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../slopvac-plugins/tests/fixtures/fake_plugin.py")
}

fn python() -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|dir| dir.join("python3"))
        .find(|cand| {
            Command::new(cand)
                .env_clear()
                .args(["-c", "import json, subprocess"])
                .status()
                .is_ok_and(|s| s.success())
        })
        .expect("a python3 on PATH")
}

#[test]
fn the_binary_runs_a_program_under_the_launcher_limits() {
    let out = Command::new(slopvac())
        .env_clear()
        .arg(FLAG)
        .args([
            "cpu=5",
            "nofile=64",
            "fsize=1048576",
            "data=536870912",
            "sweep=2000",
            "--",
            "/bin/sh",
            "-c",
            "ulimit -n; echo ran",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "64\nran\n");
}

#[test]
fn a_malformed_launcher_invocation_exits_127_and_is_not_clap_usage_text() {
    let out = Command::new(slopvac())
        .env_clear()
        .args([FLAG, "cpu=5"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(127), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.starts_with("slopvac plugin launcher:"), "{err}");
    assert!(!err.contains("usage"), "{err}");
}

#[test]
fn a_plugin_started_through_the_slopvac_binary_speaks_the_protocol() {
    let spec = PluginSpec {
        name: "fake".into(),
        program: python(),
        args: vec![
            fixture().display().to_string(),
            "ok".to_owned(),
            json!({}).to_string(),
        ],
        cwd: std::env::temp_dir(),
        declaration: PluginDeclaration {
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
        },
        path_env: "/usr/bin:/bin".into(),
        env: Vec::new(),
    };
    let cfg = HostConfig {
        timeouts: Timeouts {
            initialize: Duration::from_secs(30),
            hook: Duration::from_secs(30),
            shutdown: Duration::from_secs(10),
            exit: Duration::from_secs(10),
            ..Timeouts::default()
        },
        launcher: Launcher::Program(slopvac()),
        ..HostConfig::default()
    };
    let plugin = Plugin::start(&spec, &cfg, &Allow).expect("the binary launches the plugin");
    assert_eq!(plugin.negotiated().info.name, "fake");
    assert!(plugin.shutdown().clean());
}
