//! Code block kinds and span trimming, shared by both parsers.

use slopvac_model::Span;

use crate::BlockKind;

/// `span` without its final line terminator.
pub(crate) fn trim_terminator(source: &str, span: Span) -> Span {
    let kept = source[span.start()..span.end()]
        .trim_end_matches(['\n', '\r'])
        .len();
    Span::new(span.start(), span.start() + kept).expect("kept <= len")
}

/// The block kind of a code block. `info` is `None` for an indented block.
pub(crate) fn code_kind(info: Option<&str>, body: Span) -> BlockKind {
    let trimmed = info.map(str::trim).filter(|i| !i.is_empty());
    BlockKind::CodeBlock {
        lang: trimmed.and_then(|i| i.split_whitespace().next().map(str::to_owned)),
        info: trimmed.map(str::to_owned),
        body,
        fenced: info.is_some(),
    }
}

/// Body of an mdast code node, as byte offsets in `text`.
///
/// markdown-rs gives the node's range and its decoded `value`, not the range of each
/// line. The body runs from the first line after the opener to the closing fence, or to
/// the end of the node when it is unclosed. The first line loses its container prefix,
/// which is what is left of the line once the decoded first line of `value` is removed.
/// Whether the closing fence is there follows from how many lines `value` has.
pub(crate) fn mdast_body(text: &str, start: usize, end: usize, value: &str) -> (usize, usize) {
    let mut lines: Vec<(usize, usize)> = Vec::new(); // (start, content end)
    let mut pos = start;
    let bytes = text.as_bytes();
    while pos < end {
        let line_start = pos;
        while pos < end && bytes[pos] != b'\n' && bytes[pos] != b'\r' {
            pos += 1;
        }
        lines.push((line_start, pos));
        if bytes.get(pos) == Some(&b'\r') && bytes.get(pos + 1) == Some(&b'\n') {
            pos += 2;
        } else {
            pos += 1;
        }
    }
    let count = lines.len();
    if count < 2 {
        let at = terminator_end(text, end);
        return (at, at);
    }
    let body_lines = if value.is_empty() {
        0..=1
    } else {
        let n = line_count(value);
        n..=n
    };
    let closed_fits = body_lines.contains(&(count - 2));
    let open_fits = body_lines.contains(&(count - 1));
    let (last_start, last_end) = lines[count - 1];
    let looks_closing = {
        let t = text[last_start..last_end].trim_start_matches([' ', '\t', '>']);
        t.starts_with(['`', '~']) && t.chars().all(|c| c == t.as_bytes()[0] as char || c == ' ')
    };
    let closed = match (closed_fits, open_fits) {
        (true, false) => true,
        (false, true) => false,
        _ => looks_closing,
    };
    let body_end = if closed {
        last_start
    } else {
        terminator_end(text, end)
    };
    let (first_start, first_end) = lines[1];
    let first_value = value.split(['\r', '\n']).next().unwrap_or("");
    let line = &text[first_start..first_end];
    let prefix = if !first_value.is_empty() && line.ends_with(first_value) {
        line.len() - first_value.len()
    } else {
        0
    };
    let body_start = (first_start + prefix).min(body_end);
    (body_start, body_end)
}

/// `at` moved past the line terminator that starts there, if any.
fn terminator_end(text: &str, at: usize) -> usize {
    let rest = &text[at..];
    if rest.starts_with("\r\n") {
        at + 2
    } else if rest.starts_with(['\n', '\r']) {
        at + 1
    } else {
        at
    }
}

/// Lines in `value`, where a line ends with `\n`, `\r\n` or `\r`.
fn line_count(value: &str) -> usize {
    let bytes = value.as_bytes();
    let breaks = bytes
        .iter()
        .enumerate()
        .filter(|&(i, &b)| b == b'\n' && (i == 0 || bytes[i - 1] != b'\r') || b == b'\r')
        .count();
    breaks + 1
}
