//! Conformance test N1: with the update check off, the offline commands finish in a
//! network namespace that has no route out.
//!
//! The binary runs inside `unshare --user --map-root-user --net` or `bwrap --unshare-net`.
//! Inside, loopback is a separate interface that is down, so any connection fails. A
//! self-test proves this before any command runs: the parent binds a listener on 127.0.0.1
//! and a probe connects to it. Outside the sandbox the probe connects, inside it must not.
//! The suite never contacts the internet. The control is a listener on loopback, not a
//! closed port or a TEST-NET address: a closed port refuses the connection even with no
//! sandbox, so it would report "blocked" when nothing is isolated. A listener gives a real
//! positive control and stays local.
//!
//! On a host that is not Linux no namespace can exist, so the tests always skip with a
//! message. On Linux, when no sandbox can be built (no tool, user namespaces blocked) the
//! tests fail with the collected reasons if `CI` is set to a non-empty value or
//! `SLOPVAC_REQUIRE_SANDBOX=1` is set. With neither set they print the reason and skip.
//!
//! Each command gets 30 seconds. A command that hangs on a connection is killed and fails
//! with its case name and stderr.
//!
//! What this does not prove: a command that tries to connect, fails and carries on silently
//! still passes. The test sees the exit code and the `offline-network-needed` code, not the
//! attempts.
//!
//! `reference` is not covered because it is not wired yet (it answers `command-not-wired`).

use std::fs::File;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use rstest::rstest;

const PROBE_ENV: &str = "SLOPVAC_N1_PROBE_ADDR";
const REQUIRE_ENV: &str = "SLOPVAC_REQUIRE_SANDBOX";
const OK_MARKER: &str = "n1-probe: connected";
const BLOCKED_MARKER: &str = "n1-probe: blocked";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// The connection probe. In a normal run `PROBE_ENV` is unset and this does nothing. The
/// sandbox self-test runs this test binary again with it set, once outside and once inside
/// the sandbox, and reads the marker it prints.
#[test]
fn sandbox_self_test_connection_probe() {
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
/// The `bwrap` form binds `/tmp` read-write, where every test directory lives.
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
}

/// A scratch directory under `/tmp`, which `bwrap` binds.
fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("slopvac-n1-")
        .tempdir_in("/tmp")
        .expect("scratch directory under /tmp")
}

/// Run `cmd` for at most `limit`. The output goes to files in `dir`, so a full pipe cannot
/// stall the child. Returns the exit code (`None` if killed by a signal) and both streams.
/// On expiry the child is killed and `Err` carries what it had written.
fn run_bounded(
    mut cmd: Command,
    dir: &Path,
    limit: Duration,
) -> Result<(Option<i32>, String, String), String> {
    let out_path = dir.join("stdout.txt");
    let err_path = dir.join("stderr.txt");
    cmd.stdin(Stdio::null())
        .stdout(File::create(&out_path).expect("stdout file"))
        .stderr(File::create(&err_path).expect("stderr file"));
    let mut child = cmd.spawn().map_err(|e| format!("cannot spawn: {e}"))?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| format!("wait failed: {e}"))? {
            break Some(status);
        }
        if start.elapsed() >= limit {
            let _ = child.kill(); // an already exited child is fine
            let _ = child.wait(); // reap it
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let read =
        |p: &PathBuf| String::from_utf8_lossy(&std::fs::read(p).unwrap_or_default()).into_owned();
    let (stdout, stderr) = (read(&out_path), read(&err_path));
    match status {
        Some(status) => Ok((status.code(), stdout, stderr)),
        None => Err(format!(
            "timed out after {}s\nstdout: {stdout}\nstderr: {stderr}",
            limit.as_secs()
        )),
    }
}

/// Run the connection probe, directly or through `sandbox`, and return its marker line.
fn probe(sandbox: Option<&Sandbox>, addr: SocketAddr) -> Result<String, String> {
    let exe = std::env::current_exe().expect("test binary path");
    let mut cmd = match sandbox {
        Some(sandbox) => sandbox.command(&exe),
        None => Command::new(&exe),
    };
    cmd.args([
        "--exact",
        "sandbox_self_test_connection_probe",
        "--nocapture",
    ])
    .env(PROBE_ENV, addr.to_string());
    let dir = scratch();
    let (_, stdout, stderr) = run_bounded(cmd, dir.path(), PROBE_TIMEOUT)?;
    [OK_MARKER, BLOCKED_MARKER]
        .into_iter()
        .find(|m| stdout.contains(m))
        .map(str::to_owned)
        .ok_or_else(|| format!("no probe result: {}", stderr.trim()))
}

