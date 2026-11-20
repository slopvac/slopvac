//! The message catalogs of the report crate: render errors and the labels, titles and
//! headings of the human-readable formats.
//!
//! The machine-readable writers (SARIF, `JUnit`, Checkstyle, Code Climate, rdjsonl, GitHub
//! annotations) keep their field names and schema-fixed strings as literals.

use std::sync::OnceLock;

use slopvac_messages::{Catalog, Entry};

/// Why a report could not be rendered.
#[derive(Debug, Catalog)]
pub enum RenderError {
    /// The format needs the document text and the caller gave none.
    #[code("render-missing-source")]
    #[message("{format} output needs the text of {path}")]
    MissingSource {
        /// Format that needs it.
        format: &'static str,
        /// Document path.
        path: String,
    },
    /// A span does not fit the document text.
    #[code("render-position")]
    #[message("{path}: {reason}")]
    Position {
        /// Document path.
        path: String,
        /// The problem, as text.
        reason: String,
        /// The underlying position error.
        #[source]
        source: slopvac_model::PositionError,
    },
    /// Serialisation failed.
    #[code("render-json")]
    #[message("cannot serialise the report: {reason}")]
    Json {
        /// The serialiser's message.
        reason: String,
        /// The underlying serialiser error.
        #[source]
        source: serde_json::Error,
    },
}

impl From<serde_json::Error> for RenderError {
    fn from(source: serde_json::Error) -> Self {
        Self::Json {
            reason: source.to_string(),
            source,
        }
    }
}

/// Fixed words used in several formats.
#[derive(Debug, Catalog)]
pub(crate) enum Label {
    /// The verdict of a run or document that passed.
    #[code("report-verdict-pass")]
    #[text("PASS")]
    Pass,
    /// The verdict of a run or document that failed.
    #[code("report-verdict-fail")]
    #[text("FAIL")]
    Fail,
    /// The word for error severity.
    #[code("report-severity-error")]
    #[text("error")]
    SeverityError,
    /// The word for warning severity.
    #[code("report-severity-warning")]
    #[text("warning")]
    SeverityWarning,
    /// The word for suggestion severity.
    #[code("report-severity-suggestion")]
    #[text("suggestion")]
    SeveritySuggestion,
    /// Column title for a finding's position.
    #[code("report-column-location")]
    #[text("Location")]
    ColumnLocation,
    /// Column title for a finding's severity.
    #[code("report-column-severity")]
    #[text("Severity")]
    ColumnSeverity,
    /// Column title for a finding's rule.
    #[code("report-column-rule")]
    #[text("Rule")]
    ColumnRule,
    /// Column title for a finding's message.
    #[code("report-column-message")]
    #[text("Message")]
    ColumnMessage,
    /// Column title for a document.
    #[code("report-column-document")]
    #[text("Document")]
    ColumnDocument,
    /// Column title for a document's verdict.
    #[code("report-column-verdict")]
    #[text("Verdict")]
    ColumnVerdict,
    /// Column title for a document's score.
    #[code("report-column-score")]
    #[text("Score")]
    ColumnScore,
    /// Column title for a document's finding count.
    #[code("report-column-findings")]
    #[text("Findings")]
    ColumnFindings,
    /// The Markdown report title.
    #[code("report-markdown-title")]
    #[text("# slopvac report")]
    MarkdownTitle,
    /// The Markdown heading above the notice list.
    #[code("report-markdown-notices-heading")]
    #[text("## Notices")]
    MarkdownNoticesHeading,
    /// The Markdown table header and separator row for findings.
    #[code("report-markdown-table-header")]
    #[text("| Location | Severity | Rule | Message |\n| --- | --- | --- | --- |")]
    MarkdownTableHeader,
    /// The HTML report title and heading.
    #[code("report-html-title")]
    #[text("slopvac report")]
    HtmlTitle,
    /// The HTML link that skips to the findings.
    #[code("report-html-skip")]
    #[text("Skip to findings")]
    HtmlSkip,
    /// The HTML heading above the notice list.
    #[code("report-html-notices-heading")]
    #[text("Notices")]
    HtmlNoticesHeading,
    /// The HTML heading above the document table.
    #[code("report-html-documents-heading")]
    #[text("Documents")]
    HtmlDocumentsHeading,
    /// The HTML caption of the document table.
    #[code("report-html-documents-caption")]
    #[text("Result for each document")]
    HtmlDocumentsCaption,
    /// The HTML text for a document with no findings.
    #[code("report-html-no-findings")]
    #[text("No findings.")]
    HtmlNoFindings,
}

