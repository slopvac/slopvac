//! Masking of admonition syntax.
//!
//! Neither parser knows `:::`, `!!!`, `MyST` directives or GFM alerts. A line scan finds them
//! and returns a copy of the body in which the marker lines are blank and `!!!` bodies
//! lose their indentation. Both parsers then see plain Markdown, and [`Masked`] maps their
//! offsets back to the original source.

use std::ops::Range;

use slopvac_model::Span;

use crate::{Admonition, AdmonitionSyntax, Block, BlockKind};

/// `MyST` directive names that are admonitions when written as a fenced directive.
const DIRECTIVES: &[&str] = &[
    "note",
    "warning",
    "tip",
    "important",
    "caution",
    "danger",
    "attention",
    "hint",
    "error",
    "seealso",
    "admonition",
    "versionadded",
    "versionchanged",
    "deprecated",
];

/// The masked body and the admonitions found in it.
#[derive(Debug)]
pub(crate) struct Masked {
    /// Text to parse. It has the same length as the body, less the removed indentation.
    pub(crate) text: String,
    /// `(masked position, removed bytes up to and including this removal)`.
    removals: Vec<(usize, usize)>,
    base: usize,
    /// Admonition blocks, in source order.
    pub(crate) admonitions: Vec<Block>,
}

impl Masked {
    /// Source offset of a masked offset that starts a range.
    pub(crate) fn src_start(&self, offset: usize) -> usize {
        let n = self.removals.partition_point(|&(pos, _)| pos <= offset);
        self.base + offset + n.checked_sub(1).map_or(0, |i| self.removals[i].1)
    }

    /// Source offset of a masked offset that ends a range.
    pub(crate) fn src_end(&self, offset: usize) -> usize {
        let n = self.removals.partition_point(|&(pos, _)| pos < offset);
        self.base + offset + n.checked_sub(1).map_or(0, |i| self.removals[i].1)
    }

    /// Source span of a masked range.
    pub(crate) fn span(&self, start: usize, end: usize) -> Span {
        let s = self.src_start(start);
        Span::new(s, self.src_end(end).max(s)).expect("end is clamped to start")
    }
}

struct Pending {
    admonition: Admonition,
    start: usize,
    end: usize,
}

struct IndentFrame {
    body_cols: usize,
    block: usize,
}

/// A code block the parser found in the unmasked body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodeRegion {
    /// Byte range of the whole block, in body offsets.
    pub(crate) range: Range<usize>,
    /// Whether the block is fenced.
    pub(crate) fenced: bool,
}

/// A fenced directive that is open.
struct Directive {
    end: usize,
    ch: u8,
    count: usize,
    block: usize,
}

