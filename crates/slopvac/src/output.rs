//! Deliver a lint report: `--format` picks the renderer, `--out` the destination
//! (compat rows `opt:lint:--format`, `opt:lint:--out`, `behavior:report-out-open`).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::{ArgMatches, parser::ValueSource};
use slopvac_report::{Format, render};

use crate::engine_stub::RunOutcome;
use crate::errors::CliError;

/// Shows a written report to the user.
pub trait Opener {
    /// Open `path` with the platform's default application.
    ///
    /// # Errors
    /// When no opener exists or it cannot start.
    fn open(&self, path: &Path) -> std::io::Result<()>;
}

/// The platform opener, started directly with no shell: `open` on macOS, `xdg-open` on Linux.
/// The program is looked up on `PATH`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&self, path: &Path) -> std::io::Result<()> {
        let program = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(target_os = "linux") {
            "xdg-open"
        } else {
            return Err(std::io::Error::other(
                "no opener is known for this platform",
            ));
        };
        let status = Command::new(program)
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "{program} exited with {status}"
            )))
        }
    }
}

/// The format a run uses.
///
/// As in 2.x, `--out` without an explicit `--format` writes an HTML page, and so does
/// `--open`.
///
/// # Panics
/// When the clap tree offers a format that `slopvac-report` does not know.
#[must_use]
pub fn chosen_format(m: &ArgMatches) -> Format {
    let given = m.value_source("format") == Some(ValueSource::CommandLine);
    let name = if (m.get_one::<PathBuf>("out").is_some() || m.get_flag("open")) && !given {
        "html"
    } else {
        m.get_one::<String>("format").map_or("text", String::as_str)
    };
    Format::from_name(name).expect("every --format value has a renderer")
}

/// Render the outcome and send it to standard output or the `--out` file.
///
/// With `--out` the file gets the report and standard output gets one
/// `report: <path>` line. `--open` shows the written report; without `--out` it writes the
/// HTML page to a temporary file first, as 2.x did.
///
/// # Errors
/// `render-failed` when a format cannot render, `output-failed` when writing fails, `usage`
/// when `--open` is combined with a format other than `html`.
pub fn deliver(
    m: &ArgMatches,
    outcome: &RunOutcome,
    out: &mut dyn std::io::Write,
    err_out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    deliver_with(
        m,
        outcome,
        out,
        err_out,
        &SystemOpener,
        std::env::var_os("CI").is_some(),
    )
}

/// [`deliver`] with the opener and the CI switch supplied.
///
/// # Errors
/// As [`deliver`].
pub fn deliver_with(
    m: &ArgMatches,
    outcome: &RunOutcome,
    out: &mut dyn std::io::Write,
    err_out: &mut dyn std::io::Write,
    opener: &dyn Opener,
    ci: bool,
) -> Result<(), CliError> {
    let open = m.get_flag("open");
    let format = chosen_format(m);
    if open && format != Format::Html {
        return Err(CliError::new(
            "usage",
            "--open shows an HTML report; drop --format or use --format html.",
        ));
    }
    let rendered = render(format, &outcome.report, &outcome.sources)
        .map_err(|err| CliError::new("render-failed", err.to_string()))?;
    let failed = |err: std::io::Error| {
        CliError::new(
            "output-failed",
            format!("cannot write the report: {}", err.kind()),
        )
    };
    let target = match m.get_one::<PathBuf>("out") {
        Some(path) => Some(path.clone()),
        None if open => Some(temp_report().map_err(failed)?),
        None => None,
    };
    let Some(path) = target else {
        out.write_all(rendered.as_bytes()).map_err(failed)?;
        return out.flush().map_err(failed);
    };
    write_file(&path, &rendered).map_err(failed)?;
    writeln!(out, "report: {}", path.display()).map_err(failed)?;
    out.flush().map_err(failed)?;
    if open {
        show(&path, opener, ci, err_out);
    }
    Ok(())
}

/// Open the report unless CI is on. A refusal or a failure is a warning: the report is
/// already written, so the run's outcome does not change.
fn show(path: &Path, opener: &dyn Opener, ci: bool, err_out: &mut dyn std::io::Write) {
    if ci {
        let _ = writeln!(err_out, "not opening the report: CI is set.");
    } else if let Err(err) = opener.open(&absolute(path)) {
        let _ = writeln!(err_out, "warning: could not open the report: {err}");
    }
}

/// The path to hand the opener. It is absolute, so a name that starts with `-` cannot be read
/// as an option by `open` or `xdg-open`.
fn absolute(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| {
        std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
    })
}

/// An empty file the HTML page can be written over, kept after the process exits.
fn temp_report() -> std::io::Result<PathBuf> {
    let file = tempfile::Builder::new()
        .prefix("slopvac-report-")
        .suffix(".html")
        .tempfile()?;
    let (_, path) = file.keep().map_err(|err| err.error)?;
    Ok(path)
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
