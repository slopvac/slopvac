//! Export of catalog entries as a Fluent (`.ftl`) file.
//!
//! This crate has no Fluent dependency. The output uses the plain subset of the format:
//! `id = text { $param }`, with comments above each message.

use std::fmt::Write as _;

use crate::entry::{Entry, ParamKind};
use crate::template::{Segment, segments};

/// Write `entries` as Fluent source, one message per entry in the given order.
///
/// Each message id is the entry code and each placeholder becomes `{ $name }`. A literal
/// brace becomes a quoted string placeholder, and a continuation line that would start a
/// Fluent attribute or variant is quoted the same way.
#[must_use]
pub fn to_ftl(entries: &[Entry]) -> String {
    let mut out = String::new();
    for entry in entries {
        let _ = writeln!(out, "# {}", entry.summary);
        if !entry.params.is_empty() {
            out.push_str("#\n# Variables:\n");
            for param in entry.params {
                let kind = match param.kind {
                    ParamKind::Text => "text",
                    ParamKind::Count => "count, number",
                    ParamKind::Path => "path",
                };
                let _ = writeln!(out, "#   ${} ({kind})", param.name);
            }
        }
        let _ = writeln!(out, "{} = {}\n", entry.code, pattern(entry.template));
    }
    out
}

fn pattern(template: &str) -> String {
    let mut out = String::new();
    // True from a line break until the first non-space character of the next line.
    let mut line_start = false;
    for segment in segments(template) {
        let text = match segment {
            Segment::Placeholder(name) => {
                begin_content(&mut out, &mut line_start);
                let _ = write!(out, "{{ ${name} }}");
                continue;
            }
            Segment::Literal(text) => text,
        };
        for c in text.chars() {
            match c {
                '\n' => {
                    out.push('\n');
                    line_start = true;
                }
                '{' | '}' => {
                    begin_content(&mut out, &mut line_start);
                    let _ = write!(out, "{{ \"{c}\" }}");
                }
                '[' | '*' | '.' if line_start => {
                    begin_content(&mut out, &mut line_start);
                    let _ = write!(out, "{{ \"{c}\" }}");
                }
                ' ' if line_start => {
                    indent(&mut out);
                    out.push(' ');
                }
                _ => {
                    begin_content(&mut out, &mut line_start);
                    out.push(c);
                }
            }
        }
    }
    out
}

/// Indent a continuation line before its first content, once.
fn begin_content(out: &mut String, line_start: &mut bool) {
    indent(out);
    *line_start = false;
}

fn indent(out: &mut String) {
    if out.ends_with('\n') {
        out.push_str("    ");
    }
}
