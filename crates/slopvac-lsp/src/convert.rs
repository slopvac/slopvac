//! Byte spans to LSP ranges in the negotiated position encoding.

use lsp_types::{Position, Range};
use slopvac_model::{PositionEncoding, SourceIndex, Span};

use crate::analysis::Item;

/// The LSP range of a byte span, or `None` if the span is not valid for the buffer.
pub(crate) fn span_range(
    index: &SourceIndex<'_>,
    span: Span,
    encoding: PositionEncoding,
) -> Option<Range> {
    Some(Range::new(
        offset_position(index, span.start(), encoding)?,
        offset_position(index, span.end(), encoding)?,
    ))
}

fn offset_position(
    index: &SourceIndex<'_>,
    offset: usize,
    encoding: PositionEncoding,
) -> Option<Position> {
    let (line, character) = index.lsp_position(offset, encoding).ok()?;
    Some(Position::new(
        u32::try_from(line).ok()?,
        u32::try_from(character).ok()?,
    ))
}

/// The range of an item. A whole-document item is empty at the start of its line.
pub(crate) fn item_range(
    index: &SourceIndex<'_>,
    item: &Item,
    encoding: PositionEncoding,
) -> Option<Range> {
    if let Some(span) = item.span {
        return span_range(index, span, encoding);
    }
    let line = u32::try_from(item.line).ok()?;
    Some(Range::new(Position::new(line, 0), Position::new(line, 0)))
}

fn key(p: Position) -> (u32, u32) {
    (p.line, p.character)
}

/// Whether two ranges share a point. Touching ranges count, so a cursor at either edge of a
/// finding finds it.
pub(crate) fn touches(a: Range, b: Range) -> bool {
    key(a.start) <= key(b.end) && key(b.start) <= key(a.end)
}
