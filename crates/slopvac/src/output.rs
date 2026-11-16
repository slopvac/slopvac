//! Deliver a lint report: `--format` picks the renderer, `--out` the destination
//! (compat rows `opt:lint:--format`, `opt:lint:--out`, `behavior:report-out-open`).

use std::io::Write as _;
use std::path::{Path, PathBuf};

use clap::{ArgMatches, parser::ValueSource};
use slopvac_report::{Format, render};

use crate::engine_stub::RunOutcome;
use crate::errors::CliError;

/// The format a run uses.
///
/// As in 2.x, `--out` without an explicit `--format` writes an HTML page.
///
/// # Panics
/// When the clap tree offers a format that `slopvac-report` does not know.
#[must_use]
pub fn chosen_format(m: &ArgMatches) -> Format {
    let given = m.value_source("format") == Some(ValueSource::CommandLine);
    let name = if m.get_one::<PathBuf>("out").is_some() && !given {
        "html"
    } else {
        m.get_one::<String>("format").map_or("text", String::as_str)
    };
    Format::from_name(name).expect("every --format value has a renderer")
}

/// Render the outcome and send it to standard output or the `--out` file.
///
/// With `--out` the file gets the report and standard output gets one
/// `report: <path>` line.
///
/// # Errors
/// `render-failed` when a format cannot render, `output-failed` when writing fails.
pub fn deliver(
    m: &ArgMatches,
    outcome: &RunOutcome,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    let rendered = render(chosen_format(m), &outcome.report, &outcome.sources)
        .map_err(|err| CliError::new("render-failed", err.to_string()))?;
    let failed = |err: std::io::Error| {
        CliError::new(
            "output-failed",
            format!("cannot write the report: {}", err.kind()),
        )
    };
    if let Some(path) = m.get_one::<PathBuf>("out") {
        write_file(path, &rendered).map_err(failed)?;
        writeln!(out, "report: {}", path.display()).map_err(failed)?;
    } else {
        out.write_all(rendered.as_bytes()).map_err(failed)?;
    }
    out.flush().map_err(failed)
}

/// Replace `path` atomically: write a temp file beside it, then rename over it
/// (policies 1.8), so a failed write leaves the previous report intact.
fn write_file(path: &Path, text: &str) -> std::io::Result<()> {
    let dir = match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(parent) => {
            std::fs::create_dir_all(parent)?;
            parent
        }
        None => Path::new("."),
    };
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(text.as_bytes())?;
    file.persist(path).map_err(|err| err.error)?;
    Ok(())
}
