//! The GitHub CLI: print the exact `gh issue create` command, or run it after the user said yes.
//!
//! Slopvac never sees a token. `gh` holds the login; slopvac starts it with an argument list (no
//! shell), the host and repository pinned to [`HOST`] and [`REPOSITORY`], and the title and body
//! file it printed. `GH_HOST` and `GH_REPO` are cleared from the child's environment, so nothing
//! in the caller's environment can send the issue elsewhere.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::REPOSITORY;

/// The only host a report goes to.
pub const HOST: &str = "github.com";

/// How long one `gh` call may run.
pub const GH_TIMEOUT: Duration = Duration::from_secs(60);

/// Why `gh` could not create the issue. The text holds no output of `gh`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhError {
    /// `gh` is not installed or not on `PATH`.
    Missing,
    /// `gh auth status` failed: no login.
    NotLoggedIn,
    /// `gh issue create` failed or timed out.
    Failed(Option<i32>),
}

impl std::fmt::Display for GhError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => f.write_str("the GitHub CLI `gh` was not found on PATH"),
            Self::NotLoggedIn => f.write_str("`gh auth status` failed; run `gh auth login`"),
            Self::Failed(Some(code)) => write!(f, "`gh issue create` failed (exit {code})"),
            Self::Failed(None) => f.write_str("`gh issue create` failed or timed out"),
        }
    }
}

/// The GitHub CLI.
pub trait Gh {
    /// Check that `gh` exists and is logged in.
    fn check(&self) -> Result<(), GhError>;
    /// Create the issue and return what `gh` printed (the issue URL).
    fn create(&self, title: &str, body_file: &Path) -> Result<String, GhError>;
}

/// The real `gh`.
#[derive(Debug, Clone)]
pub struct SystemGh {
    program: OsString,
    timeout: Duration,
}

impl Default for SystemGh {
    fn default() -> Self {
        Self {
            program: "gh".into(),
            timeout: GH_TIMEOUT,
        }
    }
}

impl SystemGh {
    /// A `gh` at `program` with its own deadline. Tests use it to point at a stand-in.
    #[must_use]
    pub fn at(program: impl Into<OsString>, timeout: Duration) -> Self {
        Self {
            program: program.into(),
            timeout,
        }
    }
}

/// `owner/repo` with the host in front, so `gh` cannot read it as another host's repository.
fn qualified_repository() -> String {
    format!("{HOST}/{REPOSITORY}")
}

/// The argument list of `gh issue create`, after the program name. The body path stays an
/// `OsString`, so a path that is not UTF-8 reaches `gh` unchanged.
#[must_use]
pub fn create_args(title: &str, body_file: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "issue",
        "create",
        "-R",
        &qualified_repository(),
        "--title",
        title,
        "--body-file",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.push(body_file.as_os_str().to_owned());
    args
}

/// Quote one word for a POSIX shell: single quotes, with `'` written as `'\''`.
#[must_use]
pub fn sh_quote(word: &str) -> String {
    if !word.is_empty()
        && word
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./=:@%+".contains(&b))
    {
        return word.to_owned();
    }
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// The command line a user can paste into a POSIX shell, or `None` when it cannot be printed
/// safely: the body path is not UTF-8 or holds a control character.
#[must_use]
pub fn command_line(title: &str, body_file: &Path) -> Option<String> {
    body_file.to_str()?;
    let mut words = vec!["gh".to_owned()];
    words.extend(
        create_args(title, body_file)
            .iter()
            .map(|w| w.to_string_lossy().into_owned()),
    );
    // A terminal would act on an escape sequence even inside quotes.
    if words.iter().any(|w| w.chars().any(char::is_control)) {
        return None;
    }
    Some(
        words
            .iter()
            .map(|w| sh_quote(w))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Run `program` with `args`, no input, and kill it at the deadline. Returns the exit status
/// (`None` when it was killed) and the first kilobyte of standard output.
fn run(
    program: &OsStr,
    args: &[OsString],
    timeout: Duration,
) -> Result<(Option<i32>, String), GhError> {
    let mut child = Command::new(program)
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .env_remove("GH_HOST")
        .env_remove("GH_REPO")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| GhError::Missing)?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok((None, String::new()));
            }
        }
    };
    let mut out = String::new();
    if let Some(stdout) = child.stdout.take() {
        let _ = stdout.take(1024).read_to_string(&mut out);
    }
    Ok((status.code(), out))
}

