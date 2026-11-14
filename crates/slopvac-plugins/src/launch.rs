//! The plugin launcher: resource limits and, on Linux, a child subreaper
//! (threat model P5, P6 and NEW-3).
//!
//! A limit for a child process has to be set between `fork` and `exec`, and
//! the standard library offers that step only through the `unsafe`
//! `pre_exec`, which this workspace forbids. So on Unix the host starts the
//! launcher, a mode of the `slopvac` binary itself, and the launcher sets the
//! limits on itself with `setrlimit` before the plugin starts. The plugin
//! inherits them and cannot raise them: each hard limit is set too.
//!
//! - CPU time: SIGXCPU at the soft limit, SIGKILL one second later.
//! - Open files, and file size.
//! - Core files: none.
//! - Memory, on Linux only: the data limit, which covers every private
//!   writable mapping. macOS enforces no memory limit (residual R2).
//!
//! No process limit is set. `RLIMIT_NPROC` counts every process of the user,
//! not the plugin's, so any value either does nothing or breaks the plugin on
//! a busy machine. The process group kill and the wall-clock cap end a fork
//! loop (R2).
//!
//! On Linux the launcher also marks itself a child subreaper and stays as the
//! parent of the plugin. A descendant whose parent exits is reparented to the
//! launcher rather than to init, so the host's walk still finds it below the
//! stopped group leader. When the plugin exits, the launcher kills and reaps
//! what is left below it, then exits with the plugin's status: a descendant
//! reparented to init after that would be out of every walk's reach. If the
//! cleanup cannot account for every child within two seconds, the launcher
//! kills itself with SIGKILL instead, and the host reports its teardown
//! incomplete. A plugin that kills the launcher looks the same. Elsewhere
//! the launcher replaces itself with the plugin, which keeps its process id.
//!
//! The launcher runs with an empty environment. The plugin's variables reach
//! it under a prefix and it passes them on without the prefix, so a variable
//! such as `LD_PRELOAD` that a manifest names applies to the plugin and never
//! runs code in the launcher before the limits are set.

use std::ffi::{OsStr, OsString};
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
#[cfg(unix)]
use std::process::Command;

/// The first argument that selects launcher mode.
pub const FLAG: &str = "--slopvac-plugin-launcher";
/// Prefix of the variables that carry the plugin's environment.
#[cfg(unix)]
const ENV_PREFIX: &str = "SLOPVAC_PLUGIN_ENV_";

/// Which program the host starts as the launcher.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Launcher {
    /// The running executable, which must call [`intercept`] first thing in
    /// `main`. Production uses this.
    // STUB(slopvac-8l4.9.24): the `slopvac` binary does not call `intercept`
    // yet, so a plugin it starts this way fails to start.
    #[default]
    CurrentExe,
    /// Another program that calls [`intercept`], such as this package's
    /// `slopvac-plugin-launcher`.
    Program(PathBuf),
}

impl Launcher {
    /// The absolute path of the program.
    #[cfg(unix)]
    pub(crate) fn path(&self) -> std::io::Result<PathBuf> {
        match self {
            Self::CurrentExe => std::env::current_exe(),
            Self::Program(p) => Ok(p.clone()),
        }
    }
}

/// The limits the launcher sets, from the host's [`RunCaps`](crate::RunCaps).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// CPU time in whole seconds, rounded up.
    pub(crate) cpu_secs: u64,
    /// Open files.
    pub(crate) open_files: u64,
    /// Largest file the plugin may write, in bytes.
    pub(crate) file_size: u64,
    /// Data limit in bytes, set on Linux only.
    pub(crate) memory: u64,
}

/// What a launcher invocation asks for.
#[derive(Debug, PartialEq, Eq)]
struct Plan {
    limits: Limits,
    program: PathBuf,
    args: Vec<OsString>,
}

/// The host's command that starts `program` through the launcher at
/// `launcher` under `limits`, with `env` as the plugin's whole environment.
#[cfg(unix)]
pub(crate) fn command<'a>(
    launcher: &Path,
    limits: Limits,
    program: &Path,
    args: &[String],
    env: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Command {
    let mut cmd = Command::new(launcher);
    cmd.arg(FLAG)
        .arg(format!("cpu={}", limits.cpu_secs))
        .arg(format!("nofile={}", limits.open_files))
        .arg(format!("fsize={}", limits.file_size))
        .arg(format!("data={}", limits.memory))
        .arg("--")
        .arg(program)
        .args(args)
        .env_clear();
    for (k, v) in env {
        cmd.env(format!("{ENV_PREFIX}{k}"), v);
    }
    cmd
}

