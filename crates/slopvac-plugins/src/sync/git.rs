//! Pinned-commit git sources (threat model F5, K11, N10, E8).
//!
//! # Why a `git` subprocess
//!
//! The fetch needs a git smart-HTTP client with TLS, shallow fetch of one
//! commit by id, and proxy and certificate handling that match what the user's
//! own `git` does. The Rust option is `gix`: at 0.90 it is pre-1.0, pulls in
//! about thirty `gix-*` crates and, for HTTPS, a second HTTP and TLS stack next
//! to `ureq` and rustls (the programme picked one TLS stack). A `git` process
//! costs nothing in the build, is the client every forge tests against, and
//! runs under the same containment as credential helpers ([`Contained`]).
//! Its risks are the ones the threat model lists as N10, and the controls
//! below answer each of them.
//!
//! # What runs, and what cannot
//!
//! - `git` is found on `PATH` by absolute entries only (CWE-427): a relative
//!   `PATH` entry such as `.` can name a program in the project, so it is
//!   skipped, as host helpers are looked up.
//! - The environment is cleared. Only proxy and certificate variables and the
//!   Windows system variables pass through. `HOME` and the config files point
//!   at an empty scratch directory (`GIT_CONFIG_GLOBAL` and `GIT_CONFIG_SYSTEM`
//!   at the null device), so no user or system config, credential helper,
//!   `GIT_*` variable or askpass program applies, and `GIT_TERMINAL_PROMPT=0`
//!   rules out a prompt.
//! - Only `https` is an allowed transport (`GIT_ALLOW_PROTOCOL`,
//!   `protocol.allow=never`, `protocol.https.allow=always`), which also stops
//!   `ext::`, `file::` and an HTTPS-to-HTTP redirect. Hooks point at an empty
//!   directory, `core.fsmonitor` is off and the scratch repository is made
//!   with no templates.
//! - The repository is fresh and bare, so nothing the remote serves can become
//!   local config, and `fsckObjects` is on.
//! - **No checkout happens.** The commit is read with `ls-tree` and
//!   `cat-file`, so attributes, filters, smudge programs, LFS, `.gitmodules`
//!   and `export-subst` have no effect. Symbolic links and submodule entries
//!   are refused, as are modes other than regular and executable files.
//! - The fetch is shallow (`--depth=1`) for exactly the pinned commit, with
//!   `--no-tags` and no submodule recursion. A deadline covers every command and
//!   the whole process group is killed at the deadline, and a watcher kills the
//!   fetch when the objects on disk pass the compressed size limit.
//!
//! The caller compares the resulting tree hash with the lock before anything
//! reaches the store.

use std::ffi::OsStr;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::extract::{ExtractError, Limits, StagedTree};
use crate::oci::{Contained, find_in_path};
use crate::process::sanitize_display;

/// Longest `git` error text kept for a message.
const MAX_STDERR_BYTES: u64 = 4 * 1024;
/// Longest `cat-file --batch` header line.
const MAX_HEADER_BYTES: u64 = 256;
/// How long a `git` process may take to exit after it closed its output.
const POLL: Duration = Duration::from_millis(10);
/// Variables that pass through to `git`. Proxies and certificate stores are the
/// user's network configuration; the rest are what Windows needs to start a process.
const PASS_THROUGH: &[&str] = &[
    "HTTPS_PROXY",
    "https_proxy",
    "NO_PROXY",
    "no_proxy",
    "ALL_PROXY",
    "all_proxy",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "SystemRoot",
    "windir",
    "TEMP",
    "TMP",
];

/// Why a git fetch or export failed.
#[derive(Debug)]
pub(super) enum GitError {
    /// `git` failed, or the commit is not acceptable. The text is safe to show.
    Failed(String),
    /// Writing the tree was refused.
    Extract(ExtractError),
}

impl From<ExtractError> for GitError {
    fn from(e: ExtractError) -> Self {
        Self::Extract(e)
    }
}

fn failed(msg: impl Into<String>) -> GitError {
    GitError::Failed(msg.into())
}

/// What to fetch and how far to trust the transport.
pub(super) struct Request<'a> {
    pub(super) git: &'a Path,
    pub(super) url: &'a str,
    pub(super) commit: &'a str,
    /// Test seam: also allow `file://` repositories.
    pub(super) allow_file: bool,
    pub(super) timeout: Duration,
    pub(super) limits: Limits,
}