/// Lines and sentences of the human-readable formats.
#[derive(Debug, Catalog)]
pub(crate) enum Line {
    /// A notice in the text format.
    #[code("report-text-notice")]
    #[text("notice[{code}]: {message}")]
    TextNotice { code: String, message: String },
    /// A finding in the text format.
    #[code("report-text-finding")]
    #[text("{path}:{line}:{column} {severity} {rule} {message}")]
    TextFinding {
        #[display_path]
        path: String,
        #[count]
        line: usize,
        #[count]
        column: usize,
        severity: String,
        rule: String,
        message: String,
    },
    /// A check that did not run, in the text format.
    #[code("report-text-unchecked")]
    #[text("{path}: UNCHECKED [{code}] {message}")]
    TextUnchecked {
        #[display_path]
        path: String,
        code: String,
        message: String,
    },
    /// A document's verdict and score in the text format.
    #[code("report-text-document")]
    #[text("{path}: {verdict} score {score}")]
    TextDocument {
        #[display_path]
        path: String,
        verdict: String,
        score: String,
    },
    /// A reason the gate failed, in the text format.
    #[code("report-text-reason")]
    #[text("{path}: {reason}")]
    TextReason {
        #[display_path]
        path: String,
        reason: String,
    },
    /// The totals line of the text format.
    #[code("report-text-summary")]
    #[text(
        "{documents} documents, {findings} findings ({errors} errors, {warnings} warnings, {suggestions} suggestions), score {score}: {verdict}"
    )]
    TextSummary {
        #[count]
        documents: usize,
        #[count]
        findings: usize,
        #[count]
        errors: usize,
        #[count]
        warnings: usize,
        #[count]
        suggestions: usize,
        score: String,
        verdict: String,
    },
    /// The totals sentence of the Markdown report.
    #[code("report-markdown-summary")]
    #[text(
        "**{verdict}**: {documents} documents, {findings} findings ({errors} errors, {warnings} warnings, {suggestions} suggestions), score {score}."
    )]
    MarkdownSummary {
        verdict: String,
        #[count]
        documents: usize,
        #[count]
        findings: usize,
        #[count]
        errors: usize,
        #[count]
        warnings: usize,
        #[count]
        suggestions: usize,
        score: String,
    },
    /// A notice in the Markdown report.
    #[code("report-markdown-notice")]
    #[text("- `{code}`: {message}")]
    MarkdownNotice { code: String, message: String },
    /// A document heading in the Markdown report.
    #[code("report-markdown-document-heading")]
    #[text("## {path} {verdict}")]
    MarkdownDocumentHeading {
        #[display_path]
        path: String,
        verdict: String,
    },
    /// A document's statistics in the Markdown report.
    #[code("report-markdown-document-stats")]
    #[text("Score {score}, {words} words, {findings} findings.")]
    MarkdownDocumentStats {
        score: String,
        #[count]
        words: usize,
        #[count]
        findings: usize,
    },
    /// A reason the gate failed, in the Markdown report.
    #[code("report-markdown-reason")]
    #[text("- {reason}")]
    MarkdownReason { reason: String },
    /// A check that did not run, in the Markdown report.
    #[code("report-markdown-unchecked")]
    #[text("- UNCHECKED `{code}`: {message}")]
    MarkdownUnchecked { code: String, message: String },
    /// A finding row of the Markdown table.
    #[code("report-markdown-row")]
    #[text("| {line}:{column} | {severity} | {rule} | {message} |")]
    MarkdownRow {
        #[count]
        line: usize,
        #[count]
        column: usize,
        severity: String,
        rule: String,
        message: String,
    },
    /// The HTML page title.
    #[code("report-html-page-title")]
    #[text("slopvac report: {verdict}")]
    HtmlPageTitle { verdict: String },
    /// The totals paragraph of the HTML report, after the verdict.
    #[code("report-html-summary")]
    #[text(
        "{documents} documents, {findings} findings ({errors} errors, {warnings} warnings, {suggestions} suggestions), score {score}. slopvac {version}."
    )]
    HtmlSummary {
        #[count]
        documents: usize,
        #[count]
        findings: usize,
        #[count]
        errors: usize,
        #[count]
        warnings: usize,
        #[count]
        suggestions: usize,
        score: String,
        version: String,
    },
    /// A check that did not run, in the HTML report, after the `UNCHECKED` marker.
    #[code("report-html-unchecked")]
    #[text("UNCHECKED <code>{code}</code>: {message}")]
    HtmlUnchecked { code: String, message: String },
    /// The caption of one document's findings table.
    #[code("report-html-findings-caption")]
    #[text("Findings in {path}")]
    HtmlFindingsCaption {
        #[display_path]
        path: String,
    },
}

/// Every entry of the report crate's catalogs, for the workspace registry.
#[must_use]
pub fn entries() -> &'static [Entry] {
    static ENTRIES: OnceLock<Vec<Entry>> = OnceLock::new();
    ENTRIES.get_or_init(|| [RenderError::entries(), Label::entries(), Line::entries()].concat())
}