/// Run as the plugin launcher if the first argument is [`FLAG`], and never
/// return then. Otherwise return at once.
///
/// A binary that hosts plugins calls this first thing in `main`, before it
/// reads its arguments, its environment or any configuration. On a setup
/// failure the launcher prints one line to standard error and exits with
/// status 127, so the plugin fails to start.
pub fn intercept() {
    let mut argv = std::env::args_os();
    let _ = argv.next();
    if argv.next().as_deref() != Some(OsStr::new(FLAG)) {
        return;
    }
    let rest: Vec<OsString> = argv.collect();
    match parse(&rest) {
        Ok(plan) => run(&plan),
        Err(m) => fail(&m),
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("slopvac plugin launcher: {msg}");
    std::process::exit(127)
}

/// Parse `cpu=N nofile=N fsize=N data=N -- PROGRAM ARGS...`, all or nothing.
fn parse(args: &[OsString]) -> Result<Plan, String> {
    let [cpu, nofile, fsize, data, sep, program, rest @ ..] = args else {
        return Err("malformed arguments".into());
    };
    let num = |a: &OsString, key: &str| {
        a.to_str()
            .and_then(|s| s.strip_prefix(key))
            .and_then(|s| s.strip_prefix('='))
            .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| format!("malformed `{key}` argument"))
    };
    let limits = Limits {
        cpu_secs: num(cpu, "cpu")?,
        open_files: num(nofile, "nofile")?,
        file_size: num(fsize, "fsize")?,
        memory: num(data, "data")?,
    };
    if sep != "--" {
        return Err("malformed arguments".into());
    }
    let program = PathBuf::from(program);
    // An absolute path is never looked up through `PATH`.
    if !program.is_absolute() {
        return Err("the plugin program is not an absolute path".into());
    }
    Ok(Plan {
        limits,
        program,
        args: rest.to_vec(),
    })
}

#[cfg(not(unix))]
fn run(_plan: &Plan) -> ! {
    fail("the launcher runs on Unix only")
}

#[cfg(unix)]
fn run(plan: &Plan) -> ! {
    if let Err(m) = apply(&plan.limits) {
        fail(&m);
    }
    let mut cmd = Command::new(&plan.program);
    cmd.args(&plan.args).env_clear();
    for (k, v) in std::env::vars_os() {
        if let Some(name) = k.to_str().and_then(|k| k.strip_prefix(ENV_PREFIX)) {
            cmd.env(name, v);
        }
    }
    start(cmd)
}

/// Set the limits on this process, soft and hard, never above a hard limit
/// already in place.
#[cfg(unix)]
fn apply(l: &Limits) -> Result<(), String> {
    use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
    let set = |resource: Resource, name: &str, soft: u64, hard: u64| {
        let (soft, hard) = capped(soft, hard, getrlimit(resource).maximum);
        let new = Rlimit {
            current: Some(soft),
            maximum: Some(hard),
        };
        setrlimit(resource, new).map_err(|e| format!("cannot set the {name} limit: {e}"))
    };
    set(Resource::Core, "core size", 0, 0)?;
    set(
        Resource::Cpu,
        "CPU time",
        l.cpu_secs,
        l.cpu_secs.saturating_add(1),
    )?;
    set(Resource::Nofile, "open file", l.open_files, l.open_files)?;
    set(Resource::Fsize, "file size", l.file_size, l.file_size)?;
    #[cfg(target_os = "linux")]
    set(Resource::Data, "memory", l.memory, l.memory)?;
    Ok(())
}

/// The soft and hard values to set: the wanted ones, lowered to the hard
/// limit already in place (`None`: unlimited), which no process can raise.
#[cfg(any(unix, test))]
fn capped(soft: u64, hard: u64, ceiling: Option<u64>) -> (u64, u64) {
    let cap = |v: u64| ceiling.map_or(v, |c| v.min(c));
    (cap(soft), cap(hard))
}

/// Become the plugin; its process id and group stay the launcher's.
#[cfg(all(unix, not(target_os = "linux")))]
fn start(mut cmd: Command) -> ! {
    use std::os::unix::process::CommandExt;
    let err = cmd.exec();
    fail(&format!("cannot run the plugin: {err}"))
}

/// Start the plugin as a child, reap until it exits, clear what is left
/// below the launcher, then exit as the plugin did.
#[cfg(target_os = "linux")]
fn start(mut cmd: Command) -> ! {
    use rustix::process::{getpid, set_child_subreaper};
    if let Err(e) = set_child_subreaper(Some(getpid())) {
        fail(&format!("cannot become a child subreaper: {e}"));
    }
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => fail(&format!("cannot run the plugin: {e}")),
    };
    let plugin = rustix::process::Pid::from_child(&child);
    // The launcher reaps through `wait`; dropping the handle neither waits nor kills.
    drop(child);
    linux::release_stdio();
    let status = linux::reap(plugin);
    if !linux::sweep(linux::SWEEP) {
        // Something below the launcher survived the cleanup, or could not be
        // seen. Dying of SIGKILL with no kill from the host tells the host
        // that its teardown is incomplete.
        let _ = rustix::process::kill_process(getpid(), rustix::process::Signal::KILL);
    }
    linux::relay(status)
}

#[cfg(target_os = "linux")]
mod linux {
    //! The supervising launcher.
    //!
    //! The launcher is single-threaded, and only it reaps its children: it
    //! never ignores `SIGCHLD`, so the kernel reaps none of them on its
    //! behalf. A process that a scan shows as its child therefore keeps its
    //! id until the launcher reaps it, and the launcher signals such a
    //! process by id only between the scan and its next `wait`.