/// The `git` program: the first on an absolute `PATH` entry.
pub(super) fn find_git() -> Option<PathBuf> {
    find_git_in(&std::env::var_os("PATH")?)
}

fn find_git_in(path: &OsStr) -> Option<PathBuf> {
    let absolute: Vec<PathBuf> = std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .collect();
    find_in_path("git", Some(&std::env::join_paths(absolute).ok()?))
}

/// Fetch exactly `request.commit` into a scratch repository under `scratch`
/// and write its files into `staged`.
pub(super) fn export_commit(
    request: &Request<'_>,
    scratch: &Path,
    staged: &mut StagedTree,
) -> Result<(), GitError> {
    let https = request.url.starts_with("https://");
    let local = request.allow_file && request.url.starts_with("file://");
    if !(https || local) || request.url.starts_with('-') {
        return Err(failed("only https:// repositories are fetched"));
    }
    if request.commit.len() != 40 || !request.commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(failed("the commit is not a full 40-digit hash"));
    }
    let git = Git::prepare(request, scratch)?;
    git.fetch()?;
    git.check_commit()?;
    let entries = git.list_tree()?;
    git.export(&entries, staged)
}

struct Git<'a> {
    request: &'a Request<'a>,
    work: PathBuf,
    repo: PathBuf,
    deadline: Instant,
}

impl<'a> Git<'a> {
    fn prepare(request: &'a Request<'a>, scratch: &Path) -> Result<Self, GitError> {
        let work = scratch.to_path_buf();
        for dir in ["home", "nohooks"] {
            std::fs::create_dir(work.join(dir))
                .map_err(|e| failed(format!("creating a scratch directory: {e}")))?;
        }
        let git = Self {
            request,
            repo: work.join("repo"),
            work,
            deadline: Instant::now() + request.timeout,
        };
        let mut init = git.command();
        init.args(["init", "--quiet", "--bare", "--template="])
            .arg("--")
            .arg(&git.repo);
        git.run(init, "init", None, 4096)?;
        Ok(git)
    }

    /// A `git` command with the scrubbed environment and the fixed config.
    fn command(&self) -> Command {
        let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
        let home = self.work.join("home");
        let mut cmd = Command::new(self.request.git);
        cmd.env_clear().current_dir(&self.work);
        for name in PASS_THROUGH {
            if let Some(value) = std::env::var_os(name) {
                cmd.env(name, value);
            }
        }
        let allowed = if self.request.allow_file {
            "https:file"
        } else {
            "https"
        };
        cmd.env("HOME", &home)
            .env("XDG_CONFIG_HOME", &home)
            .env("USERPROFILE", &home)
            .env("GIT_CONFIG_GLOBAL", null)
            .env("GIT_CONFIG_SYSTEM", null)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ALLOW_PROTOCOL", allowed)
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("LC_ALL", "C");
        let hooks = format!("core.hooksPath={}", self.work.join("nohooks").display());
        let mut config = vec![
            "protocol.allow=never",
            "protocol.https.allow=always",
            "protocol.version=2",
            "core.fsmonitor=false",
            "core.askPass=",
            "credential.helper=",
            "http.sslVerify=true",
            "transfer.fsckObjects=true",
            "fetch.recurseSubmodules=false",
            "gc.auto=0",
            "maintenance.auto=false",
            "fetch.writeCommitGraph=false",
            &hooks,
        ];
        if self.request.allow_file {
            config.push("protocol.file.allow=always");
        }
        for entry in config {
            cmd.arg("-c").arg(entry);
        }
        cmd
    }

    /// A command that works on the scratch repository.
    fn in_repo(&self) -> Command {
        let mut cmd = self.command();
        cmd.arg("--git-dir").arg(&self.repo);
        cmd
    }

