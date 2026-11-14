//! Markdown through pulldown-cmark.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use slopvac_model::{SourceIndex, Span};

use crate::align::{ProseBuilder, align};
use crate::code::{code_block, trim_terminator};
use crate::mask::{Masked, scan};
use crate::{Block, BlockKind, Document, ParseError, TokenKind};

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

/// Parse the `body` range of Markdown `source`.
///
/// `body` is where the caller found the content to read, after any front matter. Every
/// position in the result is a byte offset into `source`.
///
/// # Errors
/// Returns [`ParseError::Body`] when `body` is not a valid span of `source`.
pub fn parse_markdown(source: &str, body: Span) -> Result<Document, ParseError> {
    let body = resolve_body(source, body)?;
    let masked = scan(&source[body.start()..body.end()], body.start(), true);
    let mut collector = Collector::new(source, &masked);
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES;
    for (event, range) in Parser::new_ext(&masked.text, options).into_offset_iter() {
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
}

struct Collector<'a> {
    source: &'a str,
    masked: &'a Masked,
    blocks: Vec<Block>,
    code_spans: Vec<Span>,
    open: Option<Open>,
    code: Option<(bool, Range<usize>)>,
    image_depth: usize,
    /// Where the previous event ended, in masked bytes.
    prev_end: usize,
    row: usize,
    column: usize,
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
        }
    }

    /// A source span with its terminator trimmed.
    fn span(&self, range: &Range<usize>) -> Span {
        trim_terminator(self.source, self.masked.span(range.start, range.end))
    }

    fn start_block(&mut self, kind: BlockKind, range: Range<usize>) {
        self.flush();
        self.open = Some(Open {
            kind,
            tag: Some(range),
            inline: None,
            prose: ProseBuilder::default(),
        });
    }

    /// The open block, or a tight-list paragraph opened now.
    fn prose(&mut self) -> &mut Open {
        self.open.get_or_insert_with(|| Open {
            kind: BlockKind::Paragraph,
            tag: None,
            inline: None,
            prose: ProseBuilder::default(),
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
            Event::Text(text) => {
                if self.code.is_none() && self.image_depth == 0 {
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
                self.code = Some((matches!(kind, CodeBlockKind::Fenced(_)), range));
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
                if let Some((fenced, range)) = self.code.take() {
                    let span = self.span(&range);
                    self.blocks.push(Block {
                        kind: code_block(self.source, span, fenced),
                        span,
                        projection: None,
                    });
                }
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
