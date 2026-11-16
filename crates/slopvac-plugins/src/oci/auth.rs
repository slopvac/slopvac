//! Registry credentials and `WWW-Authenticate` challenges.
//!
//! Precedence: `SLOPVAC_OCI_USER` plus `SLOPVAC_OCI_TOKEN`, then `GITHUB_TOKEN`
//! for `ghcr.io` only, then the Docker config (`auths`, `credHelpers`,
//! `credsStore`), then anonymous. Credentials live in memory and in request
//! headers only.
//!
//! The `docker_credential` crate was considered. It runs a helper with no
//! timeout, no output cap and no name check, which the threat model (N6)
//! requires, so this module reads the config and runs the helper itself.

use crate::num::widen;
use std::fmt;
use std::io;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::Deserialize;

use super::OciError;
use super::process::Contained;

/// Longest a credential helper may run.
pub(crate) const HELPER_TIMEOUT: Duration = Duration::from_secs(10);
/// Most bytes a credential helper may print.
pub(crate) const HELPER_MAX_OUTPUT: usize = 64 * 1024;
const MAX_CONFIG_BYTES: u64 = 1 << 20;

/// A username and secret. `Debug` never prints the secret.
#[derive(Clone, PartialEq, Eq)]
pub struct Credential {
    /// Account name.
    pub username: String,
    /// Password or token.
    pub secret: String,
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credential")
            .field("username", &self.username)
            .field("secret", &"<redacted>")
            .finish()
    }
}

impl Credential {
    pub(crate) fn basic_header(&self) -> String {
        let raw = format!("{}:{}", self.username, self.secret);
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(raw)
        )
    }
}

/// Where credentials come from. Called once per registry request that needs them.
pub trait CredentialSource: Send + Sync {
    /// Credential for `registry` (host with optional port), or `None` for anonymous.
    ///
    /// # Errors
    /// [`OciError::Credential`] when a helper fails or the config is unreadable.
    fn credential(&self, registry: &str) -> Result<Option<Credential>, OciError>;
}

/// No credentials: anonymous access only.
#[derive(Debug, Default, Clone, Copy)]
pub struct Anonymous;

impl CredentialSource for Anonymous {
    fn credential(&self, _: &str) -> Result<Option<Credential>, OciError> {
        Ok(None)
    }
}

/// Environment and Docker config, in the documented precedence.
#[derive(Debug, Clone, Default)]
pub struct DefaultCredentials {
    /// Values for `SLOPVAC_OCI_USER`, `SLOPVAC_OCI_TOKEN` and `GITHUB_TOKEN`
    /// in that order, read by the caller so tests need no process environment.
    pub env: EnvCredentials,
    /// Directory that holds `config.json`. `None` disables the Docker config.
    pub docker_config_dir: Option<PathBuf>,
    /// Directories searched for `docker-credential-*`. `None` uses `PATH`.
    pub helper_search_path: Option<std::ffi::OsString>,
    /// Helper time limit. `None` means the default of 10 seconds; tests shorten it.
    pub helper_timeout: Option<Duration>,
}

/// Credential-related environment values.
#[derive(Clone, Default)]
pub struct EnvCredentials {
    /// `SLOPVAC_OCI_USER`.
    pub user: Option<String>,
    /// `SLOPVAC_OCI_TOKEN`.
    pub token: Option<String>,
    /// `GITHUB_TOKEN`.
    pub github_token: Option<String>,
}

impl fmt::Debug for EnvCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvCredentials")
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

