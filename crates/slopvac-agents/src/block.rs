//! Managed steering blocks.

use std::ops::Range;
use std::path::Path;

use pulldown_cmark::{Event, Parser, Tag};

use crate::error::AgentsError;
use crate::fsx::Project;

/// Opening marker of the managed block.
pub const BEGIN: &str = "<!-- slopvac:begin -->";
/// Closing marker of the managed block.
pub const END: &str = "<!-- slopvac:end -->";

/// State of a managed block or file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Not installed.
    Missing,
    /// Installed and equal to what setup would write.
    Current,
    /// Installed but different.
    Stale,
    /// Present but not safe for setup to touch.
    Malformed,
    /// Not wanted and not installed.
    Disabled,
}

impl State {
    /// The word printed by `setup --check`.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Current => "current",
            Self::Stale => "stale",
            Self::Malformed => "malformed",
            Self::Disabled => "disabled",
        }
    }
}

fn read_utf8(project: &Project, rel: &Path) -> Result<Option<(Vec<u8>, String)>, AgentsError> {
    let Some(bytes) = project.read(rel)? else {
        return Ok(None);
    };
    let text = String::from_utf8(bytes.clone())
        .map_err(|_| AgentsError::NotUtf8(project.display(rel).into()))?;
    Ok(Some((bytes, text)))
}

/// Byte ranges of every line, terminator included. A terminator is `\n`, `\r\n`
/// or a lone `\r`.
fn line_spans(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                spans.push(start..i + 1);
                start = i + 1;
            }
            b'\r' => {
                let end = if bytes.get(i + 1) == Some(&b'\n') {
                    i + 2
                } else {
                    i + 1
                };
                spans.push(start..end);
                start = end;
                i = end - 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < bytes.len() {
        spans.push(start..bytes.len());
    }
    spans
}

/// Byte range of the managed section, ignoring markers inside code blocks.
/// Runs in one pass: code block ranges and line starts both ascend.
fn section(text: &str, shown: &str) -> Result<Option<(usize, usize)>, AgentsError> {
    let fenced: Vec<Range<usize>> = Parser::new(text)
        .into_offset_iter()
        .filter_map(|(event, range)| {
            matches!(event, Event::Start(Tag::CodeBlock(_))).then_some(range)
        })
        .collect();
    let mut cursor = 0;
    let mut found: Vec<(&str, usize, usize)> = Vec::new();
    for span in line_spans(text) {
        while cursor < fenced.len() && fenced[cursor].end <= span.start {
            cursor += 1;
        }
        let inside = cursor < fenced.len() && fenced[cursor].start <= span.start;
        let line = &text[span.clone()];
        let value = line.trim().trim_start_matches('\u{feff}');
        if !inside && (value == BEGIN || value == END) {
            let bom = if line.starts_with('\u{feff}') {
                '\u{feff}'.len_utf8()
            } else {
                0
            };
            found.push((value, span.start + bom, span.end));
        }
    }
    match found.as_slice() {
        [] => Ok(None),
        [(BEGIN, start, _), (END, _, end)] => Ok(Some((*start, *end))),
        _ => Err(AgentsError::MalformedMarkers(shown.into())),
    }
}

fn newline(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else if text.contains('\n') {
        "\n"
    } else if text.contains('\r') {
        "\r"
    } else {
        "\n"
    }
}

fn native(value: &str, nl: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', nl)
}

/// State of the managed block in `rel` against `block`.
#[must_use]
pub fn block_state(project: &Project, rel: &Path, block: &str) -> State {
    let text = match read_utf8(project, rel) {
        Ok(Some((_, text))) => text,
        Ok(None) => return State::Missing,
        Err(_) => return State::Malformed,
    };
    match section(&text, &project.display(rel)) {
        Err(_) => State::Malformed,
        Ok(None) => State::Missing,
        Ok(Some((s, e))) => {
            if native(text[s..e].trim(), "\n") == block.trim() {
                State::Current
            } else {
                State::Stale
            }
        }
    }
}

/// Length of the line terminator that ends `s`, or 0.
fn trailing_term(s: &str) -> usize {
    if s.ends_with("\r\n") {
        2
    } else {
        usize::from(s.ends_with(['\n', '\r']))
    }
}

/// Length of the line terminator that starts `s`, or 0.
fn leading_term(s: &str) -> usize {
    if s.starts_with("\r\n") {
        2
    } else {
        usize::from(s.starts_with(['\n', '\r']))
    }
}

/// Whether `s` ends with an empty line: two terminators in a row.
fn ends_with_blank_line(s: &str) -> bool {
    let first = trailing_term(s);
    first > 0 && trailing_term(&s[..s.len() - first]) > 0
}

/// Insert or replace the managed block.
///
/// Every byte outside the block survives. A new block goes at the end of the file
/// after exactly one inserted line terminator, so `remove_block` can take that one
/// terminator back out and restore the original bytes.
pub fn update_block(
    project: &Project,
    rel: &Path,
    block: &str,
    preamble: &str,
) -> Result<bool, AgentsError> {
    let existing = read_utf8(project, rel)?;
    let (before, text) = match &existing {
        Some((b, t)) => (Some(b.as_slice()), t.as_str()),
        None => (None, ""),
    };
    let nl = newline(text);
    let body = format!("{}{nl}", native(block.trim_end_matches(['\r', '\n']), nl));
    let updated = if let Some((s, e)) = section(text, &project.display(rel))? {
        format!("{}{body}{}", &text[..s], &text[e..])
    } else if text.is_empty() && !preamble.is_empty() {
        let head = native(preamble.trim_end_matches(['\r', '\n']), nl);
        format!("{head}{nl}{nl}{body}")
    } else if text.is_empty() {
        body
    } else {
        format!("{text}{nl}{body}")
    };
    project.write_atomic(rel, before, updated.as_bytes())
}

/// Remove only the managed block.
///
/// A block at the end of the file also takes back the one line terminator that
/// `update_block` inserted before it. A block in the middle leaves the text around
/// it as it was, except that two blank lines in a row collapse to one.
pub fn remove_block(project: &Project, rel: &Path) -> Result<bool, AgentsError> {
    let Some((bytes, text)) = read_utf8(project, rel)? else {
        return Ok(false);
    };
    let Some((s, e)) = section(&text, &project.display(rel))? else {
        return Ok(false);
    };
    let (left, right) = (&text[..s], &text[e..]);
    let updated = if right.is_empty() {
        left[..left.len() - trailing_term(left)].to_owned()
    } else if left.is_empty() || ends_with_blank_line(left) {
        format!("{left}{}", &right[leading_term(right)..])
    } else {
        format!("{left}{right}")
    };
    project.write_atomic(rel, Some(&bytes), updated.as_bytes())
}
