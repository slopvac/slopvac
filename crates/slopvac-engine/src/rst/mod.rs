//! reStructuredText through docutils.
//!
//! RST has no Rust parser. The adapter runs a small fixed Python driver on the document.
//! The driver parses the RST with docutils and writes the prose nodes, with the source
//! lines docutils recorded for each, as JSON. The Rust side places the words of each node
//! inside that node's own lines and returns the same [`Document`] the other parsers
//! return, with byte offsets into the RST file.
//!
//! # Interpreter
//!
//! The driver runs in the first `python3`, then `python`, that can import docutils 0.21 or
//! later. [`trust`] explains which directories the search trusts. `SLOPVAC_RST_CONVERTER`
//! names an interpreter by absolute path ([`RstOptions::from_env`]). No interpreter is an
//! [`RstError::ConverterMissing`], which the engine reports as an incomplete check. A file
//! is never skipped without a word. [`RstAdapter`] resolves the interpreter once.
//!
//! # Security
//!
//! The document is untrusted. The interpreter is chosen from user-controlled places only.
//! It runs without a shell, with a fixed argument list ([`ARGS_BEFORE_DRIVER`]), the
//! embedded [`DRIVER`], a short allow-list of environment variables
//! ([`INHERITED_ENV`]), a `PATH` made of the interpreter's directory and the system
//! directories ([`restricted_path`]), an empty private working directory, a time limit and
//! a cap on the output. The driver turns off file insertion and `raw`, and reads no
//! configuration file.
//!
//! # Position precision
//!
//! The private `map` module documents it: byte exact where the words are found inside the
//! node's lines, line-precise where they are not, never dropped.
//!
//! # Protected spans
//!
//! [`RstParse::protected`] lists the source a fix must not touch. A fix may write only
//! inside prose that was placed word by word. Everything outside it is protected: literal
//! and code blocks, directive markup and options, comments, `_target:` definitions,
//! substitution definitions, table borders and any block placed by line only. Inside a
//! placed paragraph the adapter also protects inline literals, roles, footnote and
//! substitution references, the markup around inline words, a link's address, and the words
//! of a reference that name its target (`` `name`_ ``, a bare URL, or `text <url>`_ when
//! another reference uses the same name). Field names are protected too. The mapper charges
//! this work to the same meter as the placement.

mod map;
mod meter;
mod model;
mod run;
pub mod trust;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use slopvac_model::Span;
use slopvac_parse::Document;

pub use run::{ARGS_BEFORE_DRIVER, DRIVER, INHERITED_ENV, restricted_path};
use trust::{Discovery, OVERRIDE_VARIABLE, OverrideError, SkipReason, Skipped};

/// The longest conversion the adapter waits for.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// The most output the driver may write, in bytes.
pub const DEFAULT_MAX_OUTPUT: usize = 16 * 1024 * 1024;

/// How to find and run the interpreter.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RstOptions {
    /// An interpreter to use instead of searching `PATH`. Only the user may set it:
    /// [`RstOptions::from_env`] reads it from `SLOPVAC_RST_CONVERTER`, and project files
    /// never reach it.
    pub interpreter: Option<PathBuf>,
    /// The directories to search, in `PATH` syntax. `None` reads the `PATH` variable.
    pub search_path: Option<OsString>,
    /// Directories whose content the interpreter must not come from: the project being
    /// linted and the targets of the run. The search fails closed while this is empty:
    /// without it no `PATH` entry is trusted and nothing is run. An interpreter named by
    /// the user is the one exception.
    pub project_roots: Vec<PathBuf>,
    /// How long the driver may run.
    pub timeout: Duration,
    /// How many bytes the driver may write.
    pub max_output_bytes: usize,
}

impl Default for RstOptions {
    fn default() -> Self {
        Self {
            interpreter: None,
            search_path: None,
            project_roots: Vec::new(),
            timeout: DEFAULT_TIMEOUT,
            max_output_bytes: DEFAULT_MAX_OUTPUT,
        }
    }
}

impl RstOptions {
    /// The defaults, with the interpreter from `SLOPVAC_RST_CONVERTER` when it is set.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            interpreter: std::env::var_os(OVERRIDE_VARIABLE)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            ..Self::default()
        }
    }
}

