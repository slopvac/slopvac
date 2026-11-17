//! The hardened `git` subprocess.
//!
//! # Trust model
//!
//! Git configuration is trusted as the user's own: the system and global files, and the
//! repository's `.git/config` with its includes. A clone creates that file locally and
//! never takes it from the remote. Git's `safe.directory` check, which refuses a
//! repository owned by another user, stays on; nothing here sets it.
//!
//! Tracked content is not trusted: file contents, `.gitattributes` and `.gitmodules`
//! come from whoever wrote the branch. So everything a tracked file could select is
//! closed where it costs nothing, and what the user configured keeps working:
//!
//! - **Closed.** A diff driver, an external diff program and a textconv program are
//!   chosen by attributes. `--no-ext-diff` and `--no-textconv` stop them, and
//!   `-c core.fsmonitor=false` stops the fsmonitor hook. No pager, editor or hook runs,
//!   and no network is used: `GIT_NO_LAZY_FETCH=1` stops a partial clone from fetching a
//!   missing object, and a missing object is an error ([`DiffError::MissingObject`]).
//! - **Open, on purpose.** A clean or smudge filter named in the user's git config, such
//!   as `git-lfs`, runs when git compares the work tree to the index, if
//!   `.gitattributes` selects it. Turning that off would make an LFS repository report
//!   every pointer file as changed. The filter command comes from trusted config; only
//!   the selection comes from tracked content.
//!
//! # Invocation
//!
//! - argv is fixed and never goes through a shell. The only caller-supplied text is the
//!   base revision. It is resolved to a full object id first, and only that id reaches
//!   `diff`; a name that starts with `-` is refused so it cannot be read as an option.
//! - `--no-pager` and `--no-color` keep output plain, `--no-optional-locks` keeps a
//!   read-only query from writing the index, and `-c core.quotepath=off`, the pinned
//!   prefixes, `--inter-hunk-context=0` and `diff.relative=false` keep the patch format
//!   independent of user configuration.
//! - The child gets no `GIT_*` variable inherited from the caller, so `GIT_DIR`,
//!   `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_CONFIG_*` and the rest cannot redirect it to
//!   another repository or inject configuration. Then `GIT_NO_LAZY_FETCH=1` is set.
//! - The `diff` call ends with `--` and names no path.
//!
//! # Limits
//!
//! Output is read on helper threads with a hard cap on stdout, and stderr past its cap is
//! dropped. The run has a deadline. When either limit is hit the child is killed and
//! reaped and the result is an error, never a partial diff. On Unix git runs in its own
//! process group and the whole group is killed, so a filter it started does not outlive
//! it. A helper thread whose pipe a survivor still holds open is left to finish by itself.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, bounded};

use super::DiffError;

/// Options placed before every subcommand.
const GLOBAL_ARGS: [&str; 8] = [
    "--no-pager",
    "--no-optional-locks",
    "-c",
    "core.quotepath=off",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "diff.relative=false",
];

/// Options for `git diff` that pin the patch format and switch off program hooks.
pub(super) const DIFF_ARGS: [&str; 10] = [
    "diff",
    "--no-ext-diff",
    "--no-textconv",
    "--no-color",
    "--unified=0",
    "--inter-hunk-context=0",
    "--find-renames",
    "--ignore-submodules=all",
    "--src-prefix=a/",
    "--dst-prefix=b/",
];

/// How much output git may produce and how long it may run, per git call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// The most standard output bytes accepted.
    pub max_stdout: usize,
    /// The most standard error bytes kept for an error message. The rest is dropped.
    pub max_stderr: usize,
    /// The longest a git call may run.
    pub timeout: Duration,
    /// The largest changed file [`super::ChangedLines::ranges_for`] reads, in bytes.
    pub max_file: u64,
}

impl Limits {
    /// The same limits with another stdout cap.
    #[must_use]
    pub const fn with_max_stdout(mut self, bytes: usize) -> Self {
        self.max_stdout = bytes;
        self
    }

    /// The same limits with another stderr cap.
    #[must_use]
    pub const fn with_max_stderr(mut self, bytes: usize) -> Self {
        self.max_stderr = bytes;
        self
    }

    /// The same limits with another time limit.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The same limits with another changed-file size cap.
    #[must_use]
    pub const fn with_max_file(mut self, bytes: u64) -> Self {
        self.max_file = bytes;
        self
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_stdout: 256 * 1024 * 1024,
            max_stderr: 1024 * 1024,
            timeout: Duration::from_secs(120),
            max_file: 64 * 1024 * 1024,
        }
    }
}

