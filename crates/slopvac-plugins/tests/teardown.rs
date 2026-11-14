//! Teardown ordering and the process-table helper, observed through the test seam.
//!
//! A teardown reads the process table from `/bin/ps`. These tests point
//! `HostConfig::process_table` at a shell script instead, which the host runs
//! the same way: scrubbed environment, deadline, output limit. A script gets
//! its scratch directory as its first argument and the test process id as its
//! second. `kill_all_live` kills every plugin of the process, so these tests
//! run in their own binary and one at a time.
#![cfg(unix)]

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use common::*;
use serde_json::json;
use slopvac_plugins::process::TableSource;
use slopvac_plugins::{HostConfig, Plugin, kill_all_live};

/// One test at a time, so one test's `kill_all_live` cannot end another's plugin.
static SERIAL: Mutex<()> = Mutex::new(());

/// The production walk deadline.
const WALK: Duration = Duration::from_secs(2);

const PS: &str = "exec /bin/ps -A -o pid=,ppid=,pgid=,stat=,lstart=\n";

/// Log the plugin leader's state, then print the real table. With a `hold`
/// marker, the first run also creates `ps-started` and waits for `release`,
/// so a teardown is held in flight.
const OBSERVE: &str = r#"d="$1"
leader=$(sed -n 2p "$d/pids" 2>/dev/null)
if [ -n "$leader" ]; then
  state=$(/bin/ps -o stat= -p "$leader" | tr -d ' ')
  echo "${state:-gone}" >> "$d/leader-states"
fi
if [ -e "$d/hold" ] && mkdir "$d/held" 2>/dev/null; then
  : > "$d/ps-started"
  while [ ! -e "$d/release" ]; do sleep 0.01; done
fi
"#;

/// A scratch directory and a config whose table helper runs `script`, with
/// `deadline` for each teardown's walk.
fn seam(name: &str, script: &str, deadline: Duration) -> (PathBuf, HostConfig) {
    let dir = scratch(name);
    let file = dir.join("table.sh");
    std::fs::write(&file, script).unwrap();
    let cfg = HostConfig {
        process_table: Some(TableSource {
            program: "/bin/sh".into(),
            args: vec![
                file.into(),
                dir.clone().into(),
                std::process::id().to_string().into(),
            ],
            deadline,
        }),
        ..cfg()
    };
    (dir, cfg)
}

