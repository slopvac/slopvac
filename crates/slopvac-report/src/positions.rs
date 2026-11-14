//! Span to line and column conversion (docs/contract/positions.md).
//!
//! A span is the only stored position. [`SourceMap`] derives display
//! coordinates from it in any [`Unit`].

use std::fmt;

/// How a column counts characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// UTF-8 bytes.
    Utf8,
    /// UTF-16 code units.
    Utf16,
    /// Unicode code points.
    Utf32,
}

/// Whether a leading BOM sits outside line 1 (files) or inside it (LSP buffers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BomMode {
    /// File on disk: the BOM belongs to no line.
    Disk,
    /// LSP buffer: U+FEFF is an ordinary character of line 1.
    Buffer,
}

/// A 1-based line and column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    /// 1-based line.
    pub line: u32,
    /// 1-based column in the requested unit.
    pub column: u32,
}

/// A byte range `[start, end)` in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteSpan {
    /// First byte.
    pub start: usize,
    /// One past the last byte.
    pub end: usize,
}

/// Why an offset or span is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionError {
    /// Past the end of the file.
    PastEnd,
    /// Inside a multi-byte code point.
    InsideCodePoint,
    /// Inside the byte order mark.
    InsideBom,
    /// Between the CR and LF of one CRLF.
    InsideCrlf,
    /// The span starts after it ends.
    StartAfterEnd,
}

impl fmt::Display for PositionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::PastEnd => "offset is past the end of the file",
            Self::InsideCodePoint => "offset is inside a code point",
            Self::InsideBom => "offset is inside the byte order mark",
            Self::InsideCrlf => "offset is between CR and LF",
            Self::StartAfterEnd => "span starts after it ends",
        })
    }
}

impl std::error::Error for PositionError {}

/// Source text with a line table.
#[derive(Debug, Clone)]
pub struct SourceMap<'a> {
    text: &'a str,
    bom_len: usize,
    mode: BomMode,
    line_starts: Vec<usize>,
}

impl<'a> SourceMap<'a> {
    /// Index `text`. In [`BomMode::Disk`] a leading BOM is excluded from line 1.
    #[must_use]
    pub fn new(text: &'a str, mode: BomMode) -> Self {
        let bom_len = if text.starts_with('\u{feff}') { 3 } else { 0 };
        let first = if mode == BomMode::Disk { bom_len } else { 0 };
        let bytes = text.as_bytes();
        let mut line_starts = vec![first];
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\n' => line_starts.push(i + 1),
                b'\r' => {
                    if bytes.get(i + 1) == Some(&b'\n') {
                        i += 1;
                    }
                    line_starts.push(i + 1);
                }
                _ => {}
            }
            i += 1;
        }
        Self {
            text,
            bom_len,
            mode,
            line_starts,
        }
    }

    /// Check that `offset` is a valid position.
    pub fn check_offset(&self, offset: usize) -> Result<(), PositionError> {
        let bytes = self.text.as_bytes();
        if offset > bytes.len() {
            return Err(PositionError::PastEnd);
        }
        if self.mode == BomMode::Disk && (1..self.bom_len).contains(&offset) {
            return Err(PositionError::InsideBom);
        }
        if !self.text.is_char_boundary(offset) {
            return Err(PositionError::InsideCodePoint);
        }
        if offset > 0 && bytes[offset - 1] == b'\r' && bytes.get(offset) == Some(&b'\n') {
            return Err(PositionError::InsideCrlf);
        }
        Ok(())
    }

    /// Check both ends and their order.
    pub fn check_span(&self, span: ByteSpan) -> Result<(), PositionError> {
        self.check_offset(span.start)?;
        self.check_offset(span.end)?;
        if span.start > span.end {
            return Err(PositionError::StartAfterEnd);
        }
        Ok(())
    }

    /// The line and column of `offset` in `unit`.
    pub fn locate(&self, offset: usize, unit: Unit) -> Result<LineCol, PositionError> {
        self.check_offset(offset)?;
        let idx = self.line_starts.partition_point(|&s| s <= offset).max(1) - 1;
        let prefix = &self.text[self.line_starts[idx].min(offset)..offset];
        let width = match unit {
            Unit::Utf8 => prefix.len(),
            Unit::Utf16 => prefix.encode_utf16().count(),
            Unit::Utf32 => prefix.chars().count(),
        };
        Ok(LineCol {
            line: u32::try_from(idx + 1).unwrap_or(u32::MAX),
            column: u32::try_from(width + 1).unwrap_or(u32::MAX),
        })
    }

    /// The start and exclusive end of `span` in `unit`.
    pub fn locate_span(
        &self,
        span: ByteSpan,
        unit: Unit,
    ) -> Result<(LineCol, LineCol), PositionError> {
        self.check_span(span)?;
        Ok((self.locate(span.start, unit)?, self.locate(span.end, unit)?))
    }

    /// The position of the last character a non-empty span includes.
    ///
    /// For a span that ends after CRLF this is the CR. An empty span returns
    /// its own position. Code Climate uses it for inclusive ends.
    pub fn locate_last_included(
        &self,
        span: ByteSpan,
        unit: Unit,
    ) -> Result<LineCol, PositionError> {
        self.check_span(span)?;
        if span.start == span.end {
            return self.locate(span.start, unit);
        }
        let before = &self.text[..span.end];
        let mut last = before
            .char_indices()
            .next_back()
            .map_or(span.start, |(i, _)| i);
        if before.ends_with("\r\n") {
            last -= 1;
        }
        self.locate(last.max(span.start), unit)
    }

    /// A 0-based LSP position in `unit`. Build the map with [`BomMode::Buffer`].
    pub fn lsp_position(&self, offset: usize, unit: Unit) -> Result<(u32, u32), PositionError> {
        let p = self.locate(offset, unit)?;
        Ok((p.line - 1, p.column - 1))
    }
}

/// The encoding an LSP server answers with, given the client's offered encodings.
///
/// Server preference is `utf-8`, then `utf-32`, then `utf-16`. No offer means `utf-16`.
#[must_use]
pub fn negotiate_encoding(offered: Option<&[&str]>) -> (&'static str, Unit) {
    let offered = offered.unwrap_or(&[]);
    if offered.contains(&"utf-8") {
        ("utf-8", Unit::Utf8)
    } else if offered.contains(&"utf-32") {
        ("utf-32", Unit::Utf32)
    } else {
        ("utf-16", Unit::Utf16)
    }
}
