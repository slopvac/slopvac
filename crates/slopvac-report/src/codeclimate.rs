//! Code Climate issues. Only `location.lines` is emitted (docs/contract/positions.md): GitLab
//! Code Quality requires it, and the spec treats lines and positions as alternatives. The end
//! line is inclusive.

use serde_json::json;
use slopvac_model::{ColumnUnit, Report, Severity};

use crate::locate::DocIndex;
use crate::{RenderError, Sources};

/// The spec wants a description on one line.
fn single_line(text: &str) -> String {
    text.split(['\r', '\n'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// FNV-1a, 64 bit. Stable across releases, unlike the standard hasher.
fn fingerprint(parts: &[&str]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for b in part.bytes().chain(std::iter::once(0)) {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

pub(crate) fn render(report: &Report, sources: &dyn Sources) -> Result<String, RenderError> {
    let mut issues = Vec::new();
    for document in &report.documents {
        let doc = DocIndex::new(document, sources, "codeclimate");
        for f in &document.findings {
            let at = doc.locate(f, ColumnUnit::CodePoint)?;
            let lines_end = match f.extent.map(|e| e.span) {
                Some(span) if !span.is_empty() => doc.last_included(span)?.line,
                _ => at.start.line,
            };
            let severity = match f.severity {
                Severity::Error => "major",
                Severity::Warning => "minor",
                Severity::Suggestion => "info",
            };
            let key = format!("{}:{}", at.start.line, at.start.column);
            issues.push(json!({
                "type": "issue",
                "check_name": f.rule_id.as_str(),
                "description": single_line(f.message.as_str()),
                "categories": ["Style"],
                "severity": severity,
                "fingerprint": fingerprint(&[document.path.as_str(), f.rule_id.as_str(), &key, f.message.as_str()]),
                "location": {
                    "path": document.path,
                    "lines": {"begin": at.start.line, "end": lines_end},
                },
            }));
        }
    }
    Ok(serde_json::to_string_pretty(&issues)?)
}

#[cfg(test)]
mod tests {
    use super::fingerprint;

    #[test]
    fn fingerprint_is_fnv1a_and_separates_parts() {
        assert_eq!(fingerprint(&[]), "cbf29ce484222325");
        assert_ne!(fingerprint(&["ab", "c"]), fingerprint(&["a", "bc"]));
    }
}
