//! The `slopvac migrate` command.

use std::io::Write;
use std::path::PathBuf;

use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};

use crate::plan::{BACKUP_DIR, apply, plan, render_diff, restore};
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
                .help("Keep the 2.x vocabulary behavior by adding the STE style"),
        )
        .arg(
            Arg::new("restore")
                .long("restore")
                .action(ArgAction::SetTrue)
                .conflicts_with_all(["write", "preserve-behavior"])
                .help("Undo the last --write from the backup"),
        )
}

/// Run `slopvac migrate`. Writes to `out` and `err`; returns the exit code:
/// 0 done or nothing to do, 2 incomplete (a setting 3.0 cannot represent, or any
/// other failure).
pub fn run(
    matches: &ArgMatches,
    linter: &dyn Linter,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    match execute(matches, linter, out) {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "error[{}]: {e}", e.code());
            2
        }
    }
}

/// Run `slopvac migrate` and return the failure for the caller to render. Paths in
/// the error are relative to the project root.
pub fn execute(
    matches: &ArgMatches,
    linter: &dyn Linter,
    out: &mut dyn Write,
) -> Result<(), Error> {
    let root = matches
        .get_one::<PathBuf>("path")
        .cloned()
        .unwrap_or_else(|| PathBuf::from("."));
    execute_in(matches, linter, out, &root).map_err(|e| e.redacted(&root))
}

fn execute_in(
    matches: &ArgMatches,
    linter: &dyn Linter,
    out: &mut dyn Write,
    root: &std::path::Path,
) -> Result<(), Error> {
    let root = root.to_owned();
    let io = |e: std::io::Error| Error::io(&root, e);
    if matches.get_flag("restore") {
        let n = restore(&root)?;
        writeln!(out, "Restored {n} files from the backup.").map_err(io)?;
        return Ok(());
    }
    let options = Options {
        preserve_behavior: matches.get_flag("preserve-behavior"),
    };
    let renames = RenameTable::bundled();
    let plan = plan(&root, &renames, options)?;
    for warning in &plan.warnings {
        writeln!(out, "warning: {warning}").map_err(io)?;
    }
    if plan.is_empty() {
        writeln!(out, "Nothing to migrate.").map_err(io)?;
        return Ok(());
    }
    write!(out, "{}", render_diff(&plan)).map_err(io)?;
    for note in &plan.notes {
        writeln!(out, "note: {note}").map_err(io)?;
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
            writeln!(out, "  {}: {} -> {}", r.rule, r.before, r.after).map_err(io)?;
        }
    }
    if matches.get_flag("write") {
        apply(&root, &plan)?;
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
