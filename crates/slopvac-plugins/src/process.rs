//! Process groups, descendants and the environment allowlist.
//!
//! Each plugin has one teardown owner, a `Group`. A kill takes the group's
//! lock and runs only while the leader is unreaped. An unreaped leader, even a
//! zombie, keeps its process id, and a process group id stays in use while its
//! leader's id does, so neither number can name an unrelated process at kill
//! time. Only the waiter reaps the leader. It observes the exit without
//! reaping, tears the group down under the same lock, and reaps last.
//!
//! A teardown on Unix runs in three steps under that lock (threat model P5):
//!
//! 1. Stop the group with one `SIGSTOP`. Nothing a helper does can delay or
//!    redirect this signal. A stopped process cannot fork, reap a child or
//!    exit on its own.
//! 2. Walk the process table for descendants that left the group: a process
//!    whose parent is a stopped member of the group, or a descendant already
//!    stopped. Stop each one found and repeat until a round finds nothing new.
//!    The table comes from `/bin/ps`, never from `PATH`, run with a scrubbed
//!    environment, a deadline and an output limit, and parsed strictly. A
//!    helper that fails, hangs or prints anything unexpected ends the walk.
//! 3. Kill the stopped descendants, deepest first, then the group, whatever
//!    the walk found.
//!
//! No snapshot outlives the lock, and descendants are found only at kill time.
//! A process keeps its id while its parent is stopped, because nothing else
//! may reap it, so each descendant keeps the id it had in the snapshot until
//! the kill. Each later round also re-checks the parent and start time of
//! every descendant already stopped. On Linux each descendant is signalled
//! through a pidfd, and its parent is checked after the pidfd is opened.
//!
//! Residual risk R2: a parent that ignores `SIGCHLD` has exited children
//! reaped by the kernel even while stopped, so a numeric signal on macOS can
//! still reach a reused id in that window. A descendant whose parent exited
//! before the kill was reparented to init and is not found; on Linux the
//! subreaper of `slopvac-8l4.9.13` closes that gap.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Child, ExitStatus};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::{Duration, Instant};

static LIVE: Mutex<Vec<Arc<Group>>> = Mutex::new(Vec::new());

/// How long one teardown may spend reading the process table.
const WALK_DEADLINE: Duration = Duration::from_secs(2);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Kill every plugin process group that is still running.
///
/// The binary's interrupt handling calls this, so no plugin outlives the host
/// (plugin-protocol.md section 3). It stops every group first, then tears
/// each one down through its `Group`, so it waits for a teardown in flight
/// and never signals a reaped leader's id.
///
/// This function is not async-signal-safe (CWE-479): it takes locks, waits on
/// a condition variable, allocates and starts `/bin/ps`. Never call it from a
/// raw signal handler. The handler should only set an atomic flag or write a
/// byte to a self-pipe, and an ordinary thread that watches for it calls this
/// function. A signal crate that runs handlers on its own thread does the same.
pub fn kill_all_live() {
    let groups: Vec<Arc<Group>> = lock(&LIVE).clone();
    for g in &groups {
        g.freeze();
    }
    for g in &groups {
        g.kill();
    }
}

/// Where a teardown reads the process table. A test seam only.
///
/// `None` in [`HostConfig`](crate::HostConfig) means `/bin/ps` with a two
/// second deadline, which is what production uses; no lookup ever goes
/// through `PATH`. Tests point this at a script. The program runs exactly as
/// `/bin/ps` would: an environment of only `LC_ALL=C` and
/// `PATH=/usr/bin:/bin`, no input, the deadline and the output limit. It must
/// print what `ps -A -o pid=,ppid=,pgid=,stat=,lstart=` prints.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct TableSource {
    /// Absolute path of the program.
    pub program: PathBuf,
    /// Its arguments.
    pub args: Vec<OsString>,
    /// How long one teardown may spend on the walk.
    pub deadline: Duration,
}

#[derive(Default)]
struct State {
    /// The leader's exit status, once it is reaped. No kill runs after that.
    status: Option<String>,
    /// A kill ran before the leader was reaped.
    forced: bool,
    /// The leader is reaped and the group unregistered.
    done: bool,
}

/// What a teardown does to the group after the descendants that left it.
#[derive(Clone, Copy)]
enum Then {
    Kill,
    Resume,
}