impl DefaultCredentials {
    /// Read the process environment: `SLOPVAC_OCI_*`, `GITHUB_TOKEN`, `DOCKER_CONFIG`
    /// and the home directory's `.docker`. The project directory is never consulted.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_lookup(|k| std::env::var(k).ok(), cfg!(windows))
    }

    /// Test seam: [`Self::from_env`] over any variable lookup and platform, so
    /// tests need no process environment.
    #[doc(hidden)]
    #[must_use]
    pub fn from_lookup_for_tests(var: impl Fn(&str) -> Option<String>, windows: bool) -> Self {
        Self::from_lookup(var, windows)
    }

    /// Docker finds its default config under `USERPROFILE` on Windows and
    /// `HOME` elsewhere; on Windows `HOME` is the fallback for shells that set
    /// only it.
    fn from_lookup(var: impl Fn(&str) -> Option<String>, windows: bool) -> Self {
        let get = |k: &str| var(k).filter(|v| !v.is_empty());
        let home = if windows {
            get("USERPROFILE").or_else(|| get("HOME"))
        } else {
            get("HOME")
        };
        let docker = get("DOCKER_CONFIG")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| Path::new(&h).join(".docker")));
        Self {
            env: EnvCredentials {
                user: get("SLOPVAC_OCI_USER"),
                token: get("SLOPVAC_OCI_TOKEN"),
                github_token: get("GITHUB_TOKEN"),
            },
            docker_config_dir: docker,
            helper_search_path: None,
            helper_timeout: None,
        }
    }
}

#[derive(Deserialize, Default)]
struct DockerConfig {
    #[serde(default)]
    auths: std::collections::HashMap<String, AuthEntry>,
    #[serde(default, rename = "credHelpers")]
    cred_helpers: std::collections::HashMap<String, String>,
    #[serde(default, rename = "credsStore")]
    creds_store: Option<String>,
}

#[derive(Deserialize, Default, PartialEq, Eq)]
struct AuthEntry {
    #[serde(default)]
    auth: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

/// Normalise a key of `auths`: drop the scheme and any path.
fn auth_key_host(key: &str) -> &str {
    let k = key.split_once("://").map_or(key, |(_, r)| r);
    k.split('/').next().unwrap_or(k)
}

/// The one value among `matches`, the config entries whose key names `registry`
/// up to ASCII case. Keys that differ only in case are one host (RFC 4343), so
/// differing values are ambiguous and refused instead of picked by map order.
fn unique_match<'a, T: PartialEq>(
    mut matches: impl Iterator<Item = &'a T>,
    section: &str,
    registry: &str,
) -> Result<Option<&'a T>, OciError> {
    let first = matches.next();
    if let Some(a) = first
        && matches.any(|b| a != b)
    {
        return Err(OciError::Credential(format!(
            "the Docker config has conflicting `{section}` entries for `{registry}`"
        )));
    }
    Ok(first)
}

/// A registry name safe to hand to a helper: host characters and an optional port.
pub(crate) fn safe_registry_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 260
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'))
}

/// `[a-z0-9_.-]+`, not starting with a dot or dash.
pub(crate) fn safe_helper_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.starts_with(['.', '-'])
        && s.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'-')
        })
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct HelperReply {
    username: String,
    secret: String,
}

/// File-name suffixes that make a helper runnable on this platform. Unix needs
/// none. Windows honours `PATHEXT` but only for `.exe` and `.com`: batch files
/// would run through `cmd.exe`, whose argument parsing is unsafe to feed.
fn executable_suffixes() -> Vec<String> {
    if !cfg!(windows) {
        return vec![String::new()];
    }
    let pathext = std::env::var("PATHEXT").unwrap_or_default();
    let mut found: Vec<String> = pathext
        .split(';')
        .map(str::to_ascii_lowercase)
        .filter(|e| matches!(e.as_str(), ".exe" | ".com"))
        .collect();
    if found.is_empty() {
        found = vec![".exe".into(), ".com".into()];
    }
    found
}

fn find_in_path(program: &str, search: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    find_with_suffixes(program, search, &executable_suffixes())
}

