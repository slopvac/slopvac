//! reStructuredText through a docutils adapter.
//!
//! RST has no Rust parser. The adapter hands the document to docutils, which renders it as
//! HTML, reads that HTML with [`slopvac_parse::parse_html`], and then maps every position
//! back into the RST source. The result is the [`Document`] type the other parsers return,
//! with source offsets into the RST file.
//!
//! # Converter
//!
//! The 2.x release looked for `rst2html` and then `rst2html.py` on `PATH`. So does
//! [`find_converter`]. A caller may name a converter in [`RstOptions::converter`]; that
//! value comes from the user's own configuration or command line, never from the document.
//! [`RstParse::converter`] reports which executable ran. No converter is an
//! [`RstError::ConverterMissing`], which the caller reports as an incomplete check (exit 2).
//! A file is never skipped without a word.
//!
//! # Security
//!
//! The document is untrusted. The converter runs without a shell, with a fixed argument
//! list that turns off `include`, `raw` and every other directive that reads a file or a
//! URL, with the repository's `docutils.conf` out of reach, with a short list of inherited
//! environment variables, a time limit and a cap on the output.
//!
//! # Position precision
//!
//! Positions are byte exact where the text of the HTML can be found in the RST source.
//! The private `map` module documents the matching rules and what falls back to a synthetic
//! space.

mod map;
mod run;

pub use run::ARGS as CONVERTER_ARGS;

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use slopvac_model::Span;
use slopvac_parse::{Document, ParseError};

/// Converter names, in the order 2.x tried them.
pub const CONVERTER_NAMES: [&str; 2] = ["rst2html", "rst2html.py"];

/// The longest conversion the adapter waits for.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// The most HTML the adapter accepts from the converter, in bytes. It equals the HTML
/// parser's own limit.
pub const DEFAULT_MAX_OUTPUT: usize = 16 * 1024 * 1024;

/// How to find and run the converter.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RstOptions {
    /// An executable to use instead of searching `PATH`. Only the user's configuration or
    /// command line may set it.
    pub converter: Option<PathBuf>,
    /// The directories to search, in `PATH` syntax. `None` reads the `PATH` variable.
    pub search_path: Option<OsString>,
    /// How long the converter may run.
    pub timeout: Duration,
    /// How many bytes of HTML the converter may write.
    pub max_output_bytes: usize,
}

impl Default for RstOptions {
    fn default() -> Self {
        Self {
            converter: None,
            search_path: None,
            timeout: DEFAULT_TIMEOUT,
            max_output_bytes: DEFAULT_MAX_OUTPUT,
        }
    }
}

/// An RST file, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RstParse {
    /// Blocks and code spans with byte offsets into the RST source.
    pub document: Document,
    /// The converter that ran.
    pub converter: PathBuf,
}

/// Why an RST file was not parsed. Every variant makes the check incomplete (exit 2).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RstError {
    /// Neither `rst2html` nor `rst2html.py` is on the search path.
    #[error("rst2html or rst2html.py not found on PATH; install with `pip install docutils`")]
    ConverterMissing,
    /// The converter could not start, or its pipes failed.
    #[error("cannot run the RST converter {}: {source}", .converter.display())]
    Spawn {
        /// The executable.
        converter: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The converter did not finish in time.
    #[error("the RST converter {} took longer than {timeout:?}", .converter.display())]
    Timeout {
        /// The executable.
        converter: PathBuf,
        /// The limit.
        timeout: Duration,
    },
    /// The converter wrote more HTML than allowed.
    #[error("the RST converter {} wrote more than {limit} bytes", .converter.display())]
    OutputTooLarge {
        /// The executable.
        converter: PathBuf,
        /// The limit.
        limit: usize,
    },
    /// The converter exited with an error.
    #[error("the RST converter {} failed ({status}): {stderr}", .converter.display())]
    Failed {
        /// The executable.
        converter: PathBuf,
        /// How it ended.
        status: String,
        /// The start of its error output.
        stderr: String,
    },
    /// The converter wrote output that is not UTF-8.
    #[error("the RST converter {} wrote output that is not UTF-8", .converter.display())]
    InvalidOutput {
        /// The executable.
        converter: PathBuf,
    },
    /// The HTML was too large or too costly to read.
    #[error("cannot read the converted HTML: {0}")]
    Html(#[from] ParseError),
    /// Mapping positions back to the RST source needed more work than allowed.
    #[error("the document needs more work than the position mapping allows")]
    MappingBudget,
}

impl RstError {
    /// The incomplete-check message for `paths`.
    ///
    /// A missing converter keeps the 2.x text, so that scripts that match it keep working.
    #[must_use]
    pub fn message(&self, paths: &[&Path]) -> String {
        match self {
            Self::ConverterMissing => {
                let list: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
                format!(
                    "RST target(s) skipped: rst2html or rst2html.py not found on PATH ({}); install with `pip install docutils`.",
                    list.join(", ")
                )
            }
            other => {
                let list: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
                format!("RST target(s) not checked ({}): {other}", list.join(", "))
            }
        }
    }
}

/// Find the first of [`CONVERTER_NAMES`] in `search_path`, or in the `PATH` variable.
#[must_use]
pub fn find_converter(search_path: Option<&OsStr>) -> Option<PathBuf> {
    let from_env;
    let search_path = if let Some(path) = search_path {
        path
    } else {
        from_env = std::env::var_os("PATH")?;
        &from_env
    };
    let dirs: Vec<PathBuf> = std::env::split_paths(search_path).collect();
    CONVERTER_NAMES.iter().find_map(|name| {
        dirs.iter()
            .filter(|dir| !dir.as_os_str().is_empty())
            .find_map(|dir| executable_in(dir, name))
    })
}

/// `dir/name` when it is an executable file.
fn executable_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let candidates: &[&str] = if cfg!(windows) { &["", ".exe"] } else { &[""] };
    candidates.iter().find_map(|extension| {
        let path = dir.join(format!("{name}{extension}"));
        is_executable(&path).then_some(path)
    })
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file())
}

/// Parse the RST `source`.
///
/// Every position in the result is a byte offset into `source`.
///
/// # Errors
/// Returns an [`RstError`] when no converter is available, when it fails, or when the
/// document exceeds a limit. Each one makes the check incomplete.
pub fn parse_rst(source: &str, options: &RstOptions) -> Result<RstParse, RstError> {
    let converter = match &options.converter {
        Some(path) => path.clone(),
        None => find_converter(options.search_path.as_deref()).ok_or(RstError::ConverterMissing)?,
    };
    let bom = if source.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    let output = run::run(
        &converter,
        &source.as_bytes()[bom..],
        options.timeout,
        options.max_output_bytes,
    )?;
    let html = String::from_utf8(output).map_err(|_| RstError::InvalidOutput {
        converter: converter.clone(),
    })?;
    let document = map::project(source, bom, &html)?;
    Ok(RstParse {
        document,
        converter,
    })
}

/// A span for `start..end`, which callers guarantee ascend.
pub(crate) fn span(start: usize, end: usize) -> Span {
    Span::new(start, end).expect("start <= end")
}