/// A fact about a parsed file that the engine reports with the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RstNotice {
    /// Stable kebab-case code.
    pub code: &'static str,
    /// Whether the check is incomplete (a warning), or the notice only informs.
    pub warning: bool,
    /// The message.
    pub message: String,
}

/// An RST file, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RstParse {
    /// Blocks and code spans with byte offsets into the RST source.
    pub document: Document,
    /// The interpreter that ran.
    pub interpreter: PathBuf,
    /// The docutils version that ran.
    pub docutils: String,
    /// Facts about the precision or completeness of the result.
    pub notices: Vec<RstNotice>,
    /// Source spans a fix must not touch (see [`crate::fix_document`]): all but the words
    /// of prose placed exactly, minus the code, link targets, roles and markup inside them.
    /// Sorted by start; the spans may overlap.
    pub protected: Vec<Span>,
    /// Characters the mapper looked at. It stays under a budget that grows
    /// linearly with the size of the document.
    pub work: usize,
    /// Source bytes the mapper looked at, charged to the budget or not. The uncharged part
    /// stays a small multiple of the input.
    pub scanned: usize,
}

/// Why an RST file was not parsed. Every variant makes the check incomplete (exit 2).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RstError {
    /// No trusted interpreter can import docutils.
    #[error("{}", crate::messages::rst_missing(.detail))]
    ConverterMissing {
        /// What was searched and what was passed over.
        detail: String,
    },
    /// The interpreter could not start, or its pipes failed.
    #[error("{}", crate::messages::rst_spawn(.converter, .source))]
    Spawn {
        /// The interpreter.
        converter: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The driver did not finish in time.
    #[error("{}", crate::messages::rst_timeout(.converter, .timeout))]
    Timeout {
        /// The interpreter.
        converter: PathBuf,
        /// The limit.
        timeout: Duration,
    },
    /// The driver wrote more output than allowed.
    #[error("{}", crate::messages::rst_output_limit(.converter, *.limit))]
    OutputTooLarge {
        /// The interpreter.
        converter: PathBuf,
        /// The limit.
        limit: usize,
    },
    /// The driver exited with an error.
    #[error("{}", crate::messages::rst_failed(.converter, .status, .stderr))]
    Failed {
        /// The interpreter.
        converter: PathBuf,
        /// How it ended.
        status: String,
        /// The start of its error output.
        stderr: String,
    },
    /// The driver wrote output that is not the expected JSON.
    #[error("{}", crate::messages::rst_invalid_output(.converter, .reason))]
    InvalidOutput {
        /// The interpreter.
        converter: PathBuf,
        /// What is wrong.
        reason: String,
    },
}

impl RstError {
    /// The incomplete-check message for `paths`.
    ///
    /// A missing converter keeps the first sentence of the 2.x text, so that scripts that
    /// match it keep working, and adds the 3.0 requirement.
    #[must_use]
    pub fn message(&self, paths: &[&Path]) -> String {
        let list: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        let list = list.join(", ");
        match self {
            Self::ConverterMissing { detail } => {
                crate::messages::EngineDetails::RstMissingTargets {
                    paths: list,
                    detail: detail.clone(),
                    variable: OVERRIDE_VARIABLE.into(),
                }
                .to_string()
            }
            other => crate::messages::EngineDetails::RstUncheckedTargets {
                paths: list,
                reason: other.to_string(),
            }
            .to_string(),
        }
    }

    /// Whether the cause is a missing interpreter or docutils, as opposed to a failure.
    #[must_use]
    pub const fn is_missing(&self) -> bool {
        matches!(self, Self::ConverterMissing { .. })
    }
}

/// The interpreter, found once.
#[derive(Debug)]
struct Resolved {
    interpreter: Result<PathBuf, String>,
}

/// Parses RST files. The interpreter is resolved on the first file and reused.
#[derive(Debug)]
pub struct RstAdapter {
    options: RstOptions,
    resolved: OnceLock<Resolved>,
}

