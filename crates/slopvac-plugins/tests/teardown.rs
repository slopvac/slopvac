//! Teardown ordering, observed through a stand-in for `ps`.
//!
//! The host runs `ps` from its own `PATH` to find descendants just before it
//! signals a plugin group. Each test re-runs itself in a child test process
//! with a gate script first on `PATH`. The gate logs the plugin leader's
//! process state when it runs. With a `hold` marker, the first call also
//! creates `ps-started` and waits for `release`, so a teardown is held in
//! flight. `kill_all_live` kills every plugin of the process, so these tests
//! run in their own binary.
#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use common::*;
use serde_json::json;
use slopvac_plugins::{Plugin, kill_all_live};

const GATE_ENV: &str = "SLOPVAC_TEST_PS_GATE";

const GATE: &str = r#"#!/bin/sh
d="$SLOPVAC_TEST_PS_GATE"
leader=$(sed -n 2p "$d/pids" 2>/dev/null)
if [ -n "$leader" ]; then
  state=$(/bin/ps -o stat= -p "$leader" | tr -d ' ')
  echo "${state:-gone}" >> "$d/leader-states"
fi
if [ -e "$d/hold" ] && mkdir "$d/held" 2>/dev/null; then
  : > "$d/ps-started"
  while [ ! -e "$d/release" ]; do sleep 0.01; done
fi
exec /bin/ps "$@"
"#;

/// Re-run `test` in a child test process whose `PATH` starts with the gate.
///
/// Returns `None` in the parent once the child passed, and the gate directory
/// in the child, which then runs the test body.
fn gated(test: &str) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(GATE_ENV) {
        return Some(PathBuf::from(dir));
    }
    let dir = scratch(&format!("gate-{test}"));
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let ps = bin.join("ps");
    std::fs::write(&ps, GATE).unwrap();
    std::fs::set_permissions(&ps, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(&path));
    let out = Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact", "--nocapture", "--test-threads=1"])
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env(GATE_ENV, &dir)
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Guard against a filter that matched nothing.
    assert!(stdout.contains("1 passed"), "{stdout}");
    None
}

/// Start a plugin that leaves a same-group child holding its pipes, then exits.
fn start_orphan(dir: &Path) -> (Plugin, Vec<i32>) {
    let p = Plugin::start(
        &spec_with("orphan-init", &json!({}), Some(&dir.join("pids")), decl()),
        &cfg(),
        &Allow,
    )
    .unwrap();
    let pids = pids_from(&dir.join("pids"));
    (p, pids)
}

fn leader_states(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("leader-states"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_group_is_signalled_before_its_exited_leader_is_reaped() {
    let Some(dir) = gated("the_group_is_signalled_before_its_exited_leader_is_reaped") else {
        return;
    };
    let (_p, pids) = start_orphan(&dir);
    // The leader has exited; the child dies only by the host's teardown kill.
    let gone = wait_gone(&pids);
    let states = leader_states(&dir);
    assert!(gone, "processes {pids:?} survived");
    assert!(!states.is_empty(), "teardown never looked for descendants");
    // A reaped leader no longer shows in `ps`: its ids could name another process.
    assert!(
        states.iter().all(|s| s != "gone"),
        "a kill ran after the leader was reaped: {states:?}"
    );
}

/// Hold the teardown that starts when the leader exits, run `action` on a
/// thread, and fail if it returns while the teardown is held. `action` may
/// hand the plugin back to be dropped after that check.
fn waits_for_held_teardown(
    dir: &Path,
    action: impl FnOnce(Plugin) -> Option<Plugin> + Send + 'static,
) {
    std::fs::write(dir.join("hold"), "").unwrap();
    let (p, pids) = start_orphan(dir);
    // The waiter is now inside its teardown, before the group kill.
    wait_for_file(&dir.join("ps-started"));
    let (tx, rx) = mpsc::channel();
    let t = std::thread::spawn(move || {
        let rest = action(p);
        tx.send(()).unwrap();
        drop(rest);
    });
    // Nothing lets the action finish while the gate holds, so a return inside
    // this window is the bug. A slow machine can only hide it, not fake it.
    let early = rx.recv_timeout(Duration::from_secs(1)).is_ok();
    std::fs::write(dir.join("release"), "").unwrap();
    t.join().unwrap();
    let gone = wait_gone(&pids);
    assert!(!early, "returned while the teardown was still in flight");
    assert!(gone, "processes {pids:?} survived");
}

#[test]
fn dropping_a_plugin_waits_for_a_teardown_in_flight() {
    let Some(dir) = gated("dropping_a_plugin_waits_for_a_teardown_in_flight") else {
        return;
    };
    waits_for_held_teardown(&dir, |p| {
        drop(p);
        None
    });
}

#[test]
fn an_interrupt_waits_for_a_teardown_in_flight() {
    let Some(dir) = gated("an_interrupt_waits_for_a_teardown_in_flight") else {
        return;
    };
    waits_for_held_teardown(&dir, |p| {
        kill_all_live();
        Some(p)
    });
}