fn find_with_suffixes(
    program: &str,
    search: Option<&std::ffi::OsStr>,
    suffixes: &[String],
) -> Option<PathBuf> {
    let path = search
        .map(ToOwned::to_owned)
        .or_else(|| std::env::var_os("PATH"))?;
    // A name that already ends in a runnable suffix (`desktop.exe`) is tried as
    // written before any suffix is appended.
    let lower = program.to_ascii_lowercase();
    let as_written = suffixes
        .iter()
        .any(|ext| !ext.is_empty() && lower.ends_with(ext.as_str()));
    std::env::split_paths(&path).find_map(|dir| {
        let written = as_written.then(|| dir.join(program));
        written
            .into_iter()
            .chain(
                suffixes
                    .iter()
                    .map(|ext| dir.join(format!("{program}{ext}"))),
            )
            .find(|p| p.is_file())
    })
}

/// Write the registry name to the helper's stdin on a thread.
fn feed_registry(mut stdin: std::process::ChildStdin, registry: &str) {
    // A helper that never reads is caught by the lookup's deadline.
    let reg = registry.to_owned();
    std::thread::spawn(move || {
        #[expect(
            clippy::let_underscore_must_use,
            reason = "a helper that exits without reading ends the lookup through its exit status or the deadline"
        )]
        let _ = stdin.write_all(reg.as_bytes());
    });
}

/// Read the helper's stdout on a thread: to end-of-file or one byte past the cap.
///
/// Killing the helper's whole tree closes every write end, so the reader never
/// outlives a lookup.
fn read_output(
    stdout: impl Read + Send + 'static,
) -> std::sync::mpsc::Receiver<io::Result<Vec<u8>>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let read = stdout
            .take(widen(HELPER_MAX_OUTPUT) + 1)
            .read_to_end(&mut buf)
            .map(|_| buf);
        #[expect(
            clippy::let_underscore_must_use,
            reason = "the lookup drops the receiver only after it has given up on this reader"
        )]
        let _ = tx.send(read);
    });
    rx
}

/// What polling the output reader found.
#[derive(Debug, PartialEq, Eq)]
enum Polled {
    Pending,
    Done(Vec<u8>),
    /// The lookup must end; the reason.
    Failed(String),
}

/// Take what the output reader has sent, if anything.
fn poll_output(rx: &std::sync::mpsc::Receiver<io::Result<Vec<u8>>>, program: &str) -> Polled {
    match rx.try_recv() {
        Ok(Ok(buf)) if buf.len() > HELPER_MAX_OUTPUT => Polled::Failed(format!(
            "`{program}` printed more than {HELPER_MAX_OUTPUT} bytes"
        )),
        Ok(Ok(buf)) => Polled::Done(buf),
        Ok(Err(e)) => Polled::Failed(format!("reading the output of `{program}`: {e}")),
        Err(std::sync::mpsc::TryRecvError::Empty) => Polled::Pending,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            Polled::Failed("helper reader failed".into())
        }
    }
}

/// The credential error for `why`. A failed kill is named in it: the helper may
/// still be running.
fn stop_error(why: String, stopped: io::Result<()>) -> OciError {
    match stopped {
        Ok(()) => OciError::Credential(why),
        Err(e) => OciError::Credential(format!("{why}; the helper could not be stopped: {e}")),
    }
}

/// Kill the helper's tree, and build the credential error for `why`.
fn abort_helper(helper: Contained, why: String) -> OciError {
    stop_error(why, helper.kill_tree())
}