fn start_mode(mode: &str, dir: &Path, cfg: &HostConfig) -> Plugin {
    Plugin::start(
        &spec_with(mode, &json!({}), Some(&dir.join("pids")), decl()),
        cfg,
        &Allow,
    )
    .unwrap()
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
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (dir, cfg) = seam("reap-order", &format!("{OBSERVE}{PS}"), WALK);
    // A plugin that leaves a same-group child holding its pipes, then exits.
    let _p = start_mode("orphan-init", &dir, &cfg);
    let pids = pids_from(&dir.join("pids"));
    // The leader has exited; the child dies only by the host's teardown kill.
    let gone = wait_gone(&pids);
    let states = leader_states(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
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
    name: &str,
    action: impl FnOnce(Plugin) -> Option<Plugin> + Send + 'static,
) {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // The deadline outlasts the hold, so only the release ends the teardown.
    let (dir, cfg) = seam(name, &format!("{OBSERVE}{PS}"), NO_HANG * 3);
    std::fs::write(dir.join("hold"), "").unwrap();
    let p = start_mode("orphan-init", &dir, &cfg);
    let pids = pids_from(&dir.join("pids"));
    // The waiter is now inside its teardown, before the group kill.
    wait_for_file(&dir.join("ps-started"));
    let (tx, rx) = mpsc::channel();
    let t = std::thread::spawn(move || {
        let rest = action(p);
        tx.send(()).unwrap();
        drop(rest);
    });
    // Nothing lets the action finish while the helper holds, so a return
    // inside this window is the bug.
    let early = rx.recv_timeout(Duration::from_secs(1)).is_ok();
    std::fs::write(dir.join("release"), "").unwrap();
    t.join().unwrap();
    let gone = wait_gone(&pids);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(!early, "returned while the teardown was still in flight");
    assert!(gone, "processes {pids:?} survived");
}

#[test]
fn dropping_a_plugin_waits_for_a_teardown_in_flight() {
    waits_for_held_teardown("hold-drop", |p| {
        drop(p);
        None
    });
}

#[test]
fn an_interrupt_waits_for_a_teardown_in_flight() {
    waits_for_held_teardown("hold-interrupt", |p| {
        kill_all_live();
        Some(p)
    });
}

#[test]
fn a_hung_table_helper_neither_blocks_the_kill_nor_outlives_it() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (dir, cfg) = seam(
        "hung",
        "echo $$ > \"$1/helper\"\nexec /bin/sleep 60\n",
        WALK,
    );
    let p = start_mode("children", &dir, &cfg);
    // pids[0] stays in the plugin's group; pids[1] left it with `setsid`.
    let pids = pids_from(&dir.join("pids"));
    let started = Instant::now();
    kill_all_live();
    let took = started.elapsed();
    let helper: i32 = std::fs::read_to_string(dir.join("helper"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let group_gone = wait_gone(&pids[..1]);
    let helper_gone = wait_gone(&[helper]);
    // The walk failed, so the child that left the group is the test's to end.
    let _ = std::process::Command::new("kill")
        .args(["-9", &pids[1].to_string()])
        .status();
    drop(p);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(took < NO_HANG, "the kill waited {took:?} for the helper");
    assert!(group_gone, "the group member {} survived", pids[0]);
    assert!(helper_gone, "the helper {helper} survived its deadline");
}

#[test]
fn table_rows_naming_the_host_or_init_as_descendants_are_never_signalled() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // The `hang` fixture writes the leader's pid at `initialize`. The rows
    // claim the host and pid 1 as stopped children of the stopped leader.
    let forged = r#"leader=$(cat "$1/pids")
t='Sat Nov 14 23:00:07 2026'
echo "1 $leader 1 T $t"
echo "$leader $2 $leader T $t"
echo "$2 $leader $2 T $t"
"#;
    let (dir, cfg) = seam("forged", forged, WALK);
    let p = start_mode("hang", &dir, &cfg);
    let leader = pids_n(&dir.join("pids"), 1);
    // A stopped host would hang here; a killed one would end the test binary.
    kill_all_live();
    let gone = wait_gone(&leader);
    drop(p);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(gone, "the leader {leader:?} survived");
}

/// On Linux each descendant is signalled through a pidfd whose parent is
/// checked after it is opened, so a row that lies about the parent of an
/// unrelated process does not reach it.
#[cfg(target_os = "linux")]
#[test]
fn a_row_that_lies_about_a_process_parent_does_not_reach_that_process() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut victim = std::process::Command::new("/bin/sleep")
        .arg("300")
        .spawn()
        .unwrap();
    let forged = format!(
        r#"leader=$(cat "$1/pids")
t='Sat Nov 14 23:00:07 2026'
echo "$leader $2 $leader T $t"
echo "{v} $leader {v} S $t"
"#,
        v = victim.id()
    );
    let (dir, cfg) = seam("forged-parent", &forged, WALK);
    let p = start_mode("hang", &dir, &cfg);
    let leader = pids_n(&dir.join("pids"), 1);
    kill_all_live();
    let gone = wait_gone(&leader);
    let untouched = victim.try_wait().unwrap().is_none()
        && std::fs::read_to_string(format!("/proc/{}/stat", victim.id()))
            .unwrap()
            .rsplit(')')
            .next()
            .is_some_and(|rest| rest.trim_start().starts_with('S'));
    let _ = victim.kill();
    let _ = victim.wait();
    drop(p);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(gone, "the leader {leader:?} survived");
    assert!(untouched, "the unrelated process was signalled");
}