/// How to run git: the program and the limits.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Options {
    /// The limits for each git call and for reading changed files.
    pub limits: Limits,
    /// The program to run. This is `git`; a test points it at a stand-in.
    pub program: OsString,
}

impl Options {
    /// The same options with other limits.
    #[must_use]
    pub const fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// The same options with another program.
    #[must_use]
    pub fn with_program(mut self, program: impl Into<OsString>) -> Self {
        self.program = program.into();
        self
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            program: OsString::from("git"),
        }
    }
}

/// How often the wait loop looks at the child and the clock.
const POLL: Duration = Duration::from_millis(5);
/// How long to wait for a reader thread after the child ended or was killed.
const DRAIN_GRACE: Duration = Duration::from_secs(5);

/// Run `git` with the hardened argument set and return stdout.
///
/// `args` follow the global options; `cwd` is the directory git starts in.
pub(super) fn run(cwd: &Path, args: &[&str], options: &Options) -> Result<Vec<u8>, DiffError> {
    let limits = &options.limits;
    let mut command = Command::new(&options.program);
    command.args(GLOBAL_ARGS).args(args);
    command.current_dir(plain_path(cwd));
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_environment(&mut command);
    // Its own process group, so a limit can stop git and the filters it started.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);

    let mut child = command.spawn().map_err(DiffError::Spawn)?;
    let group = child.id();
    let exceeded = Arc::new(AtomicBool::new(false));
    let stdout = spawn_reader(
        child.stdout.take(),
        limits.max_stdout,
        Some(Arc::clone(&exceeded)),
    );
    let stderr = spawn_reader(child.stderr.take(), limits.max_stderr, None);

    let deadline = Instant::now() + limits.timeout;
    let status = loop {
        if Instant::now() >= deadline {
            stop(&mut child);
            return Err(DiffError::TimedOut(limits.timeout));
        }
        if exceeded.load(Ordering::Acquire) {
            stop(&mut child);
            return Err(DiffError::OutputTooLarge(limits.max_stdout));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(POLL),
            Err(e) => {
                stop(&mut child);
                return Err(DiffError::Spawn(e));
            }
        }
    };

    // The child ended and is reaped. Whatever it left in its group is stopped before the
    // result is chosen, so no outcome, including a late overflow, leaves a survivor.
    kill_group(group);
    let out = stdout.recv_timeout(DRAIN_GRACE).ok().flatten();
    let err = stderr.recv_timeout(DRAIN_GRACE).ok().flatten();
    if exceeded.load(Ordering::Acquire) {
        return Err(DiffError::OutputTooLarge(limits.max_stdout));
    }
    if status.success() {
        out.ok_or_else(|| DiffError::Git("git output could not be read".to_owned()))
    } else {
        let text = String::from_utf8_lossy(&err.unwrap_or_default())
            .trim()
            .to_owned();
        Err(classify(text))
    }
}

/// Kill the child and its process group, and wait for it, so no zombie is left.
fn stop(child: &mut Child) {
    kill_group(child.id());
    // `kill` fails only when the child already ended; `wait` then reaps it.
    let _ = child.kill();
    let _ = child.wait();
}

/// Kill every process in the group that git leads. Git is its own group leader, so the
/// group id is its pid, and the group lives as long as any member does. On other
/// platforms there is no group and this does nothing.
fn kill_group(leader: u32) {
    #[cfg(unix)]
    if let Some(group) = i32::try_from(leader)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    {
        // The group may already be gone.
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    }
    #[cfg(not(unix))]
    let _ = leader;
}

/// Read a pipe on a helper thread. `Some(flag)` marks a pipe whose overflow is an error;
/// without it, bytes past the cap are read and dropped so the child never blocks.
fn spawn_reader<R: Read + Send + 'static>(
    pipe: Option<R>,
    cap: usize,
    overflow: Option<Arc<AtomicBool>>,
) -> Receiver<Option<Vec<u8>>> {
    let (tx, rx) = bounded(1);
    thread::spawn(move || {
        let Some(mut pipe) = pipe else {
            let _ = tx.send(None);
            return;
        };
        let mut kept = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let room = cap.saturating_sub(kept.len());
                    kept.extend_from_slice(&chunk[..n.min(room)]);
                    if n > room
                        && let Some(flag) = &overflow
                    {
                        flag.store(true, Ordering::Release);
                        break;
                    }
                }
            }
        }
        let _ = tx.send(Some(kept));
    });
    rx
}