    fn run(
        &self,
        cmd: Command,
        what: &str,
        watch: Option<Watch>,
        stdout_cap: usize,
    ) -> Result<Vec<u8>, GitError> {
        let mut job = Supervised::spawn(cmd, self.deadline, watch)?;
        drop(job.stdin.take());
        let mut out = Vec::new();
        if let Some(stdout) = job.stdout.take() {
            stdout
                .take(stdout_cap as u64 + 1)
                .read_to_end(&mut out)
                .map_err(|e| failed(format!("reading `git {what}`: {e}")))?;
        }
        if out.len() > stdout_cap {
            return Err(failed(format!(
                "`git {what}` printed more than {stdout_cap} bytes"
            )));
        }
        job.finish(what)?;
        Ok(out)
    }

    fn fetch(&self) -> Result<(), GitError> {
        let mut cmd = self.in_repo();
        cmd.args([
            "fetch",
            "--quiet",
            "--depth=1",
            "--no-tags",
            "--no-recurse-submodules",
            "--",
        ])
        .arg(self.request.url)
        .arg(self.request.commit);
        let objects = self.repo.join("objects");
        let max = self.request.limits.compressed_bytes;
        let watch = Watch {
            dir: objects.clone(),
            max,
        };
        self.run(cmd, "fetch", Some(watch), 4096)?;
        if dir_size(&objects) > max {
            return Err(failed(format!(
                "the repository download exceeds {max} bytes"
            )));
        }
        Ok(())
    }

    fn check_commit(&self) -> Result<(), GitError> {
        let mut cmd = self.in_repo();
        cmd.args(["rev-parse", "--verify", "--quiet", "--end-of-options"])
            .arg(format!("{}^{{commit}}", self.request.commit));
        let out = self.run(cmd, "rev-parse", None, 256)?;
        if String::from_utf8_lossy(&out).trim() != self.request.commit {
            return Err(failed("the fetched commit is not the pinned commit"));
        }
        Ok(())
    }

    fn list_tree(&self) -> Result<Vec<Entry>, GitError> {
        let mut cmd = self.in_repo();
        cmd.args(["ls-tree", "-r", "-z", "--full-tree", "--end-of-options"])
            .arg(self.request.commit);
        let cap = self.request.limits.entries.saturating_mul(1200) + 1024;
        let out = self.run(cmd, "ls-tree", None, cap)?;
        parse_tree(&out, self.request.limits.entries)
    }

    fn export(&self, entries: &[Entry], staged: &mut StagedTree) -> Result<(), GitError> {
        let mut cmd = self.in_repo();
        cmd.args(["cat-file", "--batch"]);
        let mut job = Supervised::spawn(cmd, self.deadline, None)?;
        let mut stdin = job.stdin.take();
        let request = entries.iter().fold(String::new(), |mut acc, e| {
            acc.push_str(&e.oid);
            acc.push('\n');
            acc
        });
        // The writer runs apart from the reader, or both could block on a full pipe.
        let feeder = thread::spawn(move || {
            if let Some(pipe) = stdin.as_mut() {
                let _ = pipe.write_all(request.as_bytes());
            }
        });
        let stdout = job
            .stdout
            .take()
            .ok_or_else(|| failed("no output from `git cat-file`"))?;
        let mut reader = BufReader::new(stdout);
        let mut remaining = self.request.limits.expanded_bytes;
        let mut result = Ok(());
        for entry in entries {
            result = copy_blob(&mut reader, entry, &mut remaining, staged);
            if result.is_err() {
                break;
            }
        }
        drop(reader);
        let _ = feeder.join();
        result?;
        job.finish("cat-file")
    }
}

/// One file of the commit.
struct Entry {
    path: String,
    oid: String,
    executable: bool,
}

fn parse_tree(out: &[u8], max_entries: usize) -> Result<Vec<Entry>, GitError> {
    let mut entries = Vec::new();
    for record in out.split(|&b| b == 0).filter(|r| !r.is_empty()) {
        if entries.len() >= max_entries {
            return Err(failed(format!(
                "the commit has more than {max_entries} files"
            )));
        }
        let tab = record
            .iter()
            .position(|&b| b == b'\t')
            .ok_or_else(|| failed("`git ls-tree` printed an unexpected line"))?;
        let path = String::from_utf8(record[tab + 1..].to_vec())
            .map_err(|_| failed("a path in the commit is not UTF-8"))?;
        let meta = std::str::from_utf8(&record[..tab])
            .map_err(|_| failed("`git ls-tree` printed an unexpected line"))?;
        let mut fields = meta.split(' ');
        let (Some(mode), Some(kind), Some(oid), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(failed("`git ls-tree` printed an unexpected line"));
        };
        let shown = path.escape_debug();
        let executable = match (mode, kind) {
            ("100644", "blob") => false,
            ("100755", "blob") => true,
            ("120000", _) => return Err(failed(format!("`{shown}` is a symbolic link"))),
            ("160000", _) => return Err(failed(format!("`{shown}` is a submodule"))),
            _ => return Err(failed(format!("`{shown}` has the unsupported mode {mode}"))),
        };
        if oid.len() != 40 || !oid.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(failed("`git ls-tree` printed an unexpected object id"));
        }
        entries.push(Entry {
            path,
            oid: oid.to_owned(),
            executable,
        });
    }
    Ok(entries)
}