/// Scan `text`, which starts at source offset `base`.
///
/// `code` holds the code blocks the parser found in `text` before any masking. Lines inside
/// them are code and carry no markers, except that a fenced block whose info string is an
/// admonition directive becomes an admonition. No fence rules are repeated here.
///
/// With `indented_code`, as in Markdown, a marker indented four columns is not a marker.
#[allow(clippy::too_many_lines)]
pub(crate) fn scan(text: &str, base: usize, code: &[CodeRegion], indented_code: bool) -> Masked {
    let mut out = String::with_capacity(text.len());
    let mut removals: Vec<(usize, usize)> = Vec::new();
    let mut removed = 0;
    let mut pending: Vec<Pending> = Vec::new();
    let mut colons: Vec<(usize, usize)> = Vec::new();
    let mut directive: Option<Directive> = None;
    let mut indents: Vec<IndentFrame> = Vec::new();
    let mut alert: Option<usize> = None;
    let mut last_nonblank_end = base;
    let mut prev_quoted = false;
    let mut next_region = 0;

    for (line_start, line, terminator) in lines(text) {
        let abs = base + line_start;
        let line_full_end = line_start + line.len() + terminator.len();
        let blank = line.trim().is_empty();
        if !blank {
            let cols = leading_cols(line);
            while indents.last().is_some_and(|f| f.body_cols > cols) {
                if let Some(frame) = indents.pop() {
                    pending[frame.block].end = last_nonblank_end;
                }
            }
        }
        let strip = if blank {
            0
        } else {
            strip_cols(line, 4 * indents.len())
        };
        let content = &line[strip..];
        let content_abs = abs + strip;
        let line_end = abs + line.len();
        if !blank {
            last_nonblank_end = line_end;
        }
        let quote = quote_body(content);
        if let Some(index) = alert {
            if quote.is_some() {
                pending[index].end = line_end;
            } else {
                alert = None;
            }
        }
        while code
            .get(next_region)
            .is_some_and(|r| r.range.end <= line_start)
        {
            next_region += 1;
        }
        // Inside an admonition body the unmasked parse is not reliable: the body is
        // indented, so the parser reads a fence there as indented code.
        let region = code
            .get(next_region)
            .filter(|r| indents.is_empty() && r.range.start < line_full_end);

        let mut mask_from: Option<usize> = None;
        let lead = leading_cols(content);
        let t = content.trim_start();
        let t_abs = content_abs + (content.len() - t.len());
        if let Some(region) = region {
            let first_line = region.range.start >= line_start;
            let last_line = region.range.end <= line_full_end;
            let marker_indent = !indented_code || lead < 4;
            if first_line
                && region.fenced
                && marker_indent
                && line_start + strip + (content.len() - t.len()) == region.range.start
                && let Some((ch, count, info)) = fence_opener(t)
                && let Some((name, title)) = info
                    .trim()
                    .strip_prefix('{')
                    .and_then(|rest| rest.split_once('}'))
                    .filter(|(name, _)| DIRECTIVES.contains(&name.trim()))
            {
                let block = pending.len();
                let title = title.trim();
                pending.push(Pending {
                    admonition: Admonition {
                        kind: name.trim().to_owned(),
                        title_span: title_span(title, t, t_abs),
                        title: nonempty(title),
                        syntax: AdmonitionSyntax::Directive,
                    },
                    start: t_abs,
                    end: line_end,
                });
                directive = Some(Directive {
                    end: region.range.end,
                    ch,
                    count,
                    block,
                });
                mask_from = Some(0);
            } else if !first_line
                && last_line
                && marker_indent
                && let Some(open) = directive.as_ref()
                && open.end == region.range.end
                && is_fence_closer(t, open.ch, open.count)
            {
                pending[open.block].end = line_end;
                directive = None;
                mask_from = Some(0);
            }
        } else if indented_code && lead >= 4 {
            // Not a marker; the parser did not call it code either.
        } else if t.starts_with(":::") {
            mask_from = colon_line(t, t_abs, line_end, &mut pending, &mut colons);
        } else if let Some(rest) = t.strip_prefix("!!!").or_else(|| t.strip_prefix("???"))
            && let Some(parsed) = parse_title_line(rest, true)
        {
            let block = pending.len();
            pending.push(Pending {
                admonition: parsed.into_admonition(t, t_abs, AdmonitionSyntax::Bang),
                start: t_abs,
                end: line_end,
            });
            indents.push(IndentFrame {
                body_cols: leading_cols(line) + 4,
                block,
            });
            mask_from = Some(0);
        } else if let Some(body) = quote
            && !prev_quoted
            && let Some((kind, title)) = parse_alert(body)
            && let Some(bracket) = body.find("[!")
        {
            alert = Some(pending.len());
            pending.push(Pending {
                admonition: Admonition {
                    kind,
                    title_span: title.and_then(|x| title_span(x, content, content_abs)),
                    title: title.and_then(nonempty),
                    syntax: AdmonitionSyntax::Alert,
                },
                start: t_abs,
                end: line_end,
            });
            mask_from = Some(offset_in(content, body) + bracket);
        }
        prev_quoted = quote.is_some() && region.is_none();

        if strip > 0 {
            removed += strip;
            removals.push((out.len(), removed));
        }
        match mask_from {
            Some(from) => {
                out.push_str(&content[..from]);
                out.extend(std::iter::repeat_n(' ', content.len() - from));
            }
            None => out.push_str(content),
        }
        out.push_str(terminator);
    }
    for frame in &indents {
        pending[frame.block].end = last_nonblank_end;
    }
    for (_, block) in &colons {
        pending[*block].end = last_nonblank_end;
    }
    if let Some(open) = &directive {
        pending[open.block].end = last_nonblank_end;
    }
    let admonitions = pending
        .into_iter()
        .map(|p| Block {
            kind: BlockKind::Admonition(p.admonition),
            span: Span::new(p.start, p.end.max(p.start)).expect("end is clamped to start"),
            projection: None,
        })
        .collect();
    Masked {
        text: out,
        removals,
        base,
        admonitions,
    }
}

/// `(start, line without terminator, terminator)` for each line.
fn lines(text: &str) -> impl Iterator<Item = (usize, &str, &str)> {
    let bytes = text.as_bytes();
    let mut pos = 0;
    std::iter::from_fn(move || {
        if pos >= bytes.len() {
            return None;
        }
        let start = pos;
        while pos < bytes.len() && bytes[pos] != b'\n' && bytes[pos] != b'\r' {
            pos += 1;
        }
        let end = pos;
        if bytes.get(pos) == Some(&b'\r') && bytes.get(pos + 1) == Some(&b'\n') {
            pos += 2;
        } else if pos < bytes.len() {
            pos += 1;
        }
        Some((start, &text[start..end], &text[end..pos]))
    })
}

fn leading_cols(line: &str) -> usize {
    line.bytes()
        .map_while(|b| match b {
            b' ' => Some(1),
            b'\t' => Some(4),
            _ => None,
        })
        .sum()
}

/// Bytes of leading whitespace that cover `cols` columns, at most.
fn strip_cols(line: &str, cols: usize) -> usize {
    let mut seen = 0;
    let mut bytes = 0;
    for b in line.bytes() {
        if seen >= cols {
            break;
        }
        seen += match b {
            b' ' => 1,
            b'\t' => 4,
            _ => break,
        };
        bytes += 1;
    }
    bytes
}

