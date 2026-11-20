//! Ask an installed executable which version it is.
//!
//! The update lock needs the version of the file on disk, and that file may not be the process
//! that runs the update: another installer can have replaced it since this process started. The
//! only evidence about such a file is what it reports, so this runs it as
//! `slopvac --offline --version --json`, with a deadline and a cap on the output, and parses a
//! strict semantic version out of the document. Any failure is an error: the caller treats an
//! unknown version as a reason to refuse, never as a version.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::messages::{UpdatePhrase, UpdateText};

/// How long the executable has to answer.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
/// The most output that is read. The document is under 1 KiB without notices.
const MAX_OUTPUT_BYTES: u64 = 64 * 1024;
/// How often the wait for the child checks it.
const POLL: Duration = Duration::from_millis(10);

/// Why the version could not be established.
#[derive(Debug, Error)]
pub(crate) enum ProbeError {
    /// The executable did not start.
    #[error("{}", spawn_text(.0))]
    Spawn(std::io::Error),
    /// The executable did not answer in time.
    #[error("{}", timeout_text(*.0))]
    Timeout(Duration),
    /// The executable exited with a failure.
    #[error("{}", failed_text(.0))]
    Failed(std::process::ExitStatus),
    /// The output was larger than the cap.
    #[error("{}", too_large_text())]
    TooLarge,
    /// The output is not a slopvac version document.
    #[error("{}", UpdatePhrase::ProbeUnparseable.as_static())]
    Unparseable,
    /// Reading the output failed.
    #[error("{}", read_text(.0))]
    Read(std::io::Error),
}

fn spawn_text(source: &std::io::Error) -> String {
    UpdateText::ProbeSpawn {
        detail: source.to_string(),
    }
    .to_string()
}

fn timeout_text(timeout: Duration) -> String {
    UpdateText::ProbeTimeout {
        seconds: timeout.as_secs_f32().to_string(),
    }
    .to_string()
}

fn failed_text(status: &std::process::ExitStatus) -> String {
    UpdateText::ProbeFailed {
        status: status.to_string(),
    }
    .to_string()
}

fn too_large_text() -> String {
    UpdateText::ProbeTooLarge {
        limit: MAX_OUTPUT_BYTES,
    }
    .to_string()
}

fn read_text(source: &std::io::Error) -> String {
    UpdateText::ProbeRead {
        detail: source.to_string(),
    }
    .to_string()
}

/// The version that the executable at `exe` reports.
pub(crate) fn installed_version(
    exe: &Path,
    timeout: Duration,
) -> Result<semver::Version, ProbeError> {
    let deadline = Instant::now() + timeout;
    let mut child = Command::new(exe)
        .args(["--offline", "--version", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(ProbeError::Spawn)?;
    let stdout = child.stdout.take().ok_or(ProbeError::Unparseable)?;
    let (tx, rx) = mpsc::channel();
    // The reader is never joined: a grandchild that keeps the pipe open must not hold us.
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let read = stdout
            .take(MAX_OUTPUT_BYTES + 1)
            .read_to_end(&mut buf)
            .map(|_| buf);
        let _ = tx.send(read); // The receiver is gone after a timeout; nothing to report.
    });
    let status = loop {
        match child.try_wait().map_err(ProbeError::Read)? {
            Some(status) => break status,
            None if Instant::now() < deadline => std::thread::sleep(POLL),
            None => {
                let _ = child.kill(); // It may have exited since the last check.
                let _ = child.wait(); // Reaps the child; the error adds nothing.
                return Err(ProbeError::Timeout(timeout));
            }
        }
    };
    // Size is judged before the exit status: a child that writes past the cap may die of the
    // closed pipe, and the cap is the more useful report.
    let remaining = deadline.saturating_duration_since(Instant::now());
    let output = rx
        .recv_timeout(remaining)
        .map_err(|_| ProbeError::Timeout(timeout))?
        .map_err(ProbeError::Read)?;
    if output.len() as u64 > MAX_OUTPUT_BYTES {
        return Err(ProbeError::TooLarge);
    }
    if !status.success() {
        return Err(ProbeError::Failed(status));
    }
    parse(&output)
}

/// The version in a `--version --json` document, which must name `slopvac`.
fn parse(output: &[u8]) -> Result<semver::Version, ProbeError> {
    let doc: serde_json::Value =
        serde_json::from_slice(output).map_err(|_| ProbeError::Unparseable)?;
    if doc.pointer("/name").and_then(serde_json::Value::as_str) != Some("slopvac") {
        return Err(ProbeError::Unparseable);
    }
    doc.pointer("/version")
        .and_then(serde_json::Value::as_str)
        .and_then(|v| semver::Version::parse(v).ok())
        .ok_or(ProbeError::Unparseable)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A script that stands in for the installed executable.
    fn script(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("slopvac");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, path)
    }

    const LONG: Duration = Duration::from_secs(300);

    #[test]
    fn the_reported_version_is_parsed() {
        let (_dir, exe) = script(
            r#"echo '{"name":"slopvac","version":"3.3.0","install_method":"x","notices":[]}'"#,
        );
        assert_eq!(
            installed_version(&exe, LONG).unwrap(),
            semver::Version::new(3, 3, 0)
        );
    }

    #[test]
    fn the_probe_passes_the_flags_that_print_the_version_document() {
        let (_dir, exe) = script(
            r#"[ "$*" = "--offline --version --json" ] && echo '{"name":"slopvac","version":"3.3.0"}'"#,
        );
        assert!(installed_version(&exe, LONG).is_ok());
    }

    #[test]
    fn output_that_is_not_a_slopvac_version_document_is_unparseable() {
        for body in [
            "echo hello",
            "echo '{}'",
            r#"echo '{"name":"slopvac","version":"3"}'"#,
            r#"echo '{"name":"slopvac","version":"v3.3.0"}'"#,
            r#"echo '{"name":"slopvac","version":330}'"#,
            r#"echo '{"name":"other","version":"3.3.0"}'"#,
        ] {
            let (_dir, exe) = script(body);
            let err = installed_version(&exe, LONG).unwrap_err();
            assert!(matches!(err, ProbeError::Unparseable), "{body}: {err}");
        }
    }

    #[test]
    fn an_executable_that_fails_is_an_error_even_if_it_printed_a_version() {
        let (_dir, exe) = script(r#"echo '{"name":"slopvac","version":"3.3.0"}'; exit 3"#);
        let err = installed_version(&exe, LONG).unwrap_err();
        assert!(matches!(err, ProbeError::Failed(_)), "{err}");
    }

    // The deadline has passed when the first check runs, and the child never exits by itself,
    // so the outcome does not depend on how fast the machine is.
    #[test]
    fn an_executable_that_has_not_answered_by_the_deadline_is_a_timeout() {
        let (_dir, exe) = script("exec sleep 3600");
        let err = installed_version(&exe, Duration::ZERO).unwrap_err();
        assert!(matches!(err, ProbeError::Timeout(_)), "{err}");
    }

    #[test]
    fn output_over_the_cap_is_refused() {
        let (_dir, exe) = script("exec head -c 200000 /dev/zero");
        let err = installed_version(&exe, LONG).unwrap_err();
        assert!(matches!(err, ProbeError::TooLarge), "{err}");
    }

    #[test]
    fn a_file_that_is_not_executable_is_a_spawn_error() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("slopvac");
        std::fs::write(&exe, b"binary").unwrap();
        let err = installed_version(&exe, LONG).unwrap_err();
        assert!(matches!(err, ProbeError::Spawn(_)), "{err}");
    }
}