/// Read one `cat-file --batch` reply and write the blob into the staged tree.
fn copy_blob(
    reader: &mut impl BufRead,
    entry: &Entry,
    remaining: &mut u64,
    staged: &mut StagedTree,
) -> Result<(), GitError> {
    let mut header = Vec::new();
    reader
        .by_ref()
        .take(MAX_HEADER_BYTES)
        .read_until(b'\n', &mut header)
        .map_err(|e| failed(format!("reading `git cat-file`: {e}")))?;
    let header = header
        .strip_suffix(b"\n")
        .and_then(|h| std::str::from_utf8(h).ok())
        .ok_or_else(|| failed("`git cat-file` printed an unexpected header"))?;
    let mut fields = header.split(' ');
    let (Some(oid), Some("blob"), Some(size), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return Err(failed(format!(
            "object {} is missing or is not a file",
            entry.oid
        )));
    };
    let size: u64 = size
        .parse()
        .map_err(|_| failed("`git cat-file` printed an unexpected size"))?;
    if oid != entry.oid {
        return Err(failed("`git cat-file` answered for another object"));
    }
    if size > *remaining {
        return Err(failed("the commit expands past the size limit"));
    }
    *remaining -= size;
    let mut body = reader.by_ref().take(size);
    staged.write_file(&entry.path, &mut body, entry.executable)?;
    let mut newline = [0u8; 1];
    if body.limit() != 0 || reader.read_exact(&mut newline).is_err() || newline != *b"\n" {
        return Err(failed("`git cat-file` ended early"));
    }
    Ok(())
}

/// A directory whose size is watched while a command runs.
struct Watch {
    dir: PathBuf,
    max: u64,
}

