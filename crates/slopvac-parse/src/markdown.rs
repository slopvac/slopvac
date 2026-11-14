//! Markdown through pulldown-cmark.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use slopvac_model::{SourceIndex, Span};

use crate::align::{ProseBuilder, align};
use crate::code::{code_kind, trim_terminator};
use crate::mask::{CodeRegion, Masked, scan};
use crate::{Block, BlockKind, Container, Document, ParseError, TokenKind};

const BOM: &str = "\u{feff}";

/// Placeholder text of a masked token. Rules must not read what it stands for.
pub(crate) const MASK: &str = "\0";

/// Resolve and check the body range. A byte order mark at the start is not part of it.
pub(crate) fn resolve_body(source: &str, body: Span) -> Result<Span, ParseError> {
    SourceIndex::disk(source).check_span(body)?;
    let skip = if body.start() == 0 && source.starts_with(BOM) {
        BOM.len().min(body.end())
    } else {
        body.start()
    };
    Ok(Span::new(skip, body.end()).expect("start was checked against end"))
}

/// Sort blocks into source order. Containers come before what they contain.
pub(crate) fn finish(mut blocks: Vec<Block>, mut code_spans: Vec<Span>) -> Document {
    blocks.sort_by_key(|b| (b.span.start(), std::cmp::Reverse(b.span.end())));
    code_spans.sort_by_key(Span::start);
    Document { blocks, code_spans }
}

const OPTIONS: Options = Options::ENABLE_TABLES
    .union(Options::ENABLE_STRIKETHROUGH)
    .union(Options::ENABLE_TASKLISTS)
    .union(Options::ENABLE_HEADING_ATTRIBUTES);

/// Code blocks of `text`, from the parser's own events.
fn code_regions(text: &str) -> Vec<CodeRegion> {
    Parser::new_ext(text, OPTIONS)
        .into_offset_iter()
        .filter_map(|(event, range)| match event {
            Event::Start(Tag::CodeBlock(kind)) => Some(CodeRegion {
                range,
                fenced: matches!(kind, CodeBlockKind::Fenced(_)),
            }),
            _ => None,
        })
        .collect()
}

/// Parse the `body` range of Markdown `source`.
///
/// `body` is where the caller found the content to read, after any front matter. Every
/// position in the result is a byte offset into `source`.
///
/// # Errors
/// Returns [`ParseError::Body`] when `body` is not a valid span of `source`.
pub fn parse_markdown(source: &str, body: Span) -> Result<Document, ParseError> {
    let body = resolve_body(source, body)?;
    let text = &source[body.start()..body.end()];
    // The first pass reads the unmasked body to find out what is code.
    let masked = scan(text, body.start(), &code_regions(text), true);
    let mut collector = Collector::new(source, &masked);
    for (event, range) in Parser::new_ext(&masked.text, OPTIONS).into_offset_iter() {
        collector.event(&event, range);
    }
    collector.flush();
    let Collector {
        mut blocks,
        code_spans,
        ..
    } = collector;
    blocks.extend(masked.admonitions.iter().cloned());
    Ok(finish(blocks, code_spans))
}

/// A prose block under construction.
struct Open {
    kind: BlockKind,
    /// The tag's range, when the block has a tag. Tight list text has none.
    tag: Option<Range<usize>>,
    /// Range of the inline events seen so far.
    inline: Option<Range<usize>>,
    prose: ProseBuilder,
    containers: Vec<Container>,
}

/// A code block under construction.
struct OpenCode {
    /// The info string; `None` for an indented block.
    info: Option<String>,
    range: Range<usize>,
    /// Hull of the block's text events: the body without fences and container prefixes.
    hull: Option<Range<usize>>,
    containers: Vec<Container>,
}

struct Collector<'a> {
    source: &'a str,
    masked: &'a Masked,
    blocks: Vec<Block>,
    code_spans: Vec<Span>,
    open: Option<Open>,
    code: Option<OpenCode>,
    image_depth: usize,
    /// Where the previous event ended, in masked bytes.
    prev_end: usize,
    row: usize,
    column: usize,
    /// The lists and block quotes the parser is inside.
    containers: Vec<Container>,
    /// Whether each enclosing list is ordered.
    lists: Vec<bool>,
}

