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
//! reaped and the result is an error, never a partial diff. A helper thread whose pipe a
//! grandchild still holds open is left to finish by itself.

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
pub struct Limits {
    /// The most standard output bytes accepted.
    pub max_stdout: usize,
    /// The most standard error bytes kept for an error message. The rest is dropped.
    pub max_stderr: usize,
    /// The longest a git call may run.
    pub timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_stdout: 256 * 1024 * 1024,
            max_stderr: 1024 * 1024,
            timeout: Duration::from_secs(120),
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
pub(super) fn run(cwd: &Path, args: &[&str], limits: &Limits) -> Result<Vec<u8>, DiffError> {
    let mut command = Command::new("git");
    command.args(GLOBAL_ARGS).args(args);
    command.current_dir(cwd);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    scrub_environment(&mut command);
    command.env("GIT_NO_LAZY_FETCH", "1");

    let mut child = command.spawn().map_err(DiffError::Spawn)?;
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

/// Kill the child and wait for it, so no zombie is left.
fn stop(child: &mut Child) {
    // `kill` fails only when the child already ended; `wait` then reaps it.
    let _ = child.kill();
    let _ = child.wait();
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

/// Remove every `GIT_*` variable from the child's environment.
fn scrub_environment(command: &mut Command) {
    let names: Vec<OsString> = std::env::vars_os()
        .map(|(name, _)| name)
        .filter(|name| is_git_variable(name))
        .collect();
    for name in names {
        command.env_remove(name);
    }
}

fn is_git_variable(name: &OsStr) -> bool {
    // Windows treats variable names case-insensitively.
    name.to_string_lossy()
        .to_ascii_uppercase()
        .starts_with("GIT_")
}