/// Turn git's standard error into a typed error where the cause is known.
fn classify(stderr: String) -> DiffError {
    // With lazy fetch off, a promisor object that is not local cannot be read.
    let missing = [
        "unable to read",
        "could not fetch",
        "promisor",
        "bad object",
    ];
    if missing.iter().any(|m| stderr.contains(m)) {
        DiffError::MissingObject(stderr)
    } else {
        DiffError::Git(stderr)
    }
}

/// What to do to the child's environment, as `(name, value)`: `None` removes the
/// variable, `Some` sets it. Applied in order.
type EnvEdit = (OsString, Option<&'static str>);

/// The environment changes for a child, given the names the parent has.
///
/// Only `GIT_*` variables are removed. Everything else stays, because git needs the
/// platform's own variables to find its configuration and temp space: `HOME` and `XDG_*`
/// on Unix, and `USERPROFILE`, `HOMEDRIVE`, `HOMEPATH`, `APPDATA`, `LOCALAPPDATA`,
/// `PROGRAMDATA`, `SystemRoot`, `TEMP` and `TMP` on Windows. `GIT_NO_LAZY_FETCH=1` comes
/// last, so it wins over an inherited value.
fn env_edits(inherited: impl IntoIterator<Item = OsString>) -> Vec<EnvEdit> {
    let mut edits: Vec<EnvEdit> = inherited
        .into_iter()
        .filter(|name| is_git_variable(name))
        .map(|name| (name, None))
        .collect();
    edits.push((OsString::from("GIT_NO_LAZY_FETCH"), Some("1")));
    edits
}

fn apply_environment(command: &mut Command) {
    for (name, value) in env_edits(std::env::vars_os().map(|(name, _)| name)) {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
}

fn is_git_variable(name: &OsStr) -> bool {
    // Windows treats variable names case-insensitively.
    name.to_string_lossy()
        .to_ascii_uppercase()
        .starts_with("GIT_")
}

/// A directory path git can use as its working directory.
///
/// `fs::canonicalize` on Windows returns a `\\?\C:\...` verbatim path. Git for Windows
/// does not accept that form for a working directory or a config path, so the prefix is
/// dropped when the rest is an ordinary drive path. UNC and other forms are left alone.
fn plain_path(path: &Path) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            let bytes = rest.as_bytes();
            if bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && bytes[2] == b'\\'
            {
                return std::path::PathBuf::from(rest);
            }
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn only_git_variables_are_removed_and_the_lazy_fetch_switch_is_last() {
        let inherited = names(&[
            "GIT_DIR",
            "git_work_tree",
            "GIT_CONFIG_COUNT",
            "GIT_NO_LAZY_FETCH",
            "PATH",
            "HOME",
            "USERPROFILE",
            "HOMEDRIVE",
            "HOMEPATH",
            "APPDATA",
            "LOCALAPPDATA",
            "PROGRAMDATA",
            "SystemRoot",
            "TEMP",
            "TMP",
            "XDG_CONFIG_HOME",
            "GITHUB_TOKEN",
        ]);

        let edits = env_edits(inherited);

        let removed: Vec<&str> = edits
            .iter()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| name.to_str().unwrap())
            .collect();
        assert_eq!(
            removed,
            [
                "GIT_DIR",
                "git_work_tree",
                "GIT_CONFIG_COUNT",
                "GIT_NO_LAZY_FETCH"
            ]
        );
        assert_eq!(
            edits.last(),
            Some(&(OsString::from("GIT_NO_LAZY_FETCH"), Some("1")))
        );
        assert_eq!(edits.len(), removed.len() + 1, "nothing else is touched");
    }

    #[cfg(windows)]
    #[test]
    fn a_verbatim_drive_path_loses_its_prefix_and_other_forms_stay() {
        let plain = |s: &str| plain_path(Path::new(s)).to_string_lossy().into_owned();
        assert_eq!(plain(r"\\?\C:\Users\x\repo"), r"C:\Users\x\repo");
        assert_eq!(plain(r"\\?\UNC\server\share"), r"\\?\UNC\server\share");
        assert_eq!(plain(r"C:\Users\x"), r"C:\Users\x");
    }
}
