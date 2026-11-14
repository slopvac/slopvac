//! Code block bodies and span trimming, shared by both parsers.

use slopvac_model::Span;

use crate::BlockKind;

/// `span` without its final line terminator.
pub(crate) fn trim_terminator(source: &str, span: Span) -> Span {
    let kept = source[span.start()..span.end()]
        .trim_end_matches(['\n', '\r'])
        .len();
    Span::new(span.start(), span.start() + kept).expect("kept <= len")
}

/// The block kind for a code block that covers `span` (terminator trimmed).
pub(crate) fn code_block(source: &str, span: Span, fenced: bool) -> BlockKind {
    if !fenced {
        return BlockKind::CodeBlock {
            lang: None,
            info: None,
            body: span,
            fenced,
        };
    }
    let text = &source[span.start()..span.end()];
    let opener = text.find(['\n', '\r']).map_or(text, |i| &text[..i]);
    let opener = strip_container(opener);
    let ch = opener.bytes().next().unwrap_or(b'`');
    let count = opener.bytes().take_while(|&b| b == ch).count();
    let info = opener[count..].trim();
    let body = fenced_body(text, span.start(), ch, count);
    BlockKind::CodeBlock {
        lang: info.split_whitespace().next().map(str::to_owned),
        info: (!info.is_empty()).then(|| info.to_owned()),
        body,
        fenced,
    }
}

/// Body of a fenced block: the lines between the opener and the closer.
fn fenced_body(text: &str, base: usize, ch: u8, count: usize) -> Span {
    let Some(first) = text.find(['\n', '\r']) else {
        return Span::empty(base + text.len());
    };
    let first_end = if text[first..].starts_with("\r\n") {
        first + 2
    } else {
        first + 1
    };
    let last_start = text.rfind(['\n', '\r']).map_or(0, |i| i + 1);
    let last = strip_container(&text[last_start..]).trim_end();
    let run = last.bytes().take_while(|&b| b == ch).count();
    let closed = last_start >= first_end && run >= count && last[run..].trim().is_empty();
    let end = if closed { last_start } else { text.len() };
    Span::new(base + first_end, base + end.max(first_end)).expect("end is clamped to start")
}

/// A line without the block quote markers and indentation that precede its content.
fn strip_container(line: &str) -> &str {
    line.trim_start_matches([' ', '\t', '>'])
}
