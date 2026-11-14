//! reviewdog diagnostic format. Columns are UTF-8 bytes (reviewdog.proto).

use serde_json::{Value, json};
use slopvac_model::{ColumnUnit, Report, Severity};

use crate::locate::{DocIndex, Pos};
use crate::{RenderError, RenderOptions, Sources};

fn pos(p: Pos) -> Value {
    json!({"line": p.line, "column": p.column})
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
                Some(end) => json!({"start": pos(at.start), "end": pos(end)}),
                None => json!({"start": {"line": at.start.line}}),
            };
            let severity = match f.severity {
                Severity::Error => "ERROR",
                Severity::Warning => "WARNING",
                Severity::Suggestion => "INFO",
            };
            let mut diagnostic = json!({
                "message": f.message,
                "location": {"path": document.path, "range": range},
                "severity": severity,
                "source": {"name": "slopvac"},
                "code": {"value": f.rule_id.as_str()},
            });
            if let Some(url) = options.rule_url(&f.category, f.rule_id.local()) {
                diagnostic["code"]["url"] = json!(url);
            }
            if let Some(fix) = &f.fix {
                let mut suggestions = Vec::new();
                for edit in &fix.edits {
                    let (s, e) = doc.locate_edit(edit.span, ColumnUnit::Utf8)?;
                    suggestions.push(json!({
                        "range": {"start": pos(s), "end": pos(e)},
                        "text": edit.text,
                    }));
                }
                diagnostic["suggestions"] = Value::Array(suggestions);
            }
            out.push_str(&serde_json::to_string(&diagnostic)?);
            out.push('\n');
        }
    }
    Ok(out)
}
