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
    #[error("{}", crate::messages::git_error(.0))]
    Argument(String),
    /// The pack did not load.
    #[error(transparent)]
    Load(#[from] LoadErrors),
    /// The engine failed.
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// The run could not start, for example on an unknown rule id.
    #[error(transparent)]
    Cases(#[from] super::CasesError),
    /// A corpus file is larger than the document cap. Its stable code is `document-too-large`.
    #[error("{}", crate::messages::document_too_large(.path.clone(), *.cap))]
    TooLarge {
        /// The file.
        path: String,
        /// The cap that the file exceeded, in bytes.
        cap: u64,
    },
    /// A corpus file or the output failed.
    #[error("{}", crate::messages::path_error(.path, .source))]
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
        .about(crate::messages::EnginePhrase::CasesAbout.as_static())
        .arg(
            Arg::new("dir")
                .required(true)
                .value_name("DIR")
                .help(crate::messages::EnginePhrase::CasesDirHelp.as_static()),
        )
        .arg(
            Arg::new("plugin")
                .long("plugin")
                .value_name("ID")
                .default_value("core")
                .help(crate::messages::EnginePhrase::CasesPluginHelp.as_static()),
        )
        .arg(
            Arg::new("rule")
                .long("rule")
                .value_name("ID")
                .help(crate::messages::EnginePhrase::CasesRuleHelp.as_static()),
        )
        .arg(
            Arg::new("coverage")
                .long("coverage")
                .value_name("FILE")
                .action(ArgAction::Append)
                .help(crate::messages::EnginePhrase::CasesCoverageHelp.as_static()),
        )
}

/// Run a parsed `cases` command and write the report to `out`.
///
/// Returns the exit code: 0 when every check passes, 1 when a case or example fails, 2 when
/// none failed but a rule went unchecked, which the output reports as the warning notice
/// `cases-rule-unchecked`. An `Err` is invalid input, which the caller maps to exit 2 as well.
/// Unmatched rules in a coverage report do not change the code.
pub fn run(matches: &ArgMatches, out: &mut dyn Write) -> Result<i32, CasesError> {
    let text = |name: &str| matches.get_one::<String>(name);
    let pack = PackId::new(text("plugin").map_or("core", String::as_str)).map_err(|e| {
        CasesError::Argument(
            crate::messages::EngineDetails::CaseArgument {
                argument: "--plugin".into(),
                reason: e.to_string(),
            }
            .to_string(),
        )
    })?;
    let only = text("rule")
        .map(|r| RuleId::new(r.as_str()))
        .transpose()
        .map_err(|e| {
            CasesError::Argument(
                crate::messages::EngineDetails::CaseArgument {
                    argument: "--rule".into(),
                    reason: e.to_string(),
                }
                .to_string(),
            )
        })?;
    let dir = PathBuf::from(text("dir").map_or("", String::as_str));
    let set = RuleSet::load_dir(&pack, &dir, &CompileOptions::default())?;
    let engine = Engine::new(vec![set], default_settings()?)?;

    let report = run_cases(&engine, only.as_ref())?;
    emit(out, &report.to_string())?;
    if let Some(notice) = report.unchecked_notice() {
        emit(
            out,
            &format!("warning[{}]: {}", notice.code, notice.message),
        )?;
    }
    let files: Vec<&String> = matches
        .get_many::<String>("coverage")
        .into_iter()
        .flatten()
        .collect();
    if !files.is_empty() {
        let sources = files
            .iter()
            .map(|p| {
                let shown = (*p).clone();
                // The reader refuses a symlink, but a linked corpus file is supported. Resolve
                // the link first; a missing file or a dangling link is a not-found error.
                let resolved = std::fs::canonicalize(p).map_err(|source| CasesError::Io {
                    path: shown.clone(),
                    source,
                })?;
                slopvac_fs::read_capped_regular_to_string(
                    &resolved,
                    crate::MAX_FILE_BYTES,
                    slopvac_fs::ReadOptions::new(),
                )
                .map_err(|e| match e {
                    slopvac_fs::Error::Input {
                        kind: slopvac_fs::InputKind::TooLarge,
                        ..
                    } => CasesError::TooLarge {
                        path: shown,
                        cap: crate::MAX_FILE_BYTES,
                    },
                    slopvac_fs::Error::Input {
                        kind: slopvac_fs::InputKind::NotFound,
                        ..
                    } => CasesError::Io {
                        path: shown,
                        source: std::io::ErrorKind::NotFound.into(),
                    },
                    slopvac_fs::Error::Input {
                        source: Some(source),
                        ..
                    } => CasesError::Io {
                        path: shown,
                        source,
                    },
                    other => CasesError::Io {
                        path: shown,
                        source: std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            other.to_string(),
                        ),
                    },
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
    Ok(report.exit_code())
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
