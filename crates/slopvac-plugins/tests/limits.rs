//! Resource limits, the stalled-frame rule and stderr logging (threat model
//! P6, TM-5, P14; plugin-protocol.md sections 6.3 and 7).
#![cfg(unix)]

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use serde_json::json;
use slopvac_plugins::{HostConfig, Plugin, PluginError};

/// Wait up to `within` for the failure that ended the plugin.
fn failure_within(p: &Plugin, within: Duration) -> Option<PluginError> {
    let end = Instant::now() + within;
    loop {
        if let Some(e) = p.failure() {
            return Some(e);
        }
        if Instant::now() > end {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn start_with(mode: &str, c: &HostConfig) -> Plugin {
    Plugin::start(&spec(mode), c, &Allow).expect("plugin starts")
}

#[test]
fn tm5_a_busy_loop_ends_at_the_cpu_time_limit_with_a_diagnostic() {
    let mut c = cfg();
    c.caps.cpu_time = Duration::from_secs(1);
    let p = start_with("busy", &c);
    let err = failure_within(&p, NO_HANG).expect("the busy loop was not ended");
    assert_eq!(err.slopvac_code(), "plugin-exited", "{err}");
    assert!(err.to_string().contains("CPU time limit"), "{err}");
}

#[test]
fn a_write_past_the_file_size_limit_ends_the_plugin_with_a_diagnostic() {
    let dir = scratch("fsize");
    let pidfile = dir.join("pids");
    let mut c = cfg();
    c.caps.file_size = 1 << 20;
    let p = Plugin::start(
        &spec_with("big-file", &json!({}), Some(&pidfile), decl()),
        &c,
        &Allow,
    )
    .unwrap();
    let err = failure_within(&p, NO_HANG).expect("the write was not stopped");
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(err.slopvac_code(), "plugin-exited", "{err}");
    assert!(
        err.to_string()
            .contains("the file size limit of 1048576 bytes"),
        "{err}"
    );
}

/// The launcher dies of SIGKILL on its own only when its cleanup missed a
/// descendant; a plugin that kills it looks the same, and its orphans may
/// have left the host's reach.
#[cfg(target_os = "linux")]
#[test]
fn a_launcher_that_dies_of_sigkill_without_a_host_kill_leaves_the_teardown_incomplete() {
    let p = start_with("kill-launcher", &cfg());
    let err = failure_within(&p, NO_HANG).expect("the launcher's death was not noticed");
    assert_eq!(err.slopvac_code(), "plugin-exited", "{err}");
    assert!(p.teardown().incomplete, "{:?}", p.teardown());
}

/// Linux enforces the data limit on every private writable mapping.
#[cfg(target_os = "linux")]
#[test]
fn tm5_an_allocation_loop_ends_at_the_memory_limit_with_a_diagnostic() {
    let mut c = cfg();
    c.caps.memory = 256 << 20;
    let p = start_with("alloc", &c);
    let err = failure_within(&p, NO_HANG).expect("the allocation loop was not ended");
    assert_eq!(err.slopvac_code(), "plugin-exited", "{err}");
    assert!(err.to_string().contains("MemoryError"), "{err}");
}

/// macOS has no working memory limit (residual R2), so the wall-clock cap
/// ends the loop.
#[cfg(not(target_os = "linux"))]
#[test]
fn tm5_an_allocation_loop_ends_at_the_wall_clock_cap_with_a_diagnostic() {
    let mut c = cfg();
    c.caps.wall_clock = Duration::from_secs(3);
    let p = start_with("alloc", &c);
    let err = failure_within(&p, NO_HANG).expect("the allocation loop was not ended");
    assert_eq!(err.slopvac_code(), "plugin-timeout", "{err}");
    assert!(err.to_string().contains("wall-clock cap"), "{err}");
}

/// The ids of the processes in process group `pgid`.
fn members(pgid: i32) -> Vec<i32> {
    let out = std::process::Command::new("/bin/ps")
        .args(["-A", "-o", "pid=,pgid="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace().map(|s| s.parse::<i32>().ok());
            match (f.next().flatten(), f.next().flatten()) {
                (Some(member), Some(g)) if g == pgid => Some(member),
                _ => None,
            }
        })
        .collect()
}

#[test]
fn tm5_a_fork_loop_ends_at_the_wall_clock_cap_and_every_fork_dies() {
    let dir = scratch("tm5-fork");
    let pidfile = dir.join("pids");
    let mut c = cfg();
    c.caps.wall_clock = Duration::from_secs(3);
    let p = Plugin::start(
        &spec_with("fork-loop", &json!({}), Some(&pidfile), decl()),
        &c,
        &Allow,
    )
    .unwrap();
    let mut leader = pidfile.as_os_str().to_owned();
    leader.push(".leader");
    let group = pids_n(std::path::Path::new(&leader), 1)[0];
    let err = failure_within(&p, NO_HANG).expect("the fork loop was not ended");
    let until = Instant::now() + NO_HANG;
    let mut left = members(group);
    while !left.is_empty() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
        left = members(group);
    }
    end(&left);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(err.slopvac_code(), "plugin-timeout", "{err}");
    assert!(err.to_string().contains("wall-clock cap"), "{err}");
    assert!(left.is_empty(), "forks {left:?} survived");
}

#[test]
fn the_plugin_runs_under_the_configured_resource_limits() {
    let mut c = cfg();
    c.caps.cpu_time = Duration::from_secs(100);
    c.caps.open_files = 64;
    c.caps.file_size = 1 << 20;
    c.caps.memory = 512 << 20;
    let p = start_with("rlimits", &c);
    let r = p.tag("main", &docs(1)).unwrap();
    let evidence = r[0].outcome.as_ref().unwrap()[0].evidence.clone();
    let got: serde_json::Value = serde_json::from_str(&evidence).unwrap();
    // (soft, hard): the CPU hard limit is one second above the soft one, so
    // a plugin that catches SIGXCPU is killed a second later.
    assert_eq!(got["CPU"], json!([100, 101]));
    assert_eq!(got["NOFILE"], json!([64, 64]));
    assert_eq!(got["CORE"], json!([0, 0]));
    assert_eq!(got["FSIZE"], json!([1 << 20, 1 << 20]));
    if cfg!(target_os = "linux") {
        assert_eq!(got["DATA"], json!([512 << 20, 512 << 20]));
    }
}

#[test]
fn a_partial_frame_outside_any_request_is_a_protocol_violation_once_it_stalls() {
    let mut c = cfg();
    c.timeouts.stalled_frame = Duration::from_millis(500);
    let p = start_with("stall-idle", &c);
    let err = failure_within(&p, NO_HANG).expect("the stalled frame was not noticed");
    assert_eq!(err.slopvac_code(), "plugin-protocol-violation", "{err}");
}

#[test]
fn an_idle_plugin_without_a_partial_frame_is_not_stalled() {
    let mut c = cfg();
    c.timeouts.stalled_frame = Duration::from_millis(300);
    let p = start_with("ok", &c);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(p.failure(), None);
    assert!(p.tag("main", &docs(1)).is_ok());
}

/// One event: its level, target and fields.
type Logged = (tracing::Level, String, Vec<(String, String)>);

/// A subscriber that records every event's level, target and fields.
#[derive(Default, Clone)]
struct Capture(Arc<Mutex<Vec<Logged>>>);

struct Fields(Vec<(String, String)>);

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push((field.name().to_owned(), format!("{value:?}")));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.push((field.name().to_owned(), value.to_owned()));
    }
}

