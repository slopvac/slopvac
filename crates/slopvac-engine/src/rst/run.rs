//! Run the driver on one document.
//!
//! The document is untrusted repository content, so the process is boxed in: a fixed
//! argument list, no shell, a small allow-list of environment variables, a `PATH` without
//! project directories, an empty private working directory, a time limit and a cap on the
//! output.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::RecvTimeoutError;

use super::RstError;

/// The arguments before the driver text: isolate the interpreter from `PYTHON*`
/// variables, write no bytecode, use UTF-8 everywhere, and take the program from the next
/// argument. Nothing from the repository reaches the argument list.
pub const ARGS_BEFORE_DRIVER: [&str; 5] = ["-E", "-B", "-X", "utf8", "-c"];

/// The driver, which the interpreter runs as the `-c` text.
pub const DRIVER: &str = include_str!("driver.py");

/// Variables the interpreter may inherit. Everything else is dropped, and `PATH` is set by
/// [`restricted_path`]. `HOME` keeps a user-level docutils install visible; the rest keep
/// Windows and virtual environments working.
pub const INHERITED_ENV: [&str; 10] = [
    "HOME",
    "USERPROFILE",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "TMPDIR",
    "TEMP",
    "TMP",
    "LANG",
    "LC_ALL",
    "VIRTUAL_ENV",
];

/// System directories the interpreter may use to find helpers.
#[cfg(not(windows))]
const SYSTEM_PATH: &[&str] = &["/usr/bin", "/bin"];
#[cfg(windows)]
const SYSTEM_PATH: &[&str] = &[];

/// docutils reads its configuration files from this list when `DOCUTILSCONFIG` is set, and
/// from the working directory, the home directory and `/etc` otherwise. An empty file
/// leaves the defaults, so a `docutils.conf` in the repository changes nothing. (The driver
/// reads no configuration either; this is a second lock.)
const NULL_DEVICE: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

/// Most stderr bytes kept for an error message.
const STDERR_KEPT: usize = 2048;

/// How long the import check may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// The driver exit code for a missing docutils.
const EXIT_NO_DOCUTILS: i32 = 3;
/// The driver exit code for a docutils older than 0.21.
const EXIT_OLD_DOCUTILS: i32 = 4;

/// The `PATH` of the child: the interpreter's own directory, then the system directories.
/// No project directory is on it, so a `#!/usr/bin/env` line cannot pick a project-local
/// program.
#[must_use]
pub fn restricted_path(interpreter: &Path) -> OsString {
    let mut dirs: Vec<PathBuf> = interpreter
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .into_iter()
        .collect();
    dirs.extend(SYSTEM_PATH.iter().map(PathBuf::from));
    dirs.extend(windows_system_dir());
    std::env::join_paths(dirs).unwrap_or_default()
}

/// The command that runs `program` in `interpreter`, boxed in as described in the module
/// documentation.
fn command(interpreter: &Path, cwd: &Path, program: &str) -> Command {
    let mut command = Command::new(interpreter);
    command
        .args(ARGS_BEFORE_DRIVER)
        .arg(program)
        .env_clear()
        .env("PATH", restricted_path(interpreter))
        .env("DOCUTILSCONFIG", NULL_DEVICE)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(cwd);
    for name in INHERITED_ENV {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
}

/// Whether `interpreter` can import docutils 0.21 or later.
pub(super) fn probe(interpreter: &Path) -> bool {
    const PROGRAM: &str =
        "import sys, docutils; sys.exit(0 if docutils.__version_info__[:2] >= (0, 21) else 1)";
    let Ok(cwd) = tempfile::tempdir() else {
        return false;
    };
    let Ok(mut child) = command(interpreter, cwd.path(), PROGRAM)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(2)),
            _ => {
                kill(&mut child);
                return false;
            }
        }
    }
}

/// The Windows system directory, which holds the C runtime that Python needs.
fn windows_system_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("SYSTEMROOT").map(|root| Path::new(&root).join("System32"))
    } else {
        None
    }
}

enum ReadFailure {
    TooLarge,
    Io(io::Error),
}

