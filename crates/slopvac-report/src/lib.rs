//! Finding output formats.
//!
//! One [`Report`] renders to every format. A format that needs a
//! column in another unit than code points reads the original text through [`Sources`].

mod checkstyle;
mod codeclimate;
mod escape;
mod github;
mod html;
mod junit;
mod locate;
mod markdown;
mod rdjsonl;
mod sarif;
mod text;

use std::collections::HashMap;

use slopvac_model::{PositionError, Report};

pub use locate::{Located, Pos};

/// An output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Human-readable lines.
    Text,
    /// The v3 lint report.
    Json,
    /// GitHub Actions annotations.
    Github,
    /// SARIF 2.1.0.
    Sarif,
    /// reviewdog diagnostics, one JSON object per line.
    Rdjsonl,
    /// Code Climate issues.
    CodeClimate,
    /// `JUnit` XML.
    Junit,
    /// Checkstyle XML.
    Checkstyle,
    /// A Markdown summary.
    Markdown,
    /// A standalone HTML page.
    Html,
}

impl Format {
    /// Every format, in help order.
    pub const ALL: [Self; 10] = [
        Self::Text,
        Self::Json,
        Self::Github,
        Self::Sarif,
        Self::Rdjsonl,
        Self::CodeClimate,
        Self::Junit,
        Self::Checkstyle,
        Self::Markdown,
        Self::Html,
    ];

    /// The name used with `--format`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
            Self::Github => "github",
            Self::Sarif => "sarif",
            Self::Rdjsonl => "rdjsonl",
            Self::CodeClimate => "codeclimate",
            Self::Junit => "junit",
            Self::Checkstyle => "checkstyle",
            Self::Markdown => "markdown",
            Self::Html => "html",
        }
    }

    /// Parse a `--format` value.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.name() == name)
    }
}

/// Original document text, by the path a document has in the report.
pub trait Sources {
    /// The text of `path`, when the caller has it.
    fn text(&self, path: &str) -> Option<&str>;
}

impl<S: std::hash::BuildHasher> Sources for HashMap<String, String, S> {
    fn text(&self, path: &str) -> Option<&str> {
        self.get(path).map(String::as_str)
    }
}

/// No source text. Formats that need byte or UTF-16 columns fail for findings with a span.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSources;

impl Sources for NoSources {
    fn text(&self, _: &str) -> Option<&str> {
        None
    }
}

/// Why a report could not be rendered.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// The format needs the document text and the caller gave none.
    #[error("{format} output needs the text of {path}")]
    MissingSource {
        /// Format that needs it.
        format: &'static str,
        /// Document path.
        path: String,
    },
    /// A span does not fit the document text.
    #[error("{path}: {source}")]
    Position {
        /// Document path.
        path: String,
        /// The problem.
        source: PositionError,
    },
    /// Serialisation failed.
    #[error("cannot serialise the report: {0}")]
    Json(#[from] serde_json::Error),
}

/// Options that change rendered output without changing its structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderOptions {
    /// Base URL of the published docs, ending in `/` (for example `https://host/slopvac/`).
    /// Rule pages sit at `<base>v3/rules/<category>/<rule>/`. `None` omits rule links.
    pub docs_base: Option<String>,
}

impl Default for RenderOptions {
    /// The docs address from `docs/site.toml` at build time, or none when the file is absent.
    fn default() -> Self {
        Self {
            docs_base: option_env!("SLOPVAC_DOCS_BASE").map(str::to_owned),
        }
    }
}

impl RenderOptions {
    /// The page for a rule, when a docs address is set.
    ///
    /// `local` is the part of the rule ID after the pack, which has the form
    /// `<category>.<name>` (`core:orwell.not-un` is `orwell.not-un`). The page is
    /// `<base>v3/rules/<category>/<name>/`, so the category appears once.
    #[must_use]
    pub fn rule_url(&self, category: &str, local: &str) -> Option<String> {
        let base = self.docs_base.as_deref()?.trim_end_matches('/');
        let name = local
            .strip_prefix(category)
            .and_then(|rest| rest.strip_prefix('.'))
            .unwrap_or(local);
        Some(format!("{base}/{API_VERSION}/rules/{category}/{name}/"))
    }
}

/// The docs path segment for this schema line.
pub const API_VERSION: &str = "v3";

/// Render `report` in `format` with default options. Output ends with one newline, or is empty (no findings in a line format).
pub fn render(
    format: Format,
    report: &Report,
    sources: &dyn Sources,
) -> Result<String, RenderError> {
    render_with(format, report, sources, &RenderOptions::default())
}

/// Render `report` in `format`.
pub fn render_with(
    format: Format,
    report: &Report,
    sources: &dyn Sources,
    options: &RenderOptions,
) -> Result<String, RenderError> {
    let mut out = match format {
        Format::Text => text::render(report),
        Format::Json => serde_json::to_string_pretty(report)?,
        Format::Github => github::render(report),
        Format::Sarif => sarif::render(report, sources, options)?,
        Format::Rdjsonl => rdjsonl::render(report, sources, options)?,
        Format::CodeClimate => codeclimate::render(report, sources)?,
        Format::Junit => junit::render(report),
        Format::Checkstyle => checkstyle::render(report),
        Format::Markdown => markdown::render(report),
        Format::Html => html::render(report),
    };
    let trimmed = out.trim_end_matches('\n').len();
    out.truncate(trimmed);
    if !out.is_empty() {
        out.push('\n');
    }
    Ok(out)
}

/// Drop the result of a write into a `String`. `fmt::Write for String` never returns an
/// error, so there is nothing to handle; this is the one place that says so.
#[expect(
    clippy::let_underscore_must_use,
    reason = "fmt::Write for String never returns an error"
)]
pub(crate) fn infallible(result: std::fmt::Result) {
    let _ = result;
}