/// Find a wrapper that runs and blocks a connection, or say why none does.
fn find_sandbox() -> Result<Sandbox, String> {
    if !cfg!(target_os = "linux") {
        return Err("network namespaces need Linux".into());
    }
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("loopback bind: {e}"))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    // Control: outside any sandbox the probe reaches the listener.
    match probe(None, addr) {
        Ok(m) if m == OK_MARKER => {}
        other => {
            return Err(format!(
                "the probe cannot reach loopback unwrapped: {other:?}"
            ));
        }
    }
    let mut reasons = Vec::new();
    for (tool, prefix) in WRAPPERS {
        let sandbox = Sandbox { tool, prefix };
        match probe(Some(&sandbox), addr) {
            Err(why) => reasons.push(format!("{tool} cannot create the sandbox here: {why}")),
            Ok(m) if m == BLOCKED_MARKER => return Ok(sandbox),
            Ok(_) => reasons.push(format!("{tool} ran but did not block the connection")),
        }
    }
    Err(reasons.join("; "))
}

/// The sandbox for this machine, looked up once. `None` means skip.
fn sandbox() -> Option<&'static Sandbox> {
    static SANDBOX: OnceLock<Result<Sandbox, String>> = OnceLock::new();
    match SANDBOX.get_or_init(find_sandbox) {
        Ok(sandbox) => {
            eprintln!("n1 network sandbox active: {}", sandbox.tool);
            Some(sandbox)
        }
        Err(reason) => {
            let required = cfg!(target_os = "linux")
                && (std::env::var(REQUIRE_ENV).as_deref() == Ok("1")
                    || std::env::var_os("CI").is_some_and(|v| !v.is_empty()));
            assert!(
                !required,
                "a network sandbox is required (CI or {REQUIRE_ENV}=1) but none works here: {reason}"
            );
            eprintln!("n1 network sandbox SKIPPED: {reason}");
            None
        }
    }
}

fn binary() -> PathBuf {
    assert_cmd::Command::cargo_bin("slopvac")
        .expect("binary builds")
        .get_program()
        .into()
}

/// Run `slopvac <args>` in a fresh project inside the sandbox and check the exit code.
fn assert_offline_run(name: &str, args: &[&str], expected_exit: i32) {
    let Some(sandbox) = sandbox() else {
        return;
    };
    let dir = scratch();
    let project = dir.path();
    let home = project.join("home");
    std::fs::create_dir_all(&home).expect("home");
    std::fs::write(project.join("a.md"), "The cache holds 10 entries.\n").expect("a.md");
    std::fs::write(project.join("b.md"), "The cache holds 20 entries.\n").expect("b.md");
    let out_dir = project.join("out");
    std::fs::create_dir_all(&out_dir).expect("out");

    let mut cmd = sandbox.command(&binary());
    cmd.args(args).current_dir(project).env_clear();
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

    let (code, stdout, stderr) = match run_bounded(cmd, &out_dir, COMMAND_TIMEOUT) {
        Ok(done) => done,
        Err(why) => panic!("{name}: {why}"),
    };
    assert_eq!(
        code,
        Some(expected_exit),
        "{name}: exit code\nstderr: {stderr}"
    );
    assert!(
        !stdout.contains("offline-network-needed") && !stderr.contains("offline-network-needed"),
        "{name} reported that it needed the network\nstderr: {stderr}"
    );
}

#[rstest]
#[case::lint("lint", &["lint", "a.md"], 0)]
#[case::validate("validate", &["validate"], 0)]
#[case::rules("rules", &["rules"], 0)]
#[case::explain("explain", &["explain", "core:ai-residue.chat-leakage"], 0)]
#[case::prime("prime", &["prime"], 0)]
#[case::claims("claims", &["claims", "b.md", "--original", "a.md"], 0)]
fn offline_command_finishes_with_no_route_out(
    #[case] name: &str,
    #[case] args: &[&str],
    #[case] expected_exit: i32,
) {
    assert_offline_run(name, args, expected_exit);
}