/// Bytes in the files under `dir`, counting at most [`MAX_WATCHED_NODES`] entries.
fn dir_size(dir: &Path) -> u64 {
    const MAX_WATCHED_NODES: usize = 100_000;
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_WATCHED_NODES {
                return u64::MAX;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

const KILLED_TIME: u8 = 1;
const KILLED_SIZE: u8 = 2;

/// A running `git` with a watchdog. The watchdog kills the whole process group
/// at the deadline or when the watched directory passes its limit.
struct Supervised {
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    stderr: Option<JoinHandle<Vec<u8>>>,
    watchdog: Option<JoinHandle<Option<Contained>>>,
    done: Arc<AtomicBool>,
    killed: Arc<AtomicU8>,
    deadline: Instant,
}

impl Supervised {
    fn spawn(mut cmd: Command, deadline: Instant, watch: Option<Watch>) -> Result<Self, GitError> {
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child =
            Contained::spawn(&mut cmd).map_err(|e| failed(format!("cannot start git: {e}")))?;
        let stdin = child.child_mut().stdin.take();
        let stdout = child.child_mut().stdout.take();
        let stderr = child.child_mut().stderr.take();
        let done = Arc::new(AtomicBool::new(false));
        let killed = Arc::new(AtomicU8::new(0));
        let (flag, why) = (Arc::clone(&done), Arc::clone(&killed));
        let watchdog = thread::spawn(move || {
            let mut tick = 0u32;
            loop {
                if flag.load(Ordering::SeqCst) {
                    return Some(child);
                }
                let reason = if Instant::now() >= deadline {
                    KILLED_TIME
                } else if tick.is_multiple_of(4)
                    && watch.as_ref().is_some_and(|w| dir_size(&w.dir) > w.max)
                {
                    KILLED_SIZE
                } else {
                    0
                };
                if reason != 0 {
                    why.store(reason, Ordering::SeqCst);
                    child.kill_tree();
                    return None;
                }
                tick = tick.wrapping_add(1);
                thread::sleep(Duration::from_millis(25));
            }
        });
        let stderr = thread::spawn(move || {
            let mut kept = Vec::new();
            if let Some(mut pipe) = stderr {
                let _ = (&mut pipe).take(MAX_STDERR_BYTES).read_to_end(&mut kept);
                // Keep reading so a chatty child never blocks on a full pipe.
                let _ = io::copy(&mut pipe, &mut io::sink());
            }
            kept
        });
        Ok(Self {
            stdin,
            stdout,
            stderr: Some(stderr),
            watchdog: Some(watchdog),
            done,
            killed,
            deadline,
        })
    }

    /// Stop the watchdog and take the process back, if it was not killed.
    fn stop_watchdog(&mut self) -> Option<Contained> {
        self.done.store(true, Ordering::SeqCst);
        self.watchdog.take()?.join().ok().flatten()
    }

    /// Wait for a normal exit and turn failure into an error that names `what`.
    fn finish(mut self, what: &str) -> Result<(), GitError> {
        drop(self.stdin.take());
        drop(self.stdout.take());
        let child = self.stop_watchdog();
        let why = match self.killed.load(Ordering::SeqCst) {
            KILLED_TIME => Some(format!("`git {what}` timed out")),
            KILLED_SIZE => Some(format!("`git {what}` downloaded more than the size limit")),
            _ => None,
        };
        if let Some(why) = why {
            return Err(failed(why));
        }
        let mut child = child.ok_or_else(|| failed(format!("`git {what}` was stopped")))?;
        let status = loop {
            match child.has_exited() {
                Ok(true) => {
                    break child
                        .reap()
                        .map_err(|e| failed(format!("waiting for `git {what}`: {e}")))?;
                }
                Ok(false) if Instant::now() < self.deadline => thread::sleep(POLL),
                Ok(false) => {
                    child.kill_tree();
                    return Err(failed(format!("`git {what}` timed out")));
                }
                Err(e) => {
                    child.kill_tree();
                    return Err(failed(format!("waiting for `git {what}`: {e}")));
                }
            }
        };
        let text = self
            .stderr
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        check_status(what, status, &text)
    }
}

impl Drop for Supervised {
    fn drop(&mut self) {
        drop(self.stdin.take());
        drop(self.stdout.take());
        if let Some(child) = self.stop_watchdog() {
            child.kill_tree();
        }
    }
}

fn check_status(what: &str, status: ExitStatus, stderr: &[u8]) -> Result<(), GitError> {
    if status.success() {
        return Ok(());
    }
    let text = sanitize_display(String::from_utf8_lossy(stderr).trim());
    let detail = if text.is_empty() {
        String::new()
    } else {
        format!(": {text}")
    };
    Err(failed(format!("`git {what}` failed ({status}){detail}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_path_entries_never_find_git() {
        let dir = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) { "git.exe" } else { "git" };
        std::fs::write(dir.path().join(name), b"").unwrap();
        // The directory is only reachable through a relative entry.
        let relative = std::env::join_paths([Path::new("."), Path::new("sub/bin")]).unwrap();
        assert_eq!(find_git_in(&relative), None);
    }

    #[test]
    fn an_absolute_entry_after_a_relative_one_still_finds_git() {
        let dir = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) { "git.exe" } else { "git" };
        std::fs::write(dir.path().join(name), b"").unwrap();
        let path = std::env::join_paths([Path::new("."), dir.path()]).unwrap();
        assert_eq!(find_git_in(&path), Some(dir.path().join(name)));
    }

    #[test]
    fn tree_listing_refuses_links_and_submodules() {
        let oid = "a".repeat(40);
        for (mode, kind, word) in [
            ("120000", "blob", "symbolic link"),
            ("160000", "commit", "submodule"),
            ("100664", "blob", "unsupported mode"),
        ] {
            let record = format!("{mode} {kind} {oid}\tx/y\0");
            let Err(GitError::Failed(msg)) = parse_tree(record.as_bytes(), 10) else {
                panic!("{mode} was accepted");
            };
            assert!(msg.contains(word), "{msg}");
        }
    }

    #[test]
    fn tree_listing_reads_modes_and_stops_at_the_entry_limit() {
        let oid = "b".repeat(40);
        let two = format!("100644 blob {oid}\ta.txt\0100755 blob {oid}\tbin/run\0");
        let entries = parse_tree(two.as_bytes(), 2).unwrap();
        let got: Vec<(&str, bool)> = entries
            .iter()
            .map(|e| (e.path.as_str(), e.executable))
            .collect();
        assert_eq!(got, vec![("a.txt", false), ("bin/run", true)]);
        assert!(parse_tree(two.as_bytes(), 1).is_err());
    }

    /// What a stand-in `git` sees: its environment lines, then its arguments.
    #[cfg(unix)]
    fn probe() -> (Vec<String>, Vec<String>) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("git");
        std::fs::write(
            &script,
            "#!/bin/sh\nenv\necho --ARGS--\nfor a in \"$@\"; do echo \"$a\"; done\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir_all(work.join("home")).unwrap();
        let request = Request {
            git: &script,
            url: "https://git.test/repo",
            commit: &"a".repeat(40),
            allow_file: false,
            timeout: Duration::from_secs(30),
            limits: Limits::default(),
        };
        let git = Git {
            request: &request,
            repo: work.join("repo"),
            work,
            deadline: Instant::now() + Duration::from_secs(30),
        };
        let out = git.run(git.in_repo(), "probe", None, 1 << 16).unwrap();
        let text = String::from_utf8(out).unwrap();
        let (env, args) = text.split_once("--ARGS--\n").unwrap();
        let lines = |s: &str| s.lines().map(str::to_owned).collect();
        (lines(env), lines(args))
    }

    #[cfg(unix)]
    #[test]
    fn git_runs_with_a_cleared_environment_and_no_user_config() {
        let (env, _) = probe();
        let names: Vec<&str> = env
            .iter()
            .filter_map(|l| l.split_once('=').map(|(k, _)| k))
            .collect();
        let allowed = [
            "HOME",
            "XDG_CONFIG_HOME",
            "USERPROFILE",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_SYSTEM",
            "GIT_CONFIG_NOSYSTEM",
            "GIT_TERMINAL_PROMPT",
            "GIT_ALLOW_PROTOCOL",
            "GIT_NO_REPLACE_OBJECTS",
            "GIT_OPTIONAL_LOCKS",
            "LC_ALL",
            // The shell adds these itself.
            "PWD",
            "SHLVL",
            "_",
            "OLDPWD",
        ];
        let extra: Vec<&&str> = names
            .iter()
            .filter(|n| !allowed.contains(n) && !PASS_THROUGH.contains(n))
            .collect();
        assert!(extra.is_empty(), "unexpected variables: {extra:?}");
        for line in [
            "GIT_TERMINAL_PROMPT=0",
            "GIT_CONFIG_GLOBAL=/dev/null",
            "GIT_ALLOW_PROTOCOL=https",
        ] {
            assert!(env.iter().any(|l| l == line), "{line} missing in {env:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn git_gets_only_https_and_no_hooks_helpers_or_submodules() {
        let (_, args) = probe();
        for want in [
            "protocol.allow=never",
            "protocol.https.allow=always",
            "credential.helper=",
            "core.fsmonitor=false",
            "fetch.recurseSubmodules=false",
            "transfer.fsckObjects=true",
        ] {
            assert!(args.iter().any(|a| a == want), "{want} missing in {args:?}");
        }
        assert!(
            args.iter()
                .any(|a| a.starts_with("core.hooksPath=") && a.ends_with("/nohooks")),
            "{args:?}"
        );
        assert!(
            !args.iter().any(|a| a.starts_with("protocol.file")),
            "{args:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_command_past_its_deadline_is_killed() {
        let mut cmd = Command::new("/bin/sleep");
        cmd.arg("30");
        let started = Instant::now();
        let mut job =
            Supervised::spawn(cmd, Instant::now() + Duration::from_millis(200), None).unwrap();
        drop(job.stdin.take());
        let err = job.finish("sleep").unwrap_err();
        assert!(
            matches!(&err, GitError::Failed(m) if m.contains("timed out")),
            "{err:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