fn run_helper(
    name: &str,
    registry: &str,
    search: Option<&std::ffi::OsStr>,
    timeout: Duration,
) -> Result<Option<Credential>, OciError> {
    let cred = |m: String| OciError::Credential(m);
    if !safe_helper_name(name) {
        return Err(cred(format!(
            "credential helper name `{name}` is not allowed"
        )));
    }
    if !safe_registry_name(registry) {
        return Err(cred(
            "registry name is not safe to pass to a credential helper".into(),
        ));
    }
    let program = format!("docker-credential-{name}");
    let Some(path) = find_in_path(&program, search) else {
        return Err(cred(format!("credential helper `{program}` was not found")));
    };
    // One deadline covers the helper's exit and the draining of its output.
    let deadline = Instant::now() + timeout;
    let mut command = Command::new(path);
    command
        .arg("get")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut helper = Contained::spawn(&mut command)
        .map_err(|e| cred(format!("cannot start `{program}`: {e}")))?;
    if let Some(stdin) = helper.child_mut().stdin.take() {
        feed_registry(stdin, registry);
    }
    let Some(stdout) = helper.child_mut().stdout.take() else {
        return Err(abort_helper(helper, "no helper stdout".into()));
    };
    let rx = read_output(stdout);
    let mut exited = false;
    let mut output = None;
    while !exited || output.is_none() {
        if !exited {
            exited = match helper.has_exited() {
                Ok(done) => done,
                Err(e) => {
                    return Err(abort_helper(
                        helper,
                        format!("waiting for `{program}`: {e}"),
                    ));
                }
            };
        }
        if output.is_none() {
            match poll_output(&rx, &program) {
                Polled::Pending => {}
                Polled::Done(buf) => output = Some(buf),
                Polled::Failed(why) => return Err(abort_helper(helper, why)),
            }
        }
        if exited && output.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            return Err(abort_helper(helper, format!("`{program}` timed out")));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let out = output.ok_or_else(|| cred("helper did not finish".into()))?;
    let status = helper
        .reap()
        .map_err(|e| cred(format!("waiting for `{program}`: {e}")))?;
    if !status.success() {
        // "credentials not found in native keychain" is a normal miss.
        return Ok(None);
    }
    let reply: HelperReply = serde_json::from_slice(&out).map_err(|e| {
        cred(format!(
            "`{program}` printed invalid JSON (line {}, column {})",
            e.line(),
            e.column()
        ))
    })?;
    if reply.username == "<token>" {
        return Err(cred(
            "identity tokens from credential helpers are not supported".into(),
        ));
    }
    Ok(Some(Credential {
        username: reply.username,
        secret: reply.secret,
    }))
}

impl DefaultCredentials {
    fn docker_config_credential(&self, registry: &str) -> Result<Option<Credential>, OciError> {
        let Some(dir) = &self.docker_config_dir else {
            return Ok(None);
        };
        let file = dir.join("config.json");
        let Ok(f) = std::fs::File::open(&file) else {
            return Ok(None);
        };
        let mut raw = Vec::new();
        f.take(MAX_CONFIG_BYTES + 1)
            .read_to_end(&mut raw)
            .map_err(|e| OciError::Credential(format!("reading the Docker config: {e}")))?;
        if widen(raw.len()) > MAX_CONFIG_BYTES {
            return Err(OciError::Credential(
                "the Docker config is larger than 1 MiB".into(),
            ));
        }
        let cfg: DockerConfig = serde_json::from_slice(&raw).map_err(|e| {
            OciError::Credential(format!(
                "the Docker config is not valid JSON (line {}, column {})",
                e.line(),
                e.column()
            ))
        })?;
        let helper = unique_match(
            cfg.cred_helpers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case(registry))
                .map(|(_, v)| v),
            "credHelpers",
            registry,
        )?
        .or(cfg.creds_store.as_ref());
        if let Some(name) = helper
            && let Some(c) = run_helper(
                name,
                registry,
                self.helper_search_path.as_deref(),
                self.helper_timeout.unwrap_or(HELPER_TIMEOUT),
            )?
        {
            return Ok(Some(c));
        }
        let entry = unique_match(
            cfg.auths
                .iter()
                .filter(|(k, _)| auth_key_host(k).eq_ignore_ascii_case(registry))
                .map(|(_, v)| v),
            "auths",
            registry,
        )?;
        let Some(entry) = entry else {
            return Ok(None);
        };
        if let Some(auth) = &entry.auth {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(auth.trim())
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .ok_or_else(|| OciError::Credential("an `auth` entry is not base64 text".into()))?;
            let (u, p) = decoded
                .split_once(':')
                .ok_or_else(|| OciError::Credential("an `auth` entry has no colon".into()))?;
            return Ok(Some(Credential {
                username: u.to_owned(),
                secret: p.to_owned(),
            }));
        }
        match (&entry.username, &entry.password) {
            (Some(u), Some(p)) => Ok(Some(Credential {
                username: u.clone(),
                secret: p.clone(),
            })),
            _ => Ok(None),
        }
    }
}

