//! SARIF 2.1.0 for code scanning. Columns are UTF-16 code units, declared by `columnKind`.

use std::fmt::Write as _;

use serde_json::{Value, json};
use slopvac_model::{ColumnUnit, Finding, NoticeLevel, Report, Severity};

use crate::locate::DocIndex;
use crate::{RenderError, RenderOptions, Sources};

const SCHEMA: &str =
    "https://docs.oasis-open.org/sarif/sarif/2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json";

fn level(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Suggestion => "note",
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

fn region(doc: &DocIndex<'_>, finding: &Finding) -> Result<Value, RenderError> {
    let at = doc.locate(finding, ColumnUnit::Utf16)?;
    Ok(match at.end {
        Some(end) => json!({
            "startLine": at.start.line,
            "startColumn": at.start.column,
            "endLine": end.line,
            "endColumn": end.column,
        }),
        // No source range: the stored column counts code points, which SARIF does not
        // declare, so the location carries the line only.
        None => json!({"startLine": at.start.line}),
    })
}

fn fix(doc: &DocIndex<'_>, path: &str, finding: &Finding) -> Result<Option<Value>, RenderError> {
    let Some(fix) = &finding.fix else {
        return Ok(None);
    };
    let mut replacements = Vec::new();
    for edit in fix.edits() {
        let (s, e) = doc.locate_edit(edit.span, ColumnUnit::Utf16)?;
        replacements.push(json!({
            "deletedRegion": {
                "startLine": s.line, "startColumn": s.column,
                "endLine": e.line, "endColumn": e.column,
            },
            "insertedContent": {"text": edit.text},
        }));
    }
    Ok(Some(json!({
        "description": {"text": finding.message},
        "artifactChanges": [{
            "artifactLocation": {"uri": uri(path)},
            "replacements": replacements,
        }],
    })))
}

pub(crate) fn render(
    report: &Report,
    sources: &dyn Sources,
    options: &RenderOptions,
) -> Result<String, RenderError> {
    let mut rules: Vec<Value> = Vec::new();
    let mut rule_index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut results = Vec::new();
    for document in &report.documents {
        let doc = DocIndex::new(document, sources, "sarif");
        for f in &document.findings {
            let id = f.rule_id.as_str().to_owned();
            let next = rules.len();
            let index = *rule_index.entry(id.clone()).or_insert_with(|| {
                let mut rule = json!({
                    "id": id,
                    "shortDescription": {"text": f.message},
                    "properties": {"category": f.category, "pack": f.pack.as_str()},
                });
                if let Some(url) = options.rule_page(&f.rule_id, f.category.as_str()) {
                    rule["helpUri"] = json!(url);
                }
                rules.push(rule);
                next
            });
            let mut result = json!({
                "ruleId": f.rule_id.as_str(),
                "ruleIndex": index,
                "level": level(f.severity),
                "message": {"text": f.message},
                "locations": [{"physicalLocation": {
                    "artifactLocation": {"uri": uri(document.path.as_str())},
                    "region": region(&doc, f)?,
                }}],
            });
            if let Some(fix) = fix(&doc, document.path.as_str(), f)? {
                result["fixes"] = json!([fix]);
            }
            results.push(result);
        }
    }
    let mut notifications = Vec::new();
    let unchecked = report.documents.iter().flat_map(|d| d.unchecked.iter());
    for n in report.notices.iter().chain(unchecked) {
        notifications.push(json!({
            "descriptor": {"id": n.code},
            "level": if n.level == NoticeLevel::Warning { "warning" } else { "note" },
            "message": {"text": n.message},
        }));
    }
    // Only a warning notice, at the root or in a document, means the run did not finish
    // cleanly. An info notice reports a rule that does not apply and leaves it complete.
    let complete = !report
        .notices
        .iter()
        .chain(report.documents.iter().flat_map(|d| &d.unchecked))
        .any(|n| n.level == NoticeLevel::Warning);
    let sarif = json!({
        "$schema": SCHEMA,
        "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {
                "name": "slopvac",
                "version": report.slopvac_version,
                "informationUri": "https://github.com/slopvac/slopvac",
                "rules": rules,
            }},
            "columnKind": "utf16CodeUnits",
            "invocations": [{
                "executionSuccessful": complete,
                "toolExecutionNotifications": notifications,
            }],
            "results": results,
        }],
    });
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
