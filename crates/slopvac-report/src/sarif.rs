//! SARIF 2.1.0 for code scanning. Columns are UTF-16 code units, declared by `columnKind`.

use std::collections::HashMap;
use std::fmt::Write as _;

use serde::Serialize;
use slopvac_model::{ColumnUnit, Finding, NoticeLevel, Report, Severity};

use crate::locate::DocIndex;
use crate::{RenderError, RenderOptions, Sources};

const SCHEMA: &str =
    "https://docs.oasis-open.org/sarif/sarif/2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json";

// The models below borrow from the report and serialize in declaration order. The output has
// always had alphabetically sorted keys, so each struct lists its fields in that order.

#[derive(Serialize)]
struct Log<'a> {
    #[serde(rename = "$schema")]
    schema: &'static str,
    runs: [Run<'a>; 1],
    version: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Run<'a> {
    column_kind: &'static str,
    invocations: [Invocation<'a>; 1],
    results: Vec<SarifResult<'a>>,
    tool: Tool<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Invocation<'a> {
    execution_successful: bool,
    tool_execution_notifications: Vec<Notification<'a>>,
}

#[derive(Serialize)]
struct Notification<'a> {
    descriptor: Descriptor<'a>,
    level: Level,
    message: Text<'a>,
}

#[derive(Serialize)]
struct Descriptor<'a> {
    id: &'a str,
}

#[derive(Serialize)]
struct Text<'a> {
    text: &'a str,
}

/// SARIF calls a suggestion, and an informational notice, `note`.
#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Level {
    Error,
    Warning,
    Note,
}

#[derive(Serialize)]
struct Tool<'a> {
    driver: Driver<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Driver<'a> {
    information_uri: &'static str,
    name: &'static str,
    rules: Vec<Rule<'a>>,
    version: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Rule<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    help_uri: Option<String>,
    id: &'a str,
    properties: RuleProperties<'a>,
    short_description: Text<'a>,
}

