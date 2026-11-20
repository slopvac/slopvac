//! The `slopvac migrate` command.

use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};
use slopvac_sanitize as sanitize;

use crate::backup::{apply, restore};
use crate::plan::{BACKUP_DIR, plan, render_diff};
use crate::{Error, Linter, Options, Phase, RenameTable, compare};

/// The clap definition of `slopvac migrate`.
#[must_use]
pub fn command() -> Command {
    Command::new("migrate")
        .about("Migrate a 2.x project to 3.0: config, inline directives and steering blocks")
        .long_about(
            "Shows a diff of every change by default. With --write, rewrites the files \
             and keeps the originals in .slopvac-migrate-backup. Lints before and after \
             and reports the change per rule.",
        )
        .arg(
            Arg::new("path")
                .help("Project root")
                .default_value(".")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("write")
                .long("write")
                .action(ArgAction::SetTrue)
                .help("Apply the changes instead of previewing them"),
        )
        .arg(
            Arg::new("preserve-behavior")
                .long("preserve-behavior")
                .action(ArgAction::SetTrue)
                .help("Reproduce the 2.x profile gates with the core:legacy-<profile> style"),
        )
        .arg(
            Arg::new("restore")
                .long("restore")
                .action(ArgAction::SetTrue)
                .conflicts_with_all(["write", "preserve-behavior"])
                .help("Undo the last --write from the backup"),
        )
        .arg(
            Arg::new("force")
                .long("force")
                .action(ArgAction::SetTrue)
                .requires("restore")
                .help("With --restore, also put back files that changed since the write"),
        )
}

/// Run `slopvac migrate`. `steering` is the 3.0 managed block from `slopvac setup`.
/// `state` is the per-user cache directory, where `--write` records the backup that
/// `--restore` accepts. Writes to `out` and `err`; returns the exit code:
/// 0 done or nothing to do, 2 incomplete (a setting 3.0 cannot represent, or any
/// other failure).
pub fn run(
    matches: &ArgMatches,
    linter: &dyn Linter,
    steering: &str,
    state: &Path,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    match execute(matches, linter, steering, state, out) {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "error[{}]: {e}", e.code());
            2
        }
    }
}

// TODO(slopvac-8l4.12.42): sanitize at the binary boundary and delete this wrapper. Until
// then it escapes once, here, so the binary can print the message as it is.
/// The failure of [`execute`], for the CLI to print.
///
/// Its `Display` is the error message escaped for a terminal, one line per record: a list of
/// dropped settings keeps one line per setting, and every other message is one line.
/// [`source`](std::error::Error::source) is the raw [`Error`], which library callers should
/// match on.
#[derive(Debug)]
pub struct CliFailure {
    code: &'static str,
    source: Error,
}

impl CliFailure {
    /// The stable error code of the failure.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for CliFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Each record keeps the line the error gives it, and nothing inside a record can
        // start another: `line` escapes newline and tab as well.
        match &self.source {
            Error::Dropped(records) => {
                let lines: Vec<String> = records
                    .iter()
                    .map(|r| sanitize::line(&r.to_string()))
                    .collect();
                f.write_str(&lines.join(crate::DROPPED_SEPARATOR))
            }
            other => f.write_str(&sanitize::line(&other.to_string())),
        }
    }
}

impl std::error::Error for CliFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Run `slopvac migrate` and return the failure for the caller to print. Paths in the
/// failure are relative to the project root. `state` is as for [`run`].
///
/// # Errors
/// A [`CliFailure`] holding the [`Error`] that stopped the run.
pub fn execute(
    matches: &ArgMatches,
    linter: &dyn Linter,
    steering: &str,
    state: &Path,
    out: &mut dyn Write,
) -> Result<(), CliFailure> {
    let root = matches
        .get_one::<PathBuf>("path")
        .cloned()
        .unwrap_or_else(|| PathBuf::from("."));
    execute_in(matches, linter, steering, state, out, &root)
        .map_err(|e| e.redacted(&root))
        .map_err(|source| CliFailure {
            code: source.code(),
            source,
        })
}

fn execute_in(
    matches: &ArgMatches,
    linter: &dyn Linter,
    steering: &str,
    state: &Path,
    out: &mut dyn Write,
    root: &Path,
) -> Result<(), Error> {
    let root = root.to_owned();
    let io = |e: std::io::Error| Error::io(&root, e);
    if matches.get_flag("restore") {
        let done = restore(&root, state, matches.get_flag("force"))?;
        for kept in &done.kept {
            writeln!(
                out,
                "kept {}: it changed after the migration, so the restore left it as it is",
                slopvac_sanitize::line(&kept.display().to_string())
            )
            .map_err(io)?;
        }
        writeln!(out, "Restored {} files from the backup.", done.restored).map_err(io)?;
        return Ok(());
    }
    let options = Options {
        preserve_behavior: matches.get_flag("preserve-behavior"),
    };
    let renames = RenameTable::bundled();
    let plan = plan(&root, &renames, options, steering)?;
    for warning in &plan.warnings {
        writeln!(out, "warning: {}", sanitize::line(warning)).map_err(io)?;
    }
    if plan.is_empty() {
        writeln!(out, "Nothing to migrate.").map_err(io)?;
        return Ok(());
    }
    write!(out, "{}", sanitize::terminal(&render_diff(&plan))).map_err(io)?;
    for note in &plan.notes {
        writeln!(out, "note: {}", sanitize::line(note)).map_err(io)?;
    }
    let before = linter.lint(&root, Phase::Before, &[])?;
    let after = linter.lint(&root, Phase::After, &plan.overlay(&root))?;
    let rows = compare(&before, &after, &renames);
    let changed: Vec<_> = rows.iter().filter(|r| r.changed()).collect();
    if changed.is_empty() {
        writeln!(out, "Findings: no per-rule change.").map_err(io)?;
    } else {
        writeln!(out, "Findings by rule (before -> after):").map_err(io)?;
        for r in changed {
            writeln!(
                out,
                "  {}: {} -> {}",
                sanitize::line(&r.rule),
                r.before,
                r.after
            )
            .map_err(io)?;
        }
    }
    if matches.get_flag("write") {
        apply(&root, &plan, state)?;
        writeln!(
            out,
            "Wrote {} files. Originals are in {BACKUP_DIR}; undo with `slopvac migrate --restore`.",
            plan.changes.len(),
        )
        .map_err(io)?;
    } else {
        writeln!(out, "Preview only. Run again with --write to apply.").map_err(io)?;
    }
    Ok(())
}
