//! Conformance test N1: with the update check off, the offline commands finish under a
//! network namespace that has no route out.
//!
//! The binary runs inside `unshare --user --map-root-user --net` or `bwrap --unshare-net`.
//! A direct connection there fails, so a command that dials out on its own, around any proxy,
//! would fail or hang. A self-test proves the sandbox blocks a connection before any command
//! runs. Where no sandbox can be built (not Linux, no tool, user namespaces blocked) the test
//! prints the reason and returns, because the contract needs a real sandbox to mean anything.
//!
//! `reference` is not wired yet (it answers `command-not-wired`), so it is not covered here.

use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

const PROBE_ENV: &str = "SLOPVAC_N1_PROBE_ADDR";
const OK_MARKER: &str = "n1-probe: connected";
const BLOCKED_MARKER: &str = "n1-probe: blocked";
/// A public address. Nothing is ever sent to it from inside the sandbox.
const PUBLIC_ADDR: &str = "1.1.1.1:443";

/// The helper that the self-test runs through the wrapper. In a normal run `PROBE_ENV` is
/// unset and it does nothing. The self-test re-runs this test binary with it set.
#[test]
fn n1_connect_probe() {
    let Ok(addr) = std::env::var(PROBE_ENV) else {
        return;
    };
    let addr: SocketAddr = addr.parse().expect("probe address");
    match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
        Ok(_) => println!("{OK_MARKER}"),
        Err(_) => println!("{BLOCKED_MARKER}"),
    }
}

/// Command line prefixes that put a process in a fresh, empty network namespace.
const WRAPPERS: [(&str, &[&str]); 2] = [
    ("unshare", &["--user", "--map-root-user", "--net", "--"]),
    (
        "bwrap",
        &[
            "--unshare-net",
            "--unshare-user",
            "--ro-bind",
            "/",
            "/",
            "--bind",
            "/tmp",
            "/tmp",
            "--dev",
            "/dev",
            "--proc",
            "/proc",
            "--",
        ],
    ),
];

/// A wrapper that works on this machine, with the probe's verdict.
struct Sandbox {
    tool: &'static str,
    prefix: &'static [&'static str],
}

impl Sandbox {
    fn command(&self, program: &Path) -> Command {
        let mut cmd = Command::new(self.tool);
        cmd.args(self.prefix).arg(program);
        cmd
    }

    /// Run the connect probe through the wrapper. `None` when the wrapper itself failed.
    fn probe(&self, addr: &str) -> Option<String> {
        let exe = std::env::current_exe().expect("test binary path");
        let out = self
            .command(&exe)
            .args(["--exact", "n1_connect_probe", "--nocapture"])
            .env(PROBE_ENV, addr)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        (text.contains(OK_MARKER) || text.contains(BLOCKED_MARKER)).then_some(text)
    }
}

/// Find a wrapper that runs and blocks a direct connection, or say why none does.
fn find_sandbox() -> Result<Sandbox, String> {
    if !cfg!(target_os = "linux") {
        return Err("network namespaces need Linux".into());
    }
    let mut reasons = Vec::new();
    for (tool, prefix) in WRAPPERS {
        let sandbox = Sandbox { tool, prefix };
        match sandbox.probe(PUBLIC_ADDR) {
            None => reasons.push(format!("{tool} cannot create the sandbox here")),
            Some(text) if text.contains(BLOCKED_MARKER) => return Ok(sandbox),
            Some(_) => reasons.push(format!("{tool} ran but did not block {PUBLIC_ADDR}")),
        }
    }
    Err(reasons.join("; "))
}

fn binary() -> std::path::PathBuf {
    assert_cmd::Command::cargo_bin("slopvac")
        .expect("binary builds")
        .get_program()
        .into()
}

fn run_in_sandbox(sandbox: &Sandbox, project: &Path, args: &[&str]) -> Output {
    let home = project.join("home");
    let mut cmd = sandbox.command(&binary());
    cmd.args(args).current_dir(project).env_clear();
    // Keep PATH so the wrapper's child resolves; point every user directory inside the project.
    if let Some(path) = std::env::var_os("PATH") {
        cmd.env("PATH", path);
    }
    cmd.env("HOME", &home)
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_STATE_HOME", home.join("state"))
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .env("NO_COLOR", "1");
    cmd.output().expect("run the binary in the sandbox")
}

#[test]
fn offline_commands_finish_with_no_route_out() {
    let sandbox = match find_sandbox() {
        Ok(sandbox) => sandbox,
        Err(reason) => {
            eprintln!("n1 network sandbox SKIPPED: {reason}");
            return;
        }
    };
    eprintln!("n1 network sandbox active: {}", sandbox.tool);

    let dir = tempfile::tempdir().expect("tempdir");
    let project = dir.path();
    std::fs::create_dir_all(project.join("home")).expect("home");
    std::fs::write(project.join("a.md"), "The cache holds 10 entries.\n").expect("a.md");
    std::fs::write(project.join("b.md"), "The cache holds 20 entries.\n").expect("b.md");

    let cases: [(&str, &[&str]); 6] = [
        ("lint", &["lint", "a.md"]),
        ("validate", &["validate"]),
        ("rules", &["rules"]),
        ("explain", &["explain", "core:ai-residue.chat-leakage"]),
        ("prime", &["prime"]),
        ("claims", &["claims", "b.md", "--original", "a.md"]),
    ];
    for (name, args) in cases {
        let out = run_in_sandbox(&sandbox, project, args);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            matches!(out.status.code(), Some(0 | 1)),
            "{name}: exit {:?}\nstderr: {stderr}",
            out.status.code()
        );
        assert!(
            !stdout.contains("offline-network-needed")
                && !stderr.contains("offline-network-needed"),
            "{name} reported that it needed the network\nstderr: {stderr}"
        );
    }
}
