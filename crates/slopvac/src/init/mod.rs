//! `slopvac init`: write a commented `slopvac.toml` and set up agent harnesses
//! (compat rows `cmd:init`, `opt:init:--force`, `--path`, `--skip-agents`, `--harness`,
//! `--no-skills`, `opt:init:--profile` (removed), `behavior:init-flags`,
//! `behavior:init-idempotence`).
//!
//! The template is `templates/slopvac.toml`. A test regenerates it from
//! `schemas/v3/config.schema.json` and fails when the two differ, so it cannot drift.

use std::io::Write;
use std::path::{Path, PathBuf};

use clap::ArgMatches;
use slopvac_agents::{harness, setup};

use crate::errors::CliError;
use crate::{exit, redact};

#[cfg(test)]
mod template;

/// The checked-in starter config.
pub const TEMPLATE: &str = include_str!("../../templates/slopvac.toml");

/// The harness used when `--harness` is not given.
pub const DEFAULT_HARNESS: &str = "agents";

/// The `--harness` values.
#[must_use]
pub fn harness_names() -> Vec<&'static str> {
    harness::names()
}

/// How a path appears in a message: as given when relative, redacted when absolute.
fn shown(path: &Path) -> String {
    if path.is_absolute() {
        redact::path_for_log(path, &std::env::current_dir().unwrap_or_default())
    } else {
        path.display().to_string()
    }
}

fn io_error(path: &Path, err: &std::io::Error) -> CliError {
    CliError::new(
        "output-failed",
        format!("{}: cannot write: {}", shown(path), err.kind()),
    )
}

/// Run `slopvac init`.
///
/// # Errors
/// A usage error for `--skip-agents` with harness options or for a directory
/// as `--path`; `output-failed` when the file cannot be written; the setup
/// error code when a harness fails.
pub fn run(m: &ArgMatches, out: &mut dyn Write) -> Result<u8, CliError> {
    if m.contains_id("profile")
        && let Some((_, message)) = crate::cli::REMOVED_LINT_OPTIONS
            .iter()
            .find(|(name, _)| *name == "profile")
    {
        return Err(CliError::new("removed-option", *message));
    }
    let path = m
        .get_one::<PathBuf>("path")
        .cloned()
        .unwrap_or_else(|| PathBuf::from("slopvac.toml"));
    let harnesses: Vec<String> = m
        .get_many::<String>("harness")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();
    let no_skills = m.get_flag("no-skills");
    let skip_agents = m.get_flag("skip-agents");
    if skip_agents && (!harnesses.is_empty() || no_skills) {
        return Err(CliError::new(
            "usage",
            "--skip-agents writes no steering; drop --harness and --no-skills.",
        ));
    }
    let say = |out: &mut dyn Write, line: String| {
        writeln!(out, "{line}").map_err(|err| {
            CliError::new(
                "output-failed",
                format!("cannot write output: {}", err.kind()),
            )
        })
    };
    if path.is_dir() {
        return Err(CliError::new(
            "usage",
            format!("--path {} is a directory.", shown(&path)),
        ));
    }
    if path.exists() && !m.get_flag("force") {
        say(out, format!("{} exists; keeping it.", shown(&path)))?;
    } else {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| io_error(&path, &e))?;
        }
        std::fs::write(&path, TEMPLATE).map_err(|e| io_error(&path, &e))?;
        say(out, format!("wrote {}", shown(&path)))?;
    }
    if !skip_agents {
        let mut chosen: Vec<String> = Vec::new();
        for h in harnesses {
            if !chosen.contains(&h) {
                chosen.push(h);
            }
        }
        if chosen.is_empty() {
            chosen.push(DEFAULT_HARNESS.to_owned());
        }
        say(out, format!("harnesses: {}", chosen.join(", ")))?;
        let root = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        for name in chosen {
            let outcome = setup::execute(&setup::Options {
                harness: Some(name),
                list: false,
                remove: false,
                check: false,
                root: root.clone(),
                block: slopvac_agents::content::BlockKind::Standard,
                no_skill: no_skills,
                agent: false,
                reviewer_model: None,
            })
            .map_err(|err| CliError::new(err.code(), err.to_string()))?;
            for line in outcome.lines {
                say(out, line)?;
            }
        }
    }
    say(out, "next: slopvac prime".to_owned())?;
    Ok(exit::PASS)
}