/// Run `interpreter` on the driver with `input` on stdin and return its stdout.
pub(super) fn run(
    interpreter: &Path,
    input: &[u8],
    timeout: Duration,
    max_output: usize,
) -> Result<Vec<u8>, RstError> {
    // An empty directory only this user can enter: Python puts the working directory on
    // `sys.path` for `-c`, so nothing planted there can shadow docutils.
    let cwd = tempfile::tempdir().map_err(|source| RstError::Spawn {
        converter: interpreter.to_owned(),
        source,
    })?;
    let mut cmd = command(interpreter, cwd.path(), DRIVER);
    let mut child = cmd.spawn().map_err(|source| RstError::Spawn {
        converter: interpreter.to_owned(),
        source,
    })?;
    let (Some(mut stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        kill(&mut child);
        return Err(RstError::Failed {
            converter: interpreter.to_owned(),
            status: "no pipes".to_owned(),
            stderr: String::new(),
        });
    };
    let deadline = Instant::now() + timeout;

    // Dedicated threads do the blocking I/O so this thread can enforce the deadline.
    // A write error means the interpreter stopped reading, which its exit status reports.
    let input = input.to_vec();
    thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let (out_tx, out_rx) = crossbeam_channel::bounded(1);
    thread::spawn(move || {
        let _ = out_tx.send(read_capped(stdout, max_output));
    });
    let (err_tx, err_rx) = crossbeam_channel::bounded(1);
    thread::spawn(move || {
        let _ = err_tx.send(read_stderr(stderr));
    });

    let bytes = match out_rx.recv_timeout(timeout) {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(ReadFailure::TooLarge)) => {
            kill(&mut child);
            return Err(RstError::OutputTooLarge {
                converter: interpreter.to_owned(),
                limit: max_output,
            });
        }
        Ok(Err(ReadFailure::Io(source))) => {
            kill(&mut child);
            return Err(RstError::Spawn {
                converter: interpreter.to_owned(),
                source,
            });
        }
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
            kill(&mut child);
            return Err(RstError::Timeout {
                converter: interpreter.to_owned(),
                timeout,
            });
        }
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return finish(interpreter, status, bytes, &err_rx),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(1)),
            Ok(None) => {
                kill(&mut child);
                return Err(RstError::Timeout {
                    converter: interpreter.to_owned(),
                    timeout,
                });
            }
            Err(source) => {
                kill(&mut child);
                return Err(RstError::Spawn {
                    converter: interpreter.to_owned(),
                    source,
                });
            }
        }
    }
}

fn finish(
    interpreter: &Path,
    status: ExitStatus,
    bytes: Vec<u8>,
    err_rx: &crossbeam_channel::Receiver<Vec<u8>>,
) -> Result<Vec<u8>, RstError> {
    if status.success() {
        return Ok(bytes);
    }
    match status.code() {
        Some(EXIT_NO_DOCUTILS) => {
            return Err(RstError::ConverterMissing {
                detail: format!("docutils is not installed for {}", interpreter.display()),
            });
        }
        Some(EXIT_OLD_DOCUTILS) => {
            return Err(RstError::ConverterMissing {
                detail: format!("docutils for {} is older than 0.21", interpreter.display()),
            });
        }
        _ => {}
    }
    let stderr = err_rx
        .recv_timeout(Duration::from_millis(200))
        .map(|raw| String::from_utf8_lossy(&raw).trim().to_owned())
        .unwrap_or_default();
    Err(RstError::Failed {
        converter: interpreter.to_owned(),
        status: status.code().map_or_else(
            || "terminated by a signal".to_owned(),
            |code| format!("exit status {code}"),
        ),
        stderr,
    })
}

fn kill(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Read to the end, and fail as soon as more than `cap` bytes arrive.
fn read_capped(reader: impl Read, cap: usize) -> Result<Vec<u8>, ReadFailure> {
    let mut bytes = Vec::new();
    let limit = u64::try_from(cap).unwrap_or(u64::MAX).saturating_add(1);
    reader
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(ReadFailure::Io)?;
    if bytes.len() > cap {
        return Err(ReadFailure::TooLarge);
    }
    Ok(bytes)
}

/// Keep the start of stderr and drain the rest so the interpreter never blocks on it.
fn read_stderr(mut reader: impl Read) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut chunk = [0_u8; 4096];
    while let Ok(n) = reader.read(&mut chunk) {
        if n == 0 {
            break;
        }
        let room = STDERR_KEPT.saturating_sub(kept.len());
        kept.extend_from_slice(&chunk[..n.min(room)]);
    }
    kept
}