impl tracing::Subscriber for Capture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut fields = Fields(Vec::new());
        event.record(&mut fields);
        let m = event.metadata();
        self.0
            .lock()
            .unwrap()
            .push((*m.level(), m.target().to_owned(), fields.0));
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[test]
fn each_stderr_line_is_logged_at_debug_level_sanitized_and_cut_at_4_kib() {
    let capture = Capture::default();
    let events = Arc::clone(&capture.0);
    let dir = scratch("stderr-log");
    let pidfile = dir.join("pids");
    tracing::subscriber::with_default(capture, || {
        let p = Plugin::start(
            &spec_with("stderr-lines", &json!({}), Some(&pidfile), decl()),
            &cfg(),
            &Allow,
        )
        .unwrap();
        let _ = pids_n(&pidfile, 1);
        let end = Instant::now() + NO_HANG;
        while stderr_lines(&events).len() < 4 && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(p);
    });
    std::fs::remove_dir_all(&dir).unwrap();
    let lines = stderr_lines(&events);
    assert_eq!(
        lines,
        [
            (tracing::Level::DEBUG, "first line".to_owned()),
            // The escape introducer becomes a visible escape (`slopvac_sanitize::line`).
            (tracing::Level::DEBUG, r"second \u{1b}[31mline".to_owned()),
            (tracing::Level::DEBUG, "third".to_owned()),
            // Cut at 4 KiB.
            (tracing::Level::DEBUG, format!("{} [cut]", "x".repeat(4096))),
        ]
    );
}

/// The stderr events, as (level, message), each from the plugin `fake`.
fn stderr_lines(events: &Mutex<Vec<Logged>>) -> Vec<(tracing::Level, String)> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, target, _)| target == slopvac_plugins::host::STDERR_LOG_TARGET)
        .map(|(level, _, fields)| {
            let field = |n: &str| {
                fields
                    .iter()
                    .find(|(k, _)| k == n)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default()
            };
            assert_eq!(field("plugin"), "fake");
            (*level, field("message"))
        })
        .collect()
}