/// The single teardown owner of one plugin's process group.
pub(crate) struct Group {
    pid: u32,
    table: Option<TableSource>,
    state: Mutex<State>,
    done: Condvar,
}

impl Group {
    /// Track the group a new leader heads, and register it for [`kill_all_live`].
    pub(crate) fn register(pid: u32, table: Option<TableSource>) -> Arc<Self> {
        let g = Arc::new(Self {
            pid,
            table,
            state: Mutex::new(State::default()),
            done: Condvar::new(),
        });
        lock(&LIVE).push(Arc::clone(&g));
        g
    }

    /// Stop the group now, unless the leader is reaped or a teardown already
    /// holds the lock (that teardown stops the group itself).
    fn freeze(&self) {
        let st = match self.state.try_lock() {
            Ok(st) => st,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => return,
        };
        if st.status.is_none() {
            stop_group(self.pid);
        }
    }

    /// Kill the group and the descendants that left it, unless the leader is reaped.
    ///
    /// Blocks while another teardown or the reap is in flight.
    pub(crate) fn kill(&self) {
        let mut st = lock(&self.state);
        if st.status.is_none() {
            st.forced = true;
            tear_down(self.pid, self.table.as_ref(), Then::Kill);
        }
    }

    /// Kill the descendants that left the group and let the group run on.
    ///
    /// `shutdown` calls this before the protocol exchange: a descendant that
    /// left the group is reparented when its parent exits, and the walk at
    /// the final kill no longer finds it.
    pub(crate) fn kill_strays(&self) {
        let st = lock(&self.state);
        if st.status.is_none() {
            tear_down(self.pid, self.table.as_ref(), Then::Resume);
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

    /// End the group after its leader exited: kill everything still in it
    /// while the leader holds its ids, reap the leader, then unregister.
    ///
    /// Called by the waiter only. A leader that has not exited yet is
    /// killed and then reaped.
    pub(crate) fn finish(&self, leader: Leader) {
        {
            let mut st = lock(&self.state);
            tear_down(self.pid, self.table.as_ref(), Then::Kill);
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
        use rustix::process::{WaitId, WaitIdOptions, waitid};
        let Some(pid) = unix::pid(self.child.id()) else {
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

#[cfg(unix)]
fn stop_group(leader: u32) {
    use rustix::process::{Signal, kill_process, kill_process_group};
    if let Some(pid) = unix::pid(leader) {
        // An error means the group is already gone, or the leader left it.
        let _ = kill_process_group(pid, Signal::STOP);
        let _ = kill_process(pid, Signal::STOP);
    }
}

/// Tear down the group led by `leader`, which the caller keeps unreaped.
///
/// The leader is signalled by its own id as well as through its group, since
/// it may have moved to another group of its session. No other group is
/// ever signalled.
#[cfg(unix)]
fn tear_down(leader: u32, table: Option<&TableSource>, then: Then) {
    use rustix::process::{Signal, kill_process, kill_process_group};

    /// Signals the group and the leader last, even if the walk unwinds.
    struct Last(rustix::process::Pid, Signal);
    impl Drop for Last {
        fn drop(&mut self) {
            let _ = kill_process_group(self.0, self.1);
            let _ = kill_process(self.0, self.1);
        }
    }

    let Some(pgid) = unix::pid(leader) else {
        return;
    };
    stop_group(leader);
    let _last = Last(
        pgid,
        match then {
            Then::Kill => Signal::KILL,
            Then::Resume => Signal::CONT,
        },
    );
    let within = table.map_or(WALK_DEADLINE, |t| t.deadline);
    let strays = unix::freeze_strays(leader, table, Instant::now() + within);
    for s in strays.iter().rev() {
        s.signal(Signal::KILL);
    }
}

#[cfg(not(unix))]
fn stop_group(_leader: u32) {}

/// `taskkill /T /F` on the leader's tree. The leader's open handle keeps its
/// id reserved; the tree walk itself is `taskkill`'s (job objects: NEW-3).
#[cfg(not(unix))]
fn tear_down(pid: u32, _table: Option<&TableSource>, then: Then) {
    use std::process::{Command, Stdio};
    if matches!(then, Then::Resume) {
        return;
    }
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let taskkill = std::path::Path::new(&root)
        .join("System32")
        .join("taskkill.exe");
    let Ok(mut child) = Command::new(taskkill)
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .env_clear()
        .env("SystemRoot", &root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    let deadline = Instant::now() + WALK_DEADLINE;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            _ => return,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
mod unix {
    //! The descendant walk and its process-table helper.

    use std::collections::{HashMap, HashSet};
    use std::ffi::OsStr;
    use std::io::Read;
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    use rustix::process::{Pid, Signal};

    use super::TableSource;

    /// The only program production runs to read the process table.
    const PS: &str = "/bin/ps";
    const PS_ARGS: [&str; 3] = ["-A", "-o", "pid=,ppid=,pgid=,stat=,lstart="];
    /// Largest process table the host reads.
    pub(super) const TABLE_MAX_BYTES: usize = 4 << 20;
    /// Rounds of the walk; each one reaches one level further down.
    const ROUNDS: usize = 8;

    pub(super) fn pid(raw: u32) -> Option<Pid> {
        i32::try_from(raw).ok().and_then(Pid::from_raw)
    }

    /// One row of the process table.
    #[derive(Debug, PartialEq, Eq)]
    pub(super) struct Entry {
        pub(super) pid: u32,
        pub(super) ppid: u32,
        pub(super) pgid: u32,
        pub(super) stat: String,
        /// Start time as `ps` prints it, to tell two holders of one id apart.
        pub(super) start: String,
    }

    impl Entry {
        fn stopped(&self) -> bool {
            self.stat.starts_with('T')
        }

        fn zombie(&self) -> bool {
            self.stat.starts_with('Z')
        }
    }

    /// A descendant outside the group that this teardown stopped.
    pub(super) struct Stray {
        pid: u32,
        ppid: u32,
        start: String,
        #[cfg(target_os = "linux")]
        fd: rustix::fd::OwnedFd,
    }

    impl Stray {
        /// A handle on the process in `e`, whose parent is stopped.
        ///
        /// The pidfd names whatever held the id when it was opened. A stopped
        /// parent cannot fork, so if that process still has the parent from
        /// the snapshot, it is the process from the snapshot.
        #[cfg(target_os = "linux")]
        fn open(e: &Entry) -> Option<Self> {
            use rustix::process::{PidfdFlags, pidfd_open};
            let fd = pidfd_open(pid(e.pid)?, PidfdFlags::empty()).ok()?;
            (proc_ppid(e.pid)? == e.ppid).then(|| Self {
                pid: e.pid,
                ppid: e.ppid,
                start: e.start.clone(),
                fd,
            })
        }

        /// A handle on the process in `e`, whose parent is stopped.
        ///
        /// macOS has no pidfd. The id cannot pass to another process while
        /// the parent is stopped, which is the only guard here (module docs).
        #[cfg(not(target_os = "linux"))]
        #[expect(clippy::unnecessary_wraps, reason = "matches the Linux signature")]
        fn open(e: &Entry) -> Option<Self> {
            Some(Self {
                pid: e.pid,
                ppid: e.ppid,
                start: e.start.clone(),
            })
        }

        pub(super) fn signal(&self, sig: Signal) {
            #[cfg(target_os = "linux")]
            let _ = rustix::process::pidfd_send_signal(&self.fd, sig);
            #[cfg(not(target_os = "linux"))]
            if let Some(p) = pid(self.pid) {
                let _ = rustix::process::kill_process(p, sig);
            }
        }
    }

    /// The parent of `pid` from `/proc/<pid>/stat`.
    #[cfg(target_os = "linux")]
    fn proc_ppid(pid: u32) -> Option<u32> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // The command name may hold spaces and parentheses; it ends at the last `)`.
        let rest = &stat[stat.rfind(')')? + 1..];
        rest.split_ascii_whitespace().nth(1)?.parse().ok()
    }

    /// Stop every descendant of the stopped group `leader` that left the group.
    ///
    /// Returns them in the order they were stopped, parents before children.
    pub(super) fn freeze_strays(
        leader: u32,
        source: Option<&TableSource>,
        deadline: Instant,
    ) -> Vec<Stray> {
        let me = std::process::id();
        let mut frozen: Vec<Stray> = Vec::new();
        for _ in 0..ROUNDS {
            let Some(table) = snapshot(source, deadline) else {
                break;
            };
            // A table without the leader is not this system's. The leader may
            // have left its group, and the walk starts from it either way.
            if !table.contains_key(&leader) {
                break;
            }
            // A stray whose parent or start time changed is not the process
            // that was stopped: resume what holds the id now.
            frozen.retain(|s| {
                let same = table
                    .get(&s.pid)
                    .is_some_and(|e| e.ppid == s.ppid && e.start == s.start);
                if !same {
                    s.signal(Signal::CONT);
                }
                same
            });
            let ours = |e: &Entry| {
                e.pid == leader || e.pgid == leader || frozen.iter().any(|s| s.pid == e.pid)
            };
            let held: HashSet<u32> = table
                .values()
                .filter(|e| ours(e) && e.stopped())
                .map(|e| e.pid)
                .collect();
            // Signals reach a process asynchronously; look again once all have stopped.
            let settling = table
                .values()
                .any(|e| ours(e) && !e.stopped() && !e.zombie());
            let mut found: Vec<&Entry> = table
                .values()
                .filter(|e| {
                    e.pgid != leader
                        && e.pid != leader
                        && held.contains(&e.ppid)
                        && e.pid > 1
                        && e.pid != me
                        && !frozen.iter().any(|s| s.pid == e.pid)
                })
                .collect();
            if found.is_empty() && !settling {
                break;
            }
            found.sort_by_key(|e| e.pid);
            for e in found {
                if let Some(s) = Stray::open(e) {
                    s.signal(Signal::STOP);
                    frozen.push(s);
                }
            }
            if settling {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        frozen
    }

    fn snapshot(source: Option<&TableSource>, deadline: Instant) -> Option<HashMap<u32, Entry>> {
        let out = match source {
            Some(s) => run_bounded(&s.program, &s.args, deadline)?,
            None => run_bounded(Path::new(PS), &PS_ARGS, deadline)?,
        };
        parse_table(&out)
    }

    /// Run a helper and return its standard output, or `None` if it failed,
    /// passed `deadline` or printed more than [`TABLE_MAX_BYTES`].
    ///
    /// The helper gets an empty environment apart from `LC_ALL=C` and
    /// `PATH=/usr/bin:/bin`, no input, and a process group of its own, which
    /// is killed before the helper is reaped.
    pub(super) fn run_bounded<S: AsRef<OsStr>>(
        program: &Path,
        args: &[S],
        deadline: Instant,
    ) -> Option<Vec<u8>> {
        use std::os::unix::process::CommandExt;
        let mut child = Command::new(program)
            .args(args)
            .env_clear()
            .env("LC_ALL", "C")
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .ok()?;
        let out = read_bounded(child.stdout.take(), deadline);
        let exited = out.is_some() && exited_by(&child, deadline);
        // Still unreaped, so the helper's ids are its own. The helper itself
        // is killed too, in case it moved to another group.
        let _ = child.kill();
        if let Some(g) = pid(child.id()) {
            let _ = rustix::process::kill_process_group(g, Signal::KILL);
        }
        let ok = child.wait().is_ok_and(|s| s.success());
        out.filter(|_| exited && ok)
    }

    /// Read to end of stream on a thread, up to the limit and the deadline.
    fn read_bounded(pipe: Option<std::process::ChildStdout>, deadline: Instant) -> Option<Vec<u8>> {
        let mut pipe = pipe?;
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let limit = TABLE_MAX_BYTES as u64 + 1;
            let read = (&mut pipe).take(limit).read_to_end(&mut buf);
            let _ = tx.send(read.ok().map(|_| buf));
        });
        rx.recv_deadline(deadline)
            .ok()
            .flatten()
            .filter(|b| b.len() <= TABLE_MAX_BYTES)
    }

    /// Whether the helper exits by `deadline`, observed without reaping it.
    fn exited_by(child: &Child, deadline: Instant) -> bool {
        use rustix::io::Errno;
        use rustix::process::{WaitId, WaitIdOptions, waitid};
        let Some(p) = pid(child.id()) else {
            return false;
        };
        let opts = WaitIdOptions::EXITED | WaitIdOptions::NOWAIT | WaitIdOptions::NOHANG;
        loop {
            match waitid(WaitId::Pid(p), opts) {
                Ok(Some(s)) if s.exited() || s.killed() || s.dumped() => return true,
                Err(Errno::INTR) => {}
                Ok(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                _ => return false,
            }
        }
    }

    /// Parse `ps -A -o pid=,ppid=,pgid=,stat=,lstart=` output, all or nothing.
    pub(super) fn parse_table(out: &[u8]) -> Option<HashMap<u32, Entry>> {
        let text = std::str::from_utf8(out).ok()?;
        let mut table = HashMap::new();
        for line in text.lines() {
            let entry = parse_row(line)?;
            if table.insert(entry.pid, entry).is_some() {
                return None;
            }
        }
        (!table.is_empty()).then_some(table)
    }

    /// One row: three ids, a state and a C-locale `lstart` such as `Sat Nov 14 23:00:07 2026`.
    fn parse_row(line: &str) -> Option<Entry> {
        let fields: Vec<&str> = line.split_ascii_whitespace().collect();
        let [own, parent, group, state, lstart @ ..] = &fields[..] else {
            return None;
        };
        let [wday, month, day, clock, year] = lstart[..] else {
            return None;
        };
        let word = |s: &str| s.len() == 3 && s.bytes().all(|b| b.is_ascii_alphabetic());
        let digits = |s: &str, n: std::ops::RangeInclusive<usize>| {
            n.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
        };
        let clock_ok = clock.len() == 8
            && clock.bytes().enumerate().all(|(i, b)| {
                if i % 3 == 2 {
                    b == b':'
                } else {
                    b.is_ascii_digit()
                }
            });
        let state_ok =
            (1..=8).contains(&state.len()) && state.bytes().all(|b| b.is_ascii_graphic());
        let when_ok =
            word(wday) && word(month) && digits(day, 1..=2) && clock_ok && digits(year, 4..=4);
        if !(state_ok && when_ok) {
            return None;
        }
        Some(Entry {
            pid: id(own).filter(|&p| p > 0)?,
            ppid: id(parent)?,
            pgid: id(group)?,
            stat: (*state).to_owned(),
            start: lstart.join(" "),
        })
    }

    /// A process id in decimal digits only, within the range of `pid_t`.
    fn id(s: &str) -> Option<u32> {
        if s.is_empty() || s.len() > 10 || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse::<u32>().ok().filter(|&v| i32::try_from(v).is_ok())
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_newline_and_tab_and_replaces_escapes() {
        let got = sanitize_display("a\nb\tc\u{1b}[31m\u{9b}\u{7f}\u{202e}x");
        assert_eq!(got, "a\nb\tc\u{FFFD}[31m\u{FFFD}\u{FFFD}\u{FFFD}x");
    }

    #[cfg(unix)]
    mod helper {
        use std::path::Path;
        use std::time::{Duration, Instant};

        use super::super::unix::{Entry, TABLE_MAX_BYTES, parse_table, run_bounded};

        fn sh(script: &str, within: Duration) -> Option<Vec<u8>> {
            run_bounded(
                Path::new("/bin/sh"),
                &["-c", script],
                Instant::now() + within,
            )
        }

        #[test]
        fn the_helper_sees_only_the_fixed_environment() {
            let out = run_bounded(
                Path::new("/usr/bin/env"),
                &[] as &[&str],
                Instant::now() + Duration::from_secs(10),
            )
            .unwrap();
            let mut lines: Vec<&str> = std::str::from_utf8(&out).unwrap().lines().collect();
            lines.sort_unstable();
            assert_eq!(lines, ["LC_ALL=C", "PATH=/usr/bin:/bin"]);
        }

        #[test]
        fn output_up_to_the_limit_is_kept_and_one_byte_more_is_refused() {
            let at = sh(
                &format!("head -c {TABLE_MAX_BYTES} /dev/zero"),
                Duration::from_secs(20),
            );
            let over = sh(
                &format!("head -c {} /dev/zero", TABLE_MAX_BYTES + 1),
                Duration::from_secs(20),
            );
            assert_eq!(at.map(|b| b.len()), Some(TABLE_MAX_BYTES));
            assert_eq!(over, None);
        }

        #[test]
        fn endless_output_is_cut_off_at_the_limit_before_the_deadline() {
            let started = Instant::now();
            let got = sh("exec yes", Duration::from_secs(60));
            assert_eq!(got, None);
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "{:?}",
                started.elapsed()
            );
        }

        #[test]
        fn a_helper_that_fails_yields_nothing() {
            assert_eq!(sh("echo partial; exit 3", Duration::from_secs(10)), None);
        }

        /// Whether `pid` still runs: a killed process may stay a zombie
        /// until its new parent reaps it, so poll for up to ten seconds.
        fn still_running(pid: &str) -> bool {
            let end = Instant::now() + Duration::from_secs(10);
            loop {
                let out = std::process::Command::new("/bin/ps")
                    .args(["-o", "stat=", "-p", pid])
                    .output()
                    .unwrap();
                let stat = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                if stat.is_empty() || stat.starts_with('Z') {
                    return false;
                }
                if Instant::now() > end {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        /// Run `script` with a 500 ms deadline; it writes a pid to the file
        /// named by `$0`. Returns the output, the time taken and that pid.
        fn hung(script: &str) -> (Option<Vec<u8>>, Duration, String) {
            let dir = std::env::temp_dir().join(format!(
                "slopvac-hung-{}-{}",
                std::process::id(),
                script.len()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let pidfile = dir.join("pid");
            let started = Instant::now();
            let got = run_bounded(
                Path::new("/bin/sh"),
                &[
                    std::ffi::OsStr::new("-c"),
                    script.as_ref(),
                    pidfile.as_os_str(),
                ],
                started + Duration::from_millis(500),
            );
            let took = started.elapsed();
            let pid = std::fs::read_to_string(&pidfile).unwrap().trim().to_owned();
            std::fs::remove_dir_all(&dir).unwrap();
            (got, took, pid)
        }

        #[test]
        fn a_hung_helper_and_its_children_are_killed_at_the_deadline() {
            let (got, took, child) = hung(r#"sleep 60 & echo $! > "$0"; wait"#);
            assert_eq!(got, None);
            assert!(took < Duration::from_secs(30), "{took:?}");
            assert!(
                !still_running(&child),
                "the helper's child {child} survived"
            );
        }

        #[test]
        fn a_hung_helper_that_left_its_group_is_killed_at_the_deadline() {
            // The helper leads its own group, so it moves into the host's.
            let script = r#"echo $$ > "$0"; exec /usr/bin/perl -e 'setpgrp(0, getpgrp(getppid())) or die; sleep 60'"#;
            let (got, took, helper) = hung(script);
            assert_eq!(got, None);
            assert!(took < Duration::from_secs(30), "{took:?}");
            assert!(!still_running(&helper), "the helper {helper} survived");
        }

        const ROW: &str = "Sat Nov 14 23:00:07 2026";

        #[test]
        fn a_well_formed_table_parses() {
            let text = format!(
                "    1     0     1 Ss   {ROW}\n  42     1    40 T    Sat Nov  4 01:02:03 2026\n"
            );
            let t = parse_table(text.as_bytes()).unwrap();
            assert_eq!(
                t[&42],
                Entry {
                    pid: 42,
                    ppid: 1,
                    pgid: 40,
                    stat: "T".into(),
                    start: "Sat Nov 4 01:02:03 2026".into(),
                }
            );
            assert_eq!(t.len(), 2);
        }

        #[test]
        fn a_table_with_any_malformed_row_is_refused() {
            let good = format!("1 0 1 Ss {ROW}");
            for bad in [
                format!("+5 1 1 S {ROW}"),
                format!("-5 1 1 S {ROW}"),
                format!("0 1 1 S {ROW}"),
                format!("5 1 1 S {ROW} extra"),
                "5 1 1 S Sat Nov 14 23:00:07".to_owned(),
                "5 1 1 S Sat Nov 14 23-00-07 2026".to_owned(),
                format!("4294967296 1 1 S {ROW}"),
                format!("2147483648 1 1 S {ROW}"),
                format!("1 0 1 S {ROW}"),
                String::new(),
            ] {
                let text = format!("{good}\n{bad}\n");
                assert!(parse_table(text.as_bytes()).is_none(), "{bad:?}");
            }
            assert!(parse_table(b"").is_none());
            assert!(parse_table(b"1 0 1 S Sat Nov 14 23:00:07 2026\n\xff\n").is_none());
        }
    }
}
