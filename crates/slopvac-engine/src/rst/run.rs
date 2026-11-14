//! Run the docutils converter on one document.
//!
//! The document is untrusted repository content, so the process is boxed in: a fixed
//! argument list, no shell, a small allow-list of environment variables, a time limit and
//! a cap on the output.

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::RecvTimeoutError;

use super::RstError;

/// The fixed argument list. Nothing from the repository reaches it.
///
/// - `--no-file-insertion` turns off `include`, `csv-table :file:` and every other
///   directive that reads a file or a URL.
/// - `--no-raw` turns off the `raw` directive and role.
/// - `--quiet` and `--report=5` keep system messages out of the HTML, so a docutils
///   warning is never linted as prose. `--halt=5` and `--exit-status=5` let a document
///   with errors convert anyway.
/// - `--link-stylesheet` keeps the converter from embedding a stylesheet.
/// - `--syntax-highlight=none` keeps the output free of highlighter markup.
/// - `--input-encoding` and `--output-encoding` fix UTF-8, so offsets stay byte exact.
///
/// The document goes in on stdin and the HTML comes out on stdout.
pub const ARGS: [&str; 10] = [
    "--no-file-insertion",
    "--no-raw",
    "--quiet",
    "--report=5",
    "--halt=5",
    "--exit-status=5",
    "--link-stylesheet",
    "--syntax-highlight=none",
    "--input-encoding=utf-8",
    "--output-encoding=utf-8",
];

/// Variables the converter may inherit. Everything else is dropped. `PATH` is needed for
/// the script's `env python` line, and the rest keep Windows and version managers working.
const INHERITED_ENV: [&str; 12] = [
    "PATH",
    "HOME",
    "USERPROFILE",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "PATHEXT",
    "TMPDIR",
    "TEMP",
    "TMP",
    "LANG",
    "LC_ALL",
    "VIRTUAL_ENV",
];

/// docutils reads its configuration files from this list when `DOCUTILSCONFIG` is set, and
/// from the working directory, the home directory and `/etc` otherwise. An empty file
/// leaves the defaults, so a `docutils.conf` in the repository changes nothing.
const NULL_DEVICE: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

/// Most stderr bytes kept for an error message.
const STDERR_KEPT: usize = 2048;

enum ReadFailure {
    TooLarge,
    Io(io::Error),
}

/// Run `converter` on `input` and return its stdout.
pub(super) fn run(
    converter: &Path,
    input: &[u8],
    timeout: Duration,
    max_output: usize,
) -> Result<Vec<u8>, RstError> {
    let mut command = Command::new(converter);
    command
        .args(ARGS)
        .env_clear()
        .env("DOCUTILSCONFIG", NULL_DEVICE)
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(std::env::temp_dir());
    for name in INHERITED_ENV {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = command.spawn().map_err(|source| RstError::Spawn {
        converter: converter.to_owned(),
        source,
    })?;
    let (Some(mut stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        kill(&mut child);
        return Err(RstError::Failed {
            converter: converter.to_owned(),
            status: "no pipes".to_owned(),
            stderr: String::new(),
        });
    };
    let deadline = Instant::now() + timeout;

    // Dedicated threads do the blocking I/O so this thread can enforce the deadline.
    // A write error means the converter stopped reading, which its exit status reports.
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
                converter: converter.to_owned(),
                limit: max_output,
            });
        }
        Ok(Err(ReadFailure::Io(source))) => {
            kill(&mut child);
            return Err(RstError::Spawn {
                converter: converter.to_owned(),
                source,
            });
        }
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
            kill(&mut child);
            return Err(RstError::Timeout {
                converter: converter.to_owned(),
                timeout,
            });
        }
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return finish(converter, status, bytes, &err_rx);
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(1)),
            Ok(None) => {
                kill(&mut child);
                return Err(RstError::Timeout {
                    converter: converter.to_owned(),
                    timeout,
                });
            }
            Err(source) => {
                kill(&mut child);
                return Err(RstError::Spawn {
                    converter: converter.to_owned(),
                    source,
                });
            }
        }
    }
}

fn finish(
    converter: &Path,
    status: ExitStatus,
    bytes: Vec<u8>,
    err_rx: &crossbeam_channel::Receiver<Vec<u8>>,
) -> Result<Vec<u8>, RstError> {
    if status.success() {
        return Ok(bytes);
    }
    let stderr = err_rx
        .recv_timeout(Duration::from_millis(200))
        .map(|raw| String::from_utf8_lossy(&raw).trim().to_owned())
        .unwrap_or_default();
    Err(RstError::Failed {
        converter: converter.to_owned(),
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

/// Keep the start of stderr and drain the rest so the converter never blocks on it.
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