#[derive(Serialize)]
struct RuleProperties<'a> {
    category: &'a str,
    pack: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifResult<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    fixes: Option<[Fix<'a>; 1]>,
    level: Level,
    locations: [Location; 1],
    message: Text<'a>,
    rule_id: &'a str,
    rule_index: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Location {
    physical_location: PhysicalLocation,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PhysicalLocation {
    artifact_location: ArtifactLocation,
    region: Region,
}

#[derive(Serialize)]
struct ArtifactLocation {
    uri: String,
}

/// A region with both ends, or the start line alone when the finding has no source range.
#[derive(Serialize)]
#[serde(untagged)]
enum Region {
    Range(Range),
    Line(StartLine),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Range {
    end_column: usize,
    end_line: usize,
    start_column: usize,
    start_line: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartLine {
    start_line: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Fix<'a> {
    artifact_changes: [ArtifactChange<'a>; 1],
    description: Text<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactChange<'a> {
    artifact_location: ArtifactLocation,
    replacements: Vec<Replacement<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Replacement<'a> {
    deleted_region: Range,
    inserted_content: Text<'a>,
}

fn level(severity: Severity) -> Level {
    match severity {
        Severity::Error => Level::Error,
        Severity::Warning => Level::Warning,
        Severity::Suggestion => Level::Note,
    }
}

/// Percent-encode a path for an artifact URI, keeping `/` and unreserved characters.
fn uri(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn region(doc: &DocIndex<'_>, finding: &Finding) -> Result<Region, RenderError> {
    let at = doc.locate(finding, ColumnUnit::Utf16)?;
    Ok(match at.end {
        Some(end) => Region::Range(Range {
            end_column: end.column,
            end_line: end.line,
            start_column: at.start.column,
            start_line: at.start.line,
        }),
        // No source range: the stored column counts code points, which SARIF does not
        // declare, so the location carries the line only.
        None => Region::Line(StartLine {
            start_line: at.start.line,
        }),
    })
}

fn fix<'a>(
    doc: &DocIndex<'_>,
    path: &str,
    finding: &'a Finding,
) -> Result<Option<Fix<'a>>, RenderError> {
    let Some(fix) = &finding.fix else {
        return Ok(None);
    };
    let mut replacements = Vec::new();
    for edit in fix.edits() {
        let (s, e) = doc.locate_edit(edit.span, ColumnUnit::Utf16)?;
        replacements.push(Replacement {
            deleted_region: Range {
                end_column: e.column,
                end_line: e.line,
                start_column: s.column,
                start_line: s.line,
            },
            inserted_content: Text { text: &edit.text },
        });
    }
    Ok(Some(Fix {
        artifact_changes: [ArtifactChange {
            artifact_location: ArtifactLocation { uri: uri(path) },
            replacements,
        }],
        description: Text {
            text: finding.message.as_str(),
        },
    }))
}

pub(crate) fn render(
    report: &Report,
    sources: &dyn Sources,
    options: &RenderOptions,
) -> Result<String, RenderError> {
    let mut rules: Vec<Rule<'_>> = Vec::new();
    let mut rule_index: HashMap<&str, usize> = HashMap::new();
    let mut results = Vec::new();
    for document in &report.documents {
        let doc = DocIndex::new(document, sources, "sarif");
        for f in &document.findings {
            let next = rules.len();
            let index = *rule_index.entry(f.rule_id.as_str()).or_insert_with(|| {
                rules.push(Rule {
                    help_uri: options.rule_page(&f.rule_id, f.category.as_str()),
                    id: f.rule_id.as_str(),
                    properties: RuleProperties {
                        category: f.category.as_str(),
                        pack: f.pack.as_str(),
                    },
                    short_description: Text {
                        text: f.message.as_str(),
                    },
                });
                next
            });
            // The region comes first so that a position error in it wins over one in the fix.
            let region = region(&doc, f)?;
            let fixes = fix(&doc, document.path.as_str(), f)?.map(|fix| [fix]);
            results.push(SarifResult {
                fixes,
                level: level(f.severity),
                locations: [Location {
                    physical_location: PhysicalLocation {
                        artifact_location: ArtifactLocation {
                            uri: uri(document.path.as_str()),
                        },
                        region,
                    },
                }],
                message: Text {
                    text: f.message.as_str(),
                },
                rule_id: f.rule_id.as_str(),
                rule_index: index,
            });
        }
    }
    let unchecked = report.documents.iter().flat_map(|d| d.unchecked.iter());
    let notifications = report
        .notices
        .iter()
        .chain(unchecked)
        .map(|n| Notification {
            descriptor: Descriptor {
                id: n.code.as_str(),
            },
            level: if n.level == NoticeLevel::Warning {
                Level::Warning
            } else {
                Level::Note
            },
            message: Text {
                text: n.message.as_str(),
            },
        })
        .collect();
    // Only a warning notice, at the root or in a document, means the run did not finish
    // cleanly. An info notice reports a rule that does not apply and leaves it complete.
    let complete = !report
        .notices
        .iter()
        .chain(report.documents.iter().flat_map(|d| &d.unchecked))
        .any(|n| n.level == NoticeLevel::Warning);
    let sarif = Log {
        schema: SCHEMA,
        runs: [Run {
            column_kind: "utf16CodeUnits",
            invocations: [Invocation {
                execution_successful: complete,
                tool_execution_notifications: notifications,
            }],
            results,
            tool: Tool {
                driver: Driver {
                    information_uri: "https://github.com/slopvac/slopvac",
                    name: "slopvac",
                    rules,
                    version: report.slopvac_version.as_str(),
                },
            },
        }],
        version: "2.1.0",
    };
    Ok(serde_json::to_string_pretty(&sarif)?)
}

#[cfg(test)]
mod tests {
    use super::uri;

    #[test]
    fn uris_are_percent_encoded() {
        assert_eq!(uri("docs/a b#c.md"), "docs/a%20b%23c.md");
        assert_eq!(uri("<stdin>"), "%3Cstdin%3E");
        assert_eq!(uri("é"), "%C3%A9");
    }
}