impl<'a> Collector<'a> {
    const fn new(source: &'a str, masked: &'a Masked) -> Self {
        Self {
            source,
            masked,
            blocks: Vec::new(),
            code_spans: Vec::new(),
            open: None,
            code: None,
            image_depth: 0,
            prev_end: 0,
            row: 0,
            column: 0,
            containers: Vec::new(),
            lists: Vec::new(),
        }
    }

    /// A source span with its terminator trimmed.
    fn span(&self, range: &Range<usize>) -> Span {
        trim_terminator(self.source, self.masked.span(range.start, range.end))
    }

    /// An empty body: the start of the line after the opener.
    fn empty_body(&self, range: &Range<usize>) -> Span {
        let text = &self.masked.text;
        let at = text[range.start..range.end]
            .find('\n')
            .map_or(range.end, |i| range.start + i + 1);
        self.masked.span(at, at)
    }

    fn start_block(&mut self, kind: BlockKind, range: Range<usize>) {
        self.flush();
        self.open = Some(Open {
            kind,
            tag: Some(range),
            inline: None,
            prose: ProseBuilder::default(),
            containers: self.containers.clone(),
        });
    }

    /// The open block, or a tight-list paragraph opened now.
    fn prose(&mut self) -> &mut Open {
        let containers = &self.containers;
        self.open.get_or_insert_with(|| Open {
            kind: BlockKind::Paragraph,
            tag: None,
            inline: None,
            prose: ProseBuilder::default(),
            containers: containers.clone(),
        })
    }

    fn flush(&mut self) {
        let Some(open) = self.open.take() else {
            return;
        };
        let range = open.tag.or(open.inline);
        let trim = matches!(open.kind, BlockKind::Heading { .. });
        if let (Some(range), Some(projection)) = (range, open.prose.finish(trim)) {
            self.blocks.push(Block {
                kind: open.kind,
                span: self.span(&range),
                projection: Some(projection),
                containers: open.containers,
            });
        }
    }

    fn inline_range(&mut self, range: &Range<usize>) {
        let open = self.prose();
        let merged = open.inline.as_ref().map_or(range.clone(), |r| {
            r.start.min(range.start)..r.end.max(range.end)
        });
        open.inline = Some(merged);
    }

