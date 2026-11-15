//! The `cases` command: run a pack's rule test cases and report corpus coverage.
//!
//! `command()` returns the clap tree and `run()` executes a parsed match. The CLI crate
//! mounts them (as `pack test`); nothing here owns `main`.

use std::io::Write;
use std::path::PathBuf;

use clap::{Arg, ArgAction, ArgMatches, Command};
use slopvac_model::{GenreId, GenreResult, GenreSource, NonEmptyString, PackId, RuleId};
use slopvac_rules::{CompileOptions, LoadErrors, RuleSet};

use super::{coverage, run_cases};
use crate::{Engine, EngineError, Input, Settings};

/// Why the command could not run.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CasesError {
    /// An argument is not a valid value.
    #[error("{0}")]
    Argument(String),
    /// The pack did not load.
    #[error(transparent)]
    Load(#[from] LoadErrors),
    /// The engine failed.
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// A corpus file or the output failed.
    #[error("{path}: {source}")]
    Io {
        /// The file, or `output`.
        path: String,
        /// The cause.
        source: std::io::Error,
    },
}

/// The `cases` command tree.
#[must_use]
pub fn command() -> Command {
    Command::new("cases")
        .about("Run the test cases of a rule pack")
        .arg(
            Arg::new("dir")
                .required(true)
                .value_name("DIR")
                .help("Directory of category files"),
        )
        .arg(
            Arg::new("pack")
                .long("pack")
                .value_name("ID")
                .default_value("core")
                .help("Pack id of the rules"),
        )
        .arg(
            Arg::new("rule")
                .long("rule")
                .value_name("ID")
                .help("Run one rule only"),
        )
        .arg(
            Arg::new("coverage")
                .long("coverage")
                .value_name("FILE")
                .action(ArgAction::Append)
                .help("Also report the rules that match nothing in these Markdown files"),
        )
}

/// Run a parsed `cases` command and write the report to `out`.
///
/// Returns the exit code: 0 when every case passes, 1 when one fails. Unmatched rules in a
/// coverage report do not change it.
pub fn run(matches: &ArgMatches, out: &mut dyn Write) -> Result<i32, CasesError> {
    let text = |name: &str| matches.get_one::<String>(name);
    let pack = PackId::new(text("pack").map_or("core", String::as_str))
        .map_err(|e| CasesError::Argument(format!("--pack: {e}")))?;
    let only = text("rule")
        .map(|r| RuleId::new(r.as_str()))
        .transpose()
        .map_err(|e| CasesError::Argument(format!("--rule: {e}")))?;
    let dir = PathBuf::from(text("dir").map_or("", String::as_str));
    let set = RuleSet::load_dir(&pack, &dir, &CompileOptions::default())?;
    let engine = Engine::new(vec![set], default_settings()?)?;

    let report = run_cases(&engine, only.as_ref())?;
    emit(out, &report.to_string())?;
    let files: Vec<&String> = matches
        .get_many::<String>("coverage")
        .into_iter()
        .flatten()
        .collect();
    if !files.is_empty() {
        let sources = files
            .iter()
            .map(|p| {
                std::fs::read_to_string(p).map_err(|source| CasesError::Io {
                    path: (*p).clone(),
                    source,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let inputs: Vec<Input<'_>> = files
            .iter()
            .zip(&sources)
            .map(|(p, s)| Input::markdown(p, s))
            .collect();
        emit(out, &coverage(&engine, &inputs)?.to_string())?;
    }
    Ok(i32::from(!report.is_clean()))
}

fn emit(out: &mut dyn Write, text: &str) -> Result<(), CasesError> {
    writeln!(out, "{text}").map_err(|source| CasesError::Io {
        path: "output".into(),
        source,
    })
}

/// The settings that cases run under: the `normal` profile and the `reference` genre.
pub fn default_settings() -> Result<Settings, CasesError> {
    let bad = |e: &dyn std::fmt::Display| CasesError::Argument(e.to_string());
    Ok(Settings::new(
        NonEmptyString::new("normal").map_err(|e| bad(&e))?,
        GenreResult {
            id: GenreId::new("reference").map_err(|e| bad(&e))?,
            source: GenreSource::Default,
            reasons: Vec::new(),
        },
    ))
}