    use std::time::{Duration, Instant};

    use rustix::io::Errno;
    use rustix::process::{Pid, Signal, WaitOptions, WaitStatus, getpid, kill_process, wait};

    use super::fail;

    /// How long the launcher spends killing what is left below it.
    pub(super) const SWEEP: Duration = Duration::from_secs(2);

    /// Point the launcher's own standard streams at `/dev/null`, so that
    /// only the plugin and its descendants hold the host's pipes.
    pub(super) fn release_stdio() {
        let Ok(null) = std::fs::File::options()
            .read(true)
            .write(true)
            .open("/dev/null")
        else {
            return;
        };
        let _ = rustix::stdio::dup2_stdin(&null);
        let _ = rustix::stdio::dup2_stdout(&null);
        let _ = rustix::stdio::dup2_stderr(&null);
    }

    /// Reap children until the plugin exits, and return its status.
    /// Descendants reparented to the launcher that exit first are reaped too.
    pub(super) fn reap(plugin: Pid) -> WaitStatus {
        loop {
            match wait(WaitOptions::empty()) {
                Ok(Some((pid, status))) if pid == plugin => return status,
                Ok(_) | Err(Errno::INTR) => {}
                Err(e) => fail(&format!("lost the plugin: {e}")),
            }
        }
    }

    /// Kill and reap every child of the launcher until none is left, and
    /// return whether that happened within `within`. Each killed child's own
    /// children are reparented to the launcher, so the next round reaches
    /// them. Only `ECHILD` ends the sweep early: a child that the scan cannot
    /// read or the kill cannot reach keeps it going until the deadline.
    pub(super) fn sweep(within: Duration) -> bool {
        let me = getpid();
        let end = Instant::now() + within;
        loop {
            loop {
                match wait(WaitOptions::NOHANG) {
                    Ok(Some(_)) | Err(Errno::INTR) => {}
                    Ok(None) => break,
                    Err(Errno::CHILD) => return true,
                    Err(_) => return false,
                }
            }
            if Instant::now() >= end {
                return false;
            }
            // No `wait` runs between the scan and the kills.
            for pid in children(me) {
                let _ = kill_process(pid, Signal::KILL);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The children of `me`, from the parent ids in `/proc`.
    fn children(me: Pid) -> Vec<Pid> {
        let me = me.as_raw_nonzero().get().unsigned_abs();
        let Ok(dir) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok())
            .filter(|&p| crate::process::proc_ppid(p).is_ok_and(|parent| parent == me))
            .filter_map(|p| i32::try_from(p).ok().and_then(Pid::from_raw))
            .collect()
    }

    /// Exit as the plugin did: with its code, or by its signal. A signal that
    /// does not end the launcher, such as `SIGPIPE`, which the Rust runtime
    /// ignores, becomes the exit code 128 plus its number.
    pub(super) fn relay(status: WaitStatus) -> ! {
        if let Some(code) = status.exit_status() {
            std::process::exit(code);
        }
        if let Some(sig) = status.terminating_signal() {
            if let Some(s) = Signal::from_named_raw(sig) {
                let _ = kill_process(getpid(), s);
            }
            std::process::exit(128 + sig);
        }
        std::process::exit(127)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn a_well_formed_invocation_parses() {
        let got = parse(&os(&[
            "cpu=5",
            "nofile=64",
            "fsize=10",
            "data=20",
            "--",
            "/bin/p",
            "a",
            "--b",
        ]));
        assert_eq!(
            got,
            Ok(Plan {
                limits: Limits {
                    cpu_secs: 5,
                    open_files: 64,
                    file_size: 10,
                    memory: 20,
                },
                program: "/bin/p".into(),
                args: os(&["a", "--b"]),
            })
        );
    }

    #[test]
    fn a_limit_never_goes_above_the_hard_limit_already_in_place() {
        // (wanted soft, wanted hard, ceiling, set)
        for (soft, hard, ceiling, want) in [
            (5, 6, None, (5, 6)),
            (5, 6, Some(100), (5, 6)),
            (5, 6, Some(6), (5, 6)),
            (5, 6, Some(5), (5, 5)),
            (5, 6, Some(3), (3, 3)),
        ] {
            assert_eq!(capped(soft, hard, ceiling), want, "{ceiling:?}");
        }
    }

    #[test]
    fn a_malformed_invocation_is_refused() {
        for bad in [
            &["cpu=5", "nofile=64", "fsize=10", "data=20", "--"][..],
            &["cpu=5", "nofile=64", "fsize=10", "data=20", "-", "/bin/p"],
            &["cpu=+5", "nofile=64", "fsize=10", "data=20", "--", "/bin/p"],
            &["cpu=", "nofile=64", "fsize=10", "data=20", "--", "/bin/p"],
            &["nofile=64", "cpu=5", "fsize=10", "data=20", "--", "/bin/p"],
            &["cpu=5", "nofile=64", "fsize=10", "data=20", "--", "p"],
        ] {
            assert!(parse(&os(bad)).is_err(), "{bad:?}");
        }
    }
}