    fn event(&mut self, event: &Event<'_>, range: Range<usize>) {
        let prev_end = self.prev_end;
        self.prev_end = if matches!(event, Event::Start(_)) {
            range.start
        } else {
            range.end
        };
        match event {
            Event::Start(tag) => self.start(tag, range),
            Event::End(tag) => self.end(*tag, &range),
            Event::Text(text) if self.code.is_some() => {
                if let Some(code) = self.code.as_mut() {
                    code.hull = Some(code.hull.as_ref().map_or_else(
                        || {
                            // A blank first line comes with its container prefix, and
                            // a CRLF line may start between its CR and LF. The body
                            // starts at the line terminator.
                            let bytes = self.masked.text.as_bytes();
                            let blank = matches!(&**text, "\n" | "\r\n" | "\r");
                            let width = if bytes[..range.end].ends_with(b"\r\n") {
                                2
                            } else {
                                1
                            };
                            let mut start = if blank && range.len() > width {
                                range.end - width
                            } else {
                                range.start
                            };
                            if start > 0 && bytes[start - 1] == b'\r' && bytes[start] == b'\n' {
                                start -= 1;
                            }
                            start..range.end
                        },
                        |h| h.start.min(range.start)..h.end.max(range.end),
                    ));
                }
            }
            Event::Text(text) => {
                if self.image_depth == 0 {
                    // The backslash of an escape sits outside the text's range.
                    let start = if range.start > prev_end
                        && self.masked.text.as_bytes()[range.start - 1] == b'\\'
                        && text.starts_with(|c: char| c.is_ascii_punctuation())
                    {
                        range.start - 1
                    } else {
                        range.start
                    };
                    self.inline_range(&(start..range.end));
                    let (masked, coords) = (&self.masked.text, self.masked);
                    let open = self.prose();
                    align(&mut open.prose, coords, masked, (start, range.end), text);
                }
            }
            Event::Code(_) => {
                if self.code.is_none() && self.image_depth == 0 {
                    self.inline_range(&range);
                    let span = self.masked.span(range.start, range.end);
                    self.code_spans.push(span);
                    self.prose().prose.push(TokenKind::Masked, MASK, span);
                }
            }
            Event::InlineHtml(html) => {
                if html.starts_with("<!--") && self.image_depth == 0 {
                    self.inline_range(&range);
                    let span = self.masked.span(range.start, range.end);
                    self.prose().prose.push(TokenKind::Masked, "", span);
                }
            }
            Event::SoftBreak => {
                if self.code.is_none() && self.image_depth == 0 {
                    self.prose().prose.wrap();
                }
            }
            Event::HardBreak if self.image_depth == 0 && self.prose().prose.has_text() => {
                let span = self.masked.span(range.start, range.end);
                self.prose().prose.push(TokenKind::Collapse, " ", span);
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: &Tag<'_>, range: Range<usize>) {
        match tag {
            Tag::Paragraph => self.start_block(BlockKind::Paragraph, range),
            Tag::Heading { level, .. } => self.start_block(
                BlockKind::Heading {
                    level: *level as u8,
                },
                range,
            ),
            Tag::TableCell => {
                let kind = BlockKind::TableCell {
                    row: self.row,
                    column: self.column,
                };
                self.start_block(kind, range);
            }
            Tag::Table(_) => {
                self.flush();
                self.row = 0;
                let span = self.span(&range);
                self.blocks.push(Block {
                    kind: BlockKind::Table,
                    span,
                    projection: None,
                    containers: self.containers.clone(),
                });
            }
            Tag::TableHead => {
                self.row = 0;
                self.column = 0;
            }
            Tag::TableRow => {
                self.row += 1;
                self.column = 0;
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                self.code = Some(OpenCode {
                    info: match kind {
                        CodeBlockKind::Fenced(info) => Some(info.to_string()),
                        CodeBlockKind::Indented => None,
                    },
                    range,
                    hull: None,
                    containers: self.containers.clone(),
                });
            }
            Tag::List(first) => {
                self.flush();
                self.lists.push(first.is_some());
            }
            Tag::Item => {
                self.flush();
                self.containers.push(Container::ListItem {
                    ordered: self.lists.last().copied().unwrap_or(false),
                    depth: self.lists.len(),
                });
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.containers.push(Container::BlockQuote);
            }
            Tag::Image { .. } => {
                self.inline_range(&range);
                self.image_depth += 1;
            }
            Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } => {
                self.inline_range(&range);
            }
            _ => self.flush(),
        }
    }

    fn end(&mut self, tag: TagEnd, range: &Range<usize>) {
        match tag {
            TagEnd::TableCell => {
                self.flush();
                self.column += 1;
            }
            TagEnd::CodeBlock => {
                if let Some(code) = self.code.take() {
                    let span = self.span(&code.range);
                    let body = match code.hull {
                        Some(hull) => self.masked.span(hull.start, hull.end),
                        None => self.empty_body(&code.range),
                    };
                    self.blocks.push(Block {
                        kind: code_kind(code.info.as_deref(), body),
                        span,
                        projection: None,
                        containers: code.containers,
                    });
                }
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
            }
            TagEnd::Item | TagEnd::BlockQuote(_) => {
                self.flush();
                self.containers.pop();
            }
            TagEnd::Image => {
                self.inline_range(range);
                self.image_depth = self.image_depth.saturating_sub(1);
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                self.inline_range(range);
            }
            _ => self.flush(),
        }
    }
}