fn nonempty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_owned())
}

/// Byte offset of `part` inside `whole`. `part` must be a subslice of `whole`.
fn offset_in(whole: &str, part: &str) -> usize {
    part.as_ptr() as usize - whole.as_ptr() as usize
}

/// Source span of `title`, a subslice of `line` that starts at source offset `line_abs`.
fn title_span(title: &str, line: &str, line_abs: usize) -> Option<Span> {
    if title.is_empty() {
        return None;
    }
    let start = line_abs + offset_in(line, title);
    Span::new(start, start + title.len()).ok()
}

/// A fence opener: `(char, run length, info string)`.
fn fence_opener(t: &str) -> Option<(u8, usize, &str)> {
    let ch = *t.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let count = t.bytes().take_while(|&b| b == ch).count();
    (count >= 3).then(|| (ch, count, &t[count..]))
}

fn is_fence_closer(t: &str, ch: u8, count: usize) -> bool {
    let run = t.bytes().take_while(|&b| b == ch).count();
    run >= count && t[run..].trim().is_empty()
}

/// The text after the `>` of a quote line that has at most three columns of indent.
fn quote_body(content: &str) -> Option<&str> {
    if leading_cols(content) >= 4 {
        return None;
    }
    content.trim_start().strip_prefix('>')
}

struct TitleLine<'a> {
    kind: String,
    title: Option<&'a str>,
}

impl TitleLine<'_> {
    /// `line` is the text the title was cut from and `line_abs` its source offset.
    fn into_admonition(self, line: &str, line_abs: usize, syntax: AdmonitionSyntax) -> Admonition {
        Admonition {
            kind: self.kind,
            title_span: self.title.and_then(|t| title_span(t, line, line_abs)),
            title: self.title.and_then(nonempty),
            syntax,
        }
    }
}

/// Parse `type title` after a `!!!` or `:::` marker. `rest` is the text after the marker.
fn parse_title_line(rest: &str, bang: bool) -> Option<TitleLine<'_>> {
    let rest = if bang {
        let rest = rest.strip_prefix('+').unwrap_or(rest);
        if !rest.starts_with([' ', '\t']) {
            return None;
        }
        rest
    } else {
        rest
    };
    let rest = rest.trim();
    let (kind, title) = if let Some(inner) = rest.strip_prefix('{') {
        let (name, after) = inner.split_once('}')?;
        (name.trim(), after.trim())
    } else {
        let end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(rest.len());
        let after = rest[end..].trim();
        // Docusaurus writes the title in brackets: `:::note[Title]`.
        let after = after
            .strip_prefix('[')
            .and_then(|r| r.strip_suffix(']'))
            .unwrap_or(after);
        // The name must end at a blank or a bracket, not in the middle of other text.
        if !rest[end..].is_empty() && !rest[end..].starts_with([' ', '\t', '[']) {
            return None;
        }
        (&rest[..end], after)
    };
    let valid = !kind.is_empty()
        && kind
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    if !valid {
        return None;
    }
    let title = unquote(title);
    Some(TitleLine {
        kind: kind.to_lowercase(),
        title: (!title.is_empty()).then_some(title),
    })
}

fn unquote(s: &str) -> &str {
    for q in ['"', '\''] {
        if let Some(inner) = s.strip_prefix(q).and_then(|r| r.strip_suffix(q)) {
            return inner;
        }
    }
    s
}

/// Handle a line that starts with `:::`. `t` is the line without indentation. Returns
/// where masking starts, when it does.
fn colon_line(
    t: &str,
    t_abs: usize,
    line_end: usize,
    pending: &mut Vec<Pending>,
    colons: &mut Vec<(usize, usize)>,
) -> Option<usize> {
    let count = t.bytes().take_while(|&b| b == b':').count();
    let rest = &t[count..];
    if rest.trim().is_empty() {
        if let Some(&(open, block)) = colons.last()
            && count >= open
        {
            pending[block].end = line_end;
            colons.pop();
            return Some(0);
        }
        return None;
    }
    let parsed = parse_title_line(rest, false)?;
    let block = pending.len();
    pending.push(Pending {
        admonition: parsed.into_admonition(t, t_abs, AdmonitionSyntax::Colon),
        start: t_abs,
        end: line_end,
    });
    colons.push((count, block));
    Some(0)
}

/// Parse the text after `>` of a GFM alert or Obsidian callout: `[!TYPE]`, an optional
/// fold marker, and a title.
fn parse_alert(body: &str) -> Option<(String, Option<&str>)> {
    let rest = body.trim_start().strip_prefix("[!")?;
    let (kind, after) = rest.split_once(']')?;
    let valid = !kind.is_empty()
        && kind
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid {
        return None;
    }
    let after = after.strip_prefix(['+', '-']).unwrap_or(after).trim();
    Some((kind.to_lowercase(), (!after.is_empty()).then_some(after)))
}
