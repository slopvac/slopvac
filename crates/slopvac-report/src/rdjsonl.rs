//! reviewdog diagnostic format. Columns are UTF-8 bytes (reviewdog.proto).

use serde::Serialize;
use slopvac_model::{ColumnUnit, Report, Severity};

use crate::locate::{DocIndex, Pos};
use crate::{RenderError, RenderOptions, Sources};

// The models below borrow from the report and serialize in declaration order. The output has
// always had alphabetically sorted keys, so each struct lists its fields in that order.

#[derive(Serialize)]
struct Diagnostic<'a> {
    code: Code<'a>,
    location: Location<'a>,
    message: &'a str,
    severity: Level,
    source: Source,
    #[serde(skip_serializing_if = "Option::is_none")]
    suggestions: Option<Vec<Suggestion<'a>>>,
}

#[derive(Serialize)]
struct Code<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    value: &'a str,
}

#[derive(Serialize)]
struct Location<'a> {
    path: &'a str,
    range: Range,
}

/// The range of a finding: both ends, or the start line alone when it has no source range.
#[derive(Serialize)]
#[serde(untagged)]
enum Range {
    Span(Span),
    Line { start: Line },
}

#[derive(Serialize)]
struct Span {
    end: Position,
    start: Position,
}

#[derive(Serialize)]
struct Position {
    column: usize,
    line: usize,
}

#[derive(Serialize)]
struct Line {
    line: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "UPPERCASE")]
enum Level {
    Error,
    Warning,
    Info,
}

#[derive(Serialize)]
struct Source {
    name: &'static str,
}

#[derive(Serialize)]
struct Suggestion<'a> {
    range: Span,
    text: &'a str,
}

impl From<Pos> for Position {
    fn from(p: Pos) -> Self {
        Self {
            column: p.column,
            line: p.line,
        }
    }
}

pub(crate) fn render(
    report: &Report,
    sources: &dyn Sources,
    options: &RenderOptions,
) -> Result<String, RenderError> {
    let mut out = String::new();
    for document in &report.documents {
        let doc = DocIndex::new(document, sources, "rdjsonl");
        for f in &document.findings {
            let at = doc.locate(f, ColumnUnit::Utf8)?;
            // Without a source range the stored column counts code points, not bytes, so a
            // document-level finding carries the line only. The proto makes the column optional.
            let range = match at.end {
                Some(end) => Range::Span(Span {
                    end: end.into(),
                    start: at.start.into(),
                }),
                None => Range::Line {
                    start: Line {
                        line: at.start.line,
                    },
                },
            };
            let severity = match f.severity {
                Severity::Error => Level::Error,
                Severity::Warning => Level::Warning,
                Severity::Suggestion => Level::Info,
            };
            let suggestions = match &f.fix {
                Some(fix) => {
                    let mut suggestions = Vec::new();
                    for edit in fix.edits() {
                        let (s, e) = doc.locate_edit(edit.span, ColumnUnit::Utf8)?;
                        suggestions.push(Suggestion {
                            range: Span {
                                end: e.into(),
                                start: s.into(),
                            },
                            text: &edit.text,
                        });
                    }
                    Some(suggestions)
                }
                None => None,
            };
            let diagnostic = Diagnostic {
                code: Code {
                    url: options.rule_page(&f.rule_id, f.category.as_str()),
                    value: f.rule_id.as_str(),
                },
                location: Location {
                    path: document.path.as_str(),
                    range,
                },
                message: f.message.as_str(),
                severity,
                source: Source { name: "slopvac" },
                suggestions,
            };
            out.push_str(&serde_json::to_string(&diagnostic)?);
            out.push('\n');
        }
    }
    Ok(out)
}
