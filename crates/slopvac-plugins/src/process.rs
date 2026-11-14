//! Process groups, descendants and the environment allowlist.

use std::sync::{Mutex, PoisonError};

static LIVE: Mutex<Vec<u32>> = Mutex::new(Vec::new());

pub(crate) fn register(pid: u32) {
    LIVE.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(pid);
}

pub(crate) fn unregister(pid: u32) {
    LIVE.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|&p| p != pid);
}

/// Kill every plugin process group that is still running.
///
/// The binary's Ctrl-C handler calls this, so no plugin outlives the host
/// (plugin-protocol.md section 3).
pub fn kill_all_live() {
    let pids: Vec<u32> = LIVE.lock().unwrap_or_else(PoisonError::into_inner).clone();
    for pid in pids {
        kill_tree(pid, &[]);
    }
}

/// Every descendant of `root` according to the process table.
#[cfg(unix)]
pub(crate) fn descendants(root: u32) -> Vec<u32> {
    use std::collections::HashSet;
    use std::process::Command;
    let Ok(out) = Command::new("ps").args(["-A", "-o", "pid=,ppid="]).output() else {
        return Vec::new();
    };
    let table: Vec<(u32, u32)> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .collect();
    let mut found: HashSet<u32> = HashSet::new();
    let mut frontier = vec![root];
    while let Some(p) = frontier.pop() {
        for &(pid, ppid) in &table {
            if ppid == p && pid != root && found.insert(pid) {
                frontier.push(pid);
            }
        }
    }
    found.into_iter().collect()
}

/// Kill the process group led by `pid` and every descendant seen before the kill.
///
/// The descendant walk catches children that left the group with `setsid` or a
/// double fork, as far as they exist at kill time (threat model P5, R2).
#[cfg(unix)]
pub(crate) fn kill_tree(pid: u32, known: &[u32]) {
    use rustix::process::{Pid, Signal, kill_process, kill_process_group};
    let mut strays = descendants(pid);
    strays.extend_from_slice(known);
    let raw = |p: u32| i32::try_from(p).ok().and_then(Pid::from_raw);
    if let Some(leader) = raw(pid) {
        // Errors mean the group is already gone.
        let _ = kill_process_group(leader, Signal::KILL);
    }
    for p in strays {
        if let Some(p) = raw(p) {
            let _ = kill_process(p, Signal::KILL);
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn kill_tree(pid: u32, _known: &[u32]) {
    let _ = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .output();
}

/// Replace control characters other than newline and tab, plus bidi overrides.
#[must_use]
pub fn sanitize_display(text: &str) -> String {
    text.chars()
        .map(|c| {
            let bidi = matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
            if (c.is_control() && c != '\n' && c != '\t') || bidi {
                '\u{FFFD}'
            } else {
                c
            }
        })
        .collect()
}

#[cfg(not(unix))]
pub(crate) fn descendants(_root: u32) -> Vec<u32> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_newline_and_tab_and_replaces_escapes() {
        let got = sanitize_display("a\nb\tc\u{1b}[31m\u{9b}\u{7f}\u{202e}x");
        assert_eq!(got, "a\nb\tc\u{FFFD}[31m\u{FFFD}\u{FFFD}\u{FFFD}x");
    }
}
