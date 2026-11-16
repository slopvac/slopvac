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
    let (code, message) = match err {
        TermsError::Io { path, message } => (
            "terms-io",
            format!(
                "cannot read {}: {}",
                shown(path),
                redact::redact_root(message, &cwd())
            ),
        ),
        TermsError::Parse { path, message } => (
            "terms-parse",
            format!(
                "cannot parse {}: {}",
                shown(path),
                redact::redact_root(message, &cwd())
            ),
        ),
        TermsError::SchemaVersion { path, found } => (
            "terms-schema-version",
            format!(
                "{}: schema_version {found} is not supported, expected 3",
                shown(path)
            ),
        ),
        TermsError::UnknownKey { path, key } => (
            "terms-unknown-key",
            format!("{}: unknown key `{key}`", shown(path)),
        ),
        TermsError::Entry { path, message } => {
            ("terms-entry", format!("{}: {message}", shown(path)))
        }
    };
    CliError::new(code, message)
}

fn agents_error(err: &AgentsError) -> CliError {
    CliError::new(err.code(), redact::redact_root(&err.to_string(), &cwd()))
}

/// `slopvac terms`.
pub fn terms(m: &ArgMatches, out: &mut dyn Write) -> Result<u8, CliError> {
    slopvac_terms::cli::run(m, &cwd(), out)
        .map(exit_code)
        .map_err(|e| terms_error(&e))
}

/// `slopvac genre`.
pub fn genre(m: &ArgMatches, out: &mut dyn Write) -> Result<u8, CliError> {
    let text = slopvac_context::run(m).map_err(|e| CliError::new("genre-failed", e.to_string()))?;
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