impl CredentialSource for DefaultCredentials {
    fn credential(&self, registry: &str) -> Result<Option<Credential>, OciError> {
        let e = &self.env;
        if let (Some(u), Some(t)) = (&e.user, &e.token) {
            return Ok(Some(Credential {
                username: u.clone(),
                secret: t.clone(),
            }));
        }
        if registry.eq_ignore_ascii_case("ghcr.io")
            && let Some(t) = &e.github_token
        {
            return Ok(Some(Credential {
                username: "slopvac".into(),
                secret: t.clone(),
            }));
        }
        self.docker_config_credential(registry)
    }
}

/// A parsed `WWW-Authenticate` challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
    /// Lowercased scheme: `bearer` or `basic`.
    pub scheme: String,
    /// Parameters with lowercased names.
    pub params: Vec<(String, String)>,
}

impl Challenge {
    /// A parameter by name.
    #[must_use]
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Parse the first challenge of a header value. Quoted strings may hold commas
    /// and backslash escapes.
    #[must_use]
    pub fn parse(header: &str) -> Option<Self> {
        let header = header.trim();
        let (scheme, mut rest) = header
            .split_once(char::is_whitespace)
            .unwrap_or((header, ""));
        if scheme.is_empty() {
            return None;
        }
        let mut params = Vec::new();
        loop {
            rest = rest.trim_start_matches([' ', '\t', ',']);
            if rest.is_empty() {
                break;
            }
            let eq = rest.find('=')?;
            let name = rest[..eq].trim().to_ascii_lowercase();
            // RFC 9110 allows optional whitespace around `=`.
            rest = rest[eq + 1..].trim_start_matches([' ', '\t']);
            let value;
            if let Some(q) = rest.strip_prefix('"') {
                let mut out = String::new();
                let mut chars = q.char_indices();
                let mut end = None;
                while let Some((i, c)) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some((_, n)) = chars.next() {
                                out.push(n);
                            }
                        }
                        '"' => {
                            end = Some(i + 1);
                            break;
                        }
                        _ => out.push(c),
                    }
                }
                rest = &q[end?..];
                value = out;
            } else {
                let stop = rest.find(',').unwrap_or(rest.len());
                value = rest[..stop].trim().to_owned();
                rest = &rest[stop..];
            }
            params.push((name, value));
        }
        Some(Self {
            scheme: scheme.to_ascii_lowercase(),
            params,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_bearer_challenge_with_quoted_commas() {
        let c = Challenge::parse(
            r#"Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:a/b:pull,push""#,
        )
        .unwrap();
        assert_eq!(c.scheme, "bearer");
        assert_eq!(c.param("realm"), Some("https://ghcr.io/token"));
        assert_eq!(c.param("scope"), Some("repository:a/b:pull,push"));
    }

    #[test]
    fn parses_unquoted_values_escapes_and_basic() {
        let c = Challenge::parse(r#"Basic realm="a \"b\"", charset=UTF-8"#).unwrap();
        assert_eq!(c.scheme, "basic");
        assert_eq!(c.param("realm"), Some(r#"a "b""#));
        assert_eq!(c.param("charset"), Some("UTF-8"));
        assert!(Challenge::parse("").is_none());
        assert!(Challenge::parse(r#"Bearer realm="unterminated"#).is_none());
    }

    #[test]
    fn challenge_parameters_may_have_whitespace_around_the_equals_sign() {
        let c = Challenge::parse(r#"Bearer realm = "https://r.example/token" , service= "reg""#)
            .unwrap();
        assert_eq!(c.param("realm"), Some("https://r.example/token"));
        assert_eq!(c.param("service"), Some("reg"));
    }

    #[test]
    fn helper_names_must_be_plain() {
        for ok in ["osxkeychain", "pass", "ecr-login", "desktop.exe", "a_b"] {
            assert!(safe_helper_name(ok), "{ok}");
        }
        for bad in [
            "",
            "../x",
            "a/b",
            "a;b",
            "a b",
            "-x",
            ".x",
            "A",
            "a\0",
            &"a".repeat(65),
        ] {
            assert!(!safe_helper_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn registry_names_for_helpers_are_hosts() {
        assert!(safe_registry_name("ghcr.io"));
        assert!(safe_registry_name("localhost:5000"));
        for bad in ["", "a b", "a\n", "a/b", "a;b", "$(x)"] {
            assert!(!safe_registry_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn debug_never_prints_the_secret() {
        let c = Credential {
            username: "u".into(),
            secret: "hunter2".into(),
        };
        assert!(!format!("{c:?}").contains("hunter2"));
        let e = EnvCredentials {
            github_token: Some("ghp_secret".into()),
            ..EnvCredentials::default()
        };
        assert!(!format!("{e:?}").contains("ghp_secret"));
    }

    #[test]
    fn env_precedence_and_github_token_scope() {
        let creds = DefaultCredentials {
            env: EnvCredentials {
                github_token: Some("t".into()),
                ..EnvCredentials::default()
            },
            ..DefaultCredentials::default()
        };
        assert!(creds.credential("ghcr.io").unwrap().is_some());
        assert!(
            creds.credential("registry.example").unwrap().is_none(),
            "GITHUB_TOKEN must stay on ghcr.io"
        );
        let both = DefaultCredentials {
            env: EnvCredentials {
                user: Some("me".into()),
                token: Some("s".into()),
                github_token: Some("t".into()),
            },
            ..DefaultCredentials::default()
        };
        assert_eq!(both.credential("ghcr.io").unwrap().unwrap().username, "me");
    }

    /// A reader whose every read fails, as a broken pipe would.
    struct BrokenPipe;

    impl Read for BrokenPipe {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("pipe broke"))
        }
    }

    fn poll_until_settled(rx: &std::sync::mpsc::Receiver<io::Result<Vec<u8>>>) -> Polled {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            match poll_output(rx, "docker-credential-x") {
                Polled::Pending if Instant::now() < end => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                polled @ (Polled::Pending | Polled::Done(_) | Polled::Failed(_)) => return polled,
            }
        }
    }

    #[test]
    fn a_read_error_on_the_helper_output_becomes_a_credential_error() {
        let polled = poll_until_settled(&read_output(BrokenPipe));
        let Polled::Failed(why) = polled else {
            panic!("expected the lookup to fail, got {polled:?}");
        };
        assert_eq!(
            why,
            "reading the output of `docker-credential-x`: pipe broke"
        );
        assert_eq!(
            stop_error(why.clone(), Ok(())).to_string(),
            OciError::Credential(why).to_string()
        );
    }

    #[test]
    fn complete_helper_output_is_not_an_error() {
        let polled = poll_until_settled(&read_output(&b"{}"[..]));
        assert_eq!(polled, Polled::Done(b"{}".to_vec()));
    }

    #[test]
    fn a_helper_that_could_not_be_stopped_is_named_in_the_error() {
        let stopped = Err(io::Error::other("kill refused"));
        let err = stop_error("`docker-credential-x` timed out".into(), stopped);
        assert!(
            matches!(&err, OciError::Credential(m)
                if m == "`docker-credential-x` timed out; the helper could not be stopped: kill refused"),
            "{err:?}"
        );
    }
}