impl RstAdapter {
    /// An adapter for `options`.
    #[must_use]
    pub const fn new(options: RstOptions) -> Self {
        Self {
            options,
            resolved: OnceLock::new(),
        }
    }

    /// The interpreter that runs, or why there is none. Resolved once per adapter.
    ///
    /// # Errors
    /// Returns [`RstError::ConverterMissing`] when no trusted interpreter has docutils.
    pub fn interpreter(&self) -> Result<&Path, RstError> {
        let resolved = self.resolved.get_or_init(|| Resolved {
            interpreter: self.resolve(),
        });
        match &resolved.interpreter {
            Ok(path) => Ok(path),
            Err(detail) => Err(RstError::ConverterMissing {
                detail: detail.clone(),
            }),
        }
    }

    fn resolve(&self) -> Result<PathBuf, String> {
        if let Some(path) = &self.options.interpreter {
            return trust::check_override(path, &self.options.project_roots).map_err(
                |e: OverrideError| {
                    crate::messages::EngineDetails::RstOverride {
                        variable: OVERRIDE_VARIABLE.into(),
                        reason: e.to_string(),
                    }
                    .to_string()
                },
            );
        }
        if self.options.project_roots.is_empty() {
            // Without the project and target directories nothing tells a repository
            // directory from a user directory, so no PATH entry can be trusted, and
            // probing would already run the candidate.
            return Err(crate::messages::EnginePhrase::RstUnknownBoundaries.text());
        }
        let from_env;
        let search = if let Some(path) = &self.options.search_path {
            path
        } else {
            from_env = std::env::var_os("PATH").unwrap_or_default();
            &from_env
        };
        let Discovery {
            interpreter,
            skipped,
        } = trust::discover(search, &self.options.project_roots, run::probe);
        interpreter.ok_or_else(|| describe(&skipped))
    }

    /// Parse the RST `source`. Every position in the result is a byte offset into it.
    ///
    /// # Errors
    /// Returns an [`RstError`] when no interpreter is available, when the driver fails, or
    /// when it exceeds a limit. Each one makes the check incomplete.
    pub fn parse(&self, source: &str) -> Result<RstParse, RstError> {
        let interpreter = self.interpreter()?;
        let bom = if source.starts_with('\u{feff}') {
            '\u{feff}'.len_utf8()
        } else {
            0
        };
        let bytes = run::run(
            interpreter,
            &source.as_bytes()[bom..],
            self.options.timeout,
            self.options.max_output_bytes,
        )?;
        let output: model::Output =
            serde_json::from_slice(&bytes).map_err(|e| RstError::InvalidOutput {
                converter: interpreter.to_owned(),
                reason: e.to_string(),
            })?;
        let mapped = map::project(source, bom, &output.nodes);
        Ok(RstParse {
            document: mapped.document,
            interpreter: interpreter.to_owned(),
            docutils: output.docutils,
            notices: mapped.notices,
            protected: mapped.protected,
            work: mapped.work,
            scanned: mapped.scanned + bytes.len(),
        })
    }
}

/// Why the search found nothing.
fn describe(skipped: &[Skipped]) -> String {
    let mut text = crate::messages::EnginePhrase::RstSearchEmpty.text();
    for item in skipped {
        let note = match &item.reason {
            SkipReason::NoDocutils => crate::messages::EngineDetails::RstNoDocutils {
                path: item.path.display().to_string(),
            }
            .to_string(),
            other => crate::messages::EngineDetails::RstSkipped {
                path: item.path.display().to_string(),
                reason: other.to_string(),
            }
            .to_string(),
        };
        text.push_str("; ");
        text.push_str(&note);
    }
    text
}

/// Parse the RST `source` with a fresh adapter. Use [`RstAdapter`] to parse many files.
///
/// # Errors
/// See [`RstAdapter::parse`].
pub fn parse_rst(source: &str, options: &RstOptions) -> Result<RstParse, RstError> {
    RstAdapter::new(options.clone()).parse(source)
}

/// A span for `start..end`, which callers guarantee ascend.
pub(crate) fn span(start: usize, end: usize) -> Span {
    Span::new(start, end).expect("start <= end")
}