impl Gh for SystemGh {
    fn check(&self) -> Result<(), GhError> {
        let args: Vec<OsString> = ["auth", "status", "--hostname", HOST]
            .iter()
            .map(OsString::from)
            .collect();
        match run(&self.program, &args, self.timeout)? {
            (Some(0), _) => Ok(()),
            _ => Err(GhError::NotLoggedIn),
        }
    }

    fn create(&self, title: &str, body_file: &Path) -> Result<String, GhError> {
        match run(&self.program, &create_args(title, body_file), self.timeout)? {
            (Some(0), out) => Ok(out.trim().to_owned()),
            (code, _) => Err(GhError::Failed(code)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_is_safe_for_a_posix_shell() {
        for (word, want) in [
            ("plain-word_1.md", "plain-word_1.md"),
            ("", "''"),
            ("two words", "'two words'"),
            ("it's", "'it'\\''s'"),
            ("a;b|c&d$(x)`y`", "'a;b|c&d$(x)`y`'"),
            ("[bug]: crash \"x\"", "'[bug]: crash \"x\"'"),
            ("-rf", "-rf"),
        ] {
            assert_eq!(sh_quote(word), want, "{word}");
        }
    }

    #[test]
    fn the_command_pins_the_host_and_repository_and_takes_the_body_from_a_file() {
        let line = command_line("[bug]: it's broken", Path::new("/tmp/body file.md"));
        assert_eq!(
            line.as_deref(),
            Some(
                "gh issue create -R github.com/slopvac/slopvac --title '[bug]: it'\\''s broken' --body-file '/tmp/body file.md'"
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_utf8_cannot_be_printed_but_reaches_gh_unchanged() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let path = std::path::PathBuf::from(OsString::from_vec(b"/tmp/b\xffody.md".to_vec()));
        assert_eq!(command_line("t", &path), None);
        let args = create_args("t", &path);
        assert_eq!(args.last().unwrap().as_bytes(), b"/tmp/b\xffody.md");
    }

    /// A stand-in `gh` with this script body.
    #[cfg(unix)]
    fn script(dir: &Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("gh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn a_nonzero_exit_from_create_is_a_failure_with_the_code() {
        let dir = tempfile::tempdir().unwrap();
        let gh = SystemGh::at(script(dir.path(), "echo https://x/1; exit 3"), GH_TIMEOUT);
        assert_eq!(
            gh.create("t", Path::new("/tmp/b")),
            Err(GhError::Failed(Some(3)))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_create_that_outlives_its_deadline_is_killed_and_fails_in_bounded_time() {
        let dir = tempfile::tempdir().unwrap();
        let gh = SystemGh::at(
            script(dir.path(), "exec sleep 30"),
            Duration::from_millis(300),
        );
        let started = Instant::now();
        assert_eq!(
            gh.create("t", Path::new("/tmp/b")),
            Err(GhError::Failed(None))
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_login_check_that_hangs_or_fails_is_not_logged_in() {
        let dir = tempfile::tempdir().unwrap();
        let hang = SystemGh::at(
            script(dir.path(), "exec sleep 30"),
            Duration::from_millis(300),
        );
        assert_eq!(hang.check(), Err(GhError::NotLoggedIn));
        let fail = SystemGh::at(script(dir.path(), "exit 1"), GH_TIMEOUT);
        assert_eq!(fail.check(), Err(GhError::NotLoggedIn));
    }

    #[test]
    fn a_missing_program_is_reported_as_missing() {
        let gh = SystemGh::at("/nonexistent/slopvac-no-gh", GH_TIMEOUT);
        assert_eq!(gh.check(), Err(GhError::Missing));
    }
}
