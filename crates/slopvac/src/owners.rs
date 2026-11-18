//! Commands whose logic lives in an owner crate: `terms`, `genre`, `validate`, `setup`
//! and `prime`. Each owner exposes `command()` and a run function; this module maps their
//! results and errors onto the CLI's exit codes and error codes (policies.md, 3.5) and
//! keeps absolute paths out of every message (policies.md, 1.7).

use std::io::Write;
use std::path::{Path, PathBuf};

use clap::ArgMatches;
use slopvac_agents::{AgentsError, prime, setup};
use slopvac_styles::{BuiltinSource, StylePresets};
use slopvac_terms::TermsError;

use crate::errors::CliError;
use crate::{cli, exit, redact};

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_default()
}

fn write(out: &mut dyn Write, text: &str) -> Result<(), CliError> {
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|err| {
            CliError::new(
                "output-failed",
                format!("cannot write output: {}", err.kind()),
            )
        })
}

/// A byte code from an owner crate's exit status.
fn exit_code(code: i32) -> u8 {
    u8::try_from(code).unwrap_or(exit::INCOMPLETE)
}

fn shown(path: &str) -> String {
    redact::shown_path(Path::new(path), &cwd())
}

fn terms_error(err: &TermsError) -> CliError {
    let (code, path) = match err {
        TermsError::Io { path, .. } => ("terms-io", path),
        TermsError::TooLarge { code, path, .. } => (*code, path),
        TermsError::Parse { path, .. } => ("terms-parse", path),
        TermsError::MissingSchemaVersion { path } => ("terms-missing-schema-version", path),
        TermsError::SchemaVersion { path, .. } => ("terms-schema-version", path),
        TermsError::UnknownKey { path, .. } => ("terms-unknown-key", path),
        TermsError::Entry { path, .. } => ("terms-entry", path),
    };
    // The owner's own wording, with its path (carried as data) replaced whole.
    let message = err.to_string().replace(path.as_str(), &shown(path));
    CliError::new(code, redact::redact_root(&message, &cwd()))
}

/// Name an owner's error from its fields, with every path shown through `shown_path`.
fn agents_error(err: &AgentsError) -> CliError {
    let cwd = cwd();
    let path = |p: &Path| redact::shown_path(p, &cwd);
    let message = match err {
        AgentsError::UnsafeTarget(target) => {
            format!("unsafe steering target: {}", path(Path::new(target)))
        }
        AgentsError::MalformedMarkers(p) => format!(
            "malformed or ambiguous slopvac markers in {}; repair the begin/end pair first",
            path(p)
        ),
        AgentsError::NotUtf8(p) => format!("steering must be UTF-8: {}", path(p)),
        AgentsError::Conflict(p) => format!("file changed during setup; retry: {}", path(p)),
        AgentsError::NotManaged(p) => format!(
            "{} is not a slopvac-managed file; move it aside to install it",
            path(p)
        ),
        AgentsError::Io { path: p, source } => format!("{}: {source}", path(p)),
        AgentsError::Usage(m) | AgentsError::Coded { message: m, .. } => m.clone(),
        // The config crate's rendered text, which leads with its own `error[code]:` and names
        // paths as the user typed them.
        AgentsError::Config {
            code,
            message,
            paths,
        } => {
            let body = message
                .strip_prefix(&format!("error[{code}]: "))
                .unwrap_or(message);
            // Replace each path the owner named, whole, before any text inference.
            let mut body = body.to_owned();
            for named in paths {
                body = body.replace(named.as_str(), &shown(named));
            }
            redact::redact_absolute(&body, &cwd)
        }
    };
    CliError::new(err.code(), message)
}

/// `slopvac terms`.
pub fn terms(m: &ArgMatches, out: &mut dyn Write) -> Result<u8, CliError> {
    slopvac_terms::cli::run(m, &cwd(), out)
        .map(exit_code)
        .map_err(|e| terms_error(&e))
}

/// `slopvac genre`.
pub fn genre(m: &ArgMatches, out: &mut dyn Write) -> Result<u8, CliError> {
    let text = slopvac_context::run(m).map_err(|e| {
        // The owner words a read failure as `<path as typed>: <reason>`. Replace each file
        // argument, exactly as typed, with its shown form.
        let mut message = e.to_string();
        for file in m.get_many::<String>("files").into_iter().flatten() {
            message = message.replace(file.as_str(), &shown(file));
        }
        CliError::new("genre-failed", message)
    })?;
    write(out, &text)?;
    Ok(exit::PASS)
}

/// `slopvac validate`.
pub fn validate(m: &ArgMatches, out: &mut dyn Write) -> Result<u8, CliError> {
    let presets = StylePresets(BuiltinSource);
    slopvac_config::validate::run(m, &cwd(), &presets, out).map_err(|err| {
        CliError::new(
            "output-failed",
            format!("cannot write output: {}", err.kind()),
        )
    })
}

/// `slopvac setup`.
pub fn setup(m: &ArgMatches, out: &mut dyn Write) -> Result<u8, CliError> {
    let outcome = setup::run(m).map_err(|e| agents_error(&e))?;
    let root = m
        .get_one::<PathBuf>("root")
        .cloned()
        .unwrap_or_else(|| PathBuf::from("."));
    for line in &outcome.lines {
        write(out, &format!("{}\n", redact::redact_root(line, &root)))?;
    }
    Ok(outcome.exit)
}

/// `slopvac prime`.
pub fn prime(m: &ArgMatches, out: &mut dyn Write) -> Result<u8, CliError> {
    let text = prime::run(m, &cli::command(), &cwd()).map_err(|e| agents_error(&e))?;
    write(out, &text)?;
    Ok(exit::PASS)
}
