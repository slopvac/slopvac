//! Process groups, descendants and the environment allowlist.
//!
//! Each plugin has one teardown owner, a [`Group`]. A kill takes the group's
//! lock and runs only while the leader is unreaped. An unreaped leader, even a
//! zombie, keeps its process id, and a process group id stays in use while its
//! leader's id does, so neither number can name an unrelated process at kill
//! time. Only the waiter reaps the leader. It observes the exit without
//! reaping, signals the group under the same lock, and reaps last.

use std::process::{Child, ExitStatus};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

static LIVE: Mutex<Vec<Arc<Group>>> = Mutex::new(Vec::new());

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Kill every plugin process group that is still running.
///
/// The binary's Ctrl-C handler calls this, so no plugin outlives the host
/// (plugin-protocol.md section 3). Each kill goes through the plugin's
/// [`Group`], so it waits for a teardown in flight and never signals a reaped
/// leader's id.
pub fn kill_all_live() {
    let groups: Vec<Arc<Group>> = lock(&LIVE).clone();
    for g in groups {
        g.kill();
    }
}

#[derive(Default)]
struct State {
    /// The leader's exit status, once it is reaped. No kill runs after that.
    status: Option<String>,
    /// A kill ran before the leader was reaped.
    forced: bool,
    /// Descendants seen earlier, for children that left the group.
    strays: Vec<u32>,
    /// The leader is reaped and the group unregistered.
    done: bool,
}

/// The single teardown owner of one plugin's process group.
pub(crate) struct Group {
    pid: u32,
    state: Mutex<State>,
    done: Condvar,
}

impl Group {
    /// Track the group a new leader heads, and register it for [`kill_all_live`].
    pub(crate) fn register(pid: u32) -> Arc<Self> {
        let g = Arc::new(Self {
            pid,
            state: Mutex::new(State::default()),
            done: Condvar::new(),
        });
        lock(&LIVE).push(Arc::clone(&g));
        g
    }

    /// Kill the group and the known descendants, unless the leader is reaped.
    ///
    /// Blocks while another kill or the reap is in flight.
    pub(crate) fn kill(&self) {
        let mut st = lock(&self.state);
        if st.status.is_none() {
            st.forced = true;
            kill_tree(self.pid, &st.strays);
        }
    }

    /// Remember the current descendants, so a daemon that outlives its parent is still found.
    pub(crate) fn remember_descendants(&self) {
        let mut st = lock(&self.state);
        if st.status.is_none() {
            st.strays = descendants(self.pid);
        }
    }

    /// Whether a kill ran before the leader was reaped.
    pub(crate) fn forced(&self) -> bool {
        lock(&self.state).forced
    }

    /// The leader's exit status, once it is reaped.
    pub(crate) fn status(&self) -> Option<String> {
        lock(&self.state).status.clone()
    }

    /// End the group after its leader exited: signal everything still in it
    /// while the leader holds its ids, reap the leader, then unregister.
    ///
    /// Called by the waiter only. A leader that has not exited yet is
    /// signalled and then reaped.
    pub(crate) fn finish(&self, leader: Leader) {
        {
            let mut st = lock(&self.state);
            kill_tree(self.pid, &st.strays);
            st.status = Some(leader.reap());
        }
        lock(&LIVE).retain(|g| !std::ptr::eq(Arc::as_ptr(g), self));
        lock(&self.state).done = true;
        self.done.notify_all();
    }

    /// Wait up to `timeout` until the leader is reaped and the group unregistered.
    pub(crate) fn wait_done(&self, timeout: Duration) -> bool {
        let end = Instant::now() + timeout;
        let mut st = lock(&self.state);
        while !st.done {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            st = self
                .done
                .wait_timeout(st, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }
}

/// The plugin's top process, owned by the waiter.
pub(crate) struct Leader {
    child: Child,
    exited: Option<std::io::Result<ExitStatus>>,
}

impl Leader {
    pub(crate) fn new(child: Child) -> Self {
        Self {
            child,
            exited: None,
        }
    }

    /// Take the three standard pipes.
    pub(crate) fn take_pipes(
        &mut self,
    ) -> (
        Option<std::process::ChildStdin>,
        Option<std::process::ChildStdout>,
        Option<std::process::ChildStderr>,
    ) {
        (
            self.child.stdin.take(),
            self.child.stdout.take(),
            self.child.stderr.take(),
        )
    }

    /// Block until the leader exits, but leave it unreaped so its ids stay reserved.
    #[cfg(unix)]
    pub(crate) fn wait_exit(&mut self) {
        use rustix::io::Errno;
        use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
        let Some(pid) = i32::try_from(self.child.id()).ok().and_then(Pid::from_raw) else {
            return;
        };
        let opts = WaitIdOptions::EXITED | WaitIdOptions::NOWAIT;
        loop {
            match waitid(WaitId::Pid(pid), opts) {
                Ok(Some(s)) if s.exited() || s.killed() || s.dumped() => return,
                Err(Errno::INTR) => {}
                // macOS also reports a stop under WEXITED (golang/go#19314).
                Ok(_) => std::thread::sleep(Duration::from_millis(20)),
                // Not waitable: `finish` then kills the still unreaped leader.
                Err(_) => return,
            }
        }
    }

    /// Block until the leader exits. The open process handle keeps its id reserved.
    #[cfg(not(unix))]
    pub(crate) fn wait_exit(&mut self) {
        self.exited = Some(self.child.wait());
    }

    /// Reap the leader and release its ids. Returns the exit status as text.
    fn reap(mut self) -> String {
        let status = match self.exited.take() {
            Some(s) => s,
            None => self.child.wait(),
        };
        drop(self.child);
        match status {
            Ok(s) => s.to_string(),
            Err(e) => format!("wait failed: {e}"),
        }
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
/// Callers hold the [`Group`] lock with the leader unreaped.
#[cfg(unix)]
fn kill_tree(pid: u32, known: &[u32]) {
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
fn kill_tree(pid: u32, _known: &[u32]) {
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

    #[cfg(unix)]
    #[test]
    fn observing_the_exit_leaves_the_leader_unreaped() {
        use std::process::{Command, Stdio};
        let child = Command::new("sh")
            .args(["-c", "exit 7"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut leader = Leader::new(child);
        leader.wait_exit();
        // A zombie still holds its id: signal 0 reaches it.
        let id = rustix::process::Pid::from_raw(i32::try_from(pid).unwrap()).unwrap();
        let still_held = rustix::process::test_kill_process(id).is_ok();
        let status = leader.reap();
        let released = rustix::process::test_kill_process(id).is_err();
        assert!(still_held, "the exit was observed by reaping");
        assert!(status.contains('7'), "{status}");
        assert!(released, "reaping releases the id");
    }
}
