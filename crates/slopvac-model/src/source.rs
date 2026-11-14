use crate::span::{LineCol, PositionError, Span};

const BOM: &str = "\u{feff}";

/// Unit of a column number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnUnit {
    /// UTF-8 bytes.
    Utf8,
    /// UTF-16 code units.
    Utf16,
    /// Unicode code points. This is the default for `LineCol`.
    CodePoint,
}

/// Line table over one source text. It maps validated byte offsets to line and column.
///
/// Line terminators are LF, CRLF and a CR that does not precede LF. For a file ([`Self::new`])
/// a leading BOM counts in offsets and never in columns. For an editor buffer
/// ([`Self::new_buffer`]) U+FEFF is an ordinary character of line 1.
#[derive(Debug, Clone)]
pub struct SourceIndex<'a> {
    text: &'a str,
    bom_len: usize,
    line_starts: Vec<usize>,
}

impl<'a> SourceIndex<'a> {
    /// Index `text`. Offsets are byte offsets into this exact string.
    #[must_use]
    pub fn new(text: &'a str) -> Self {
        let bom_len = if text.starts_with(BOM) { BOM.len() } else { 0 };
        Self::with_bom_len(text, bom_len)
    }

    /// Index an editor buffer, as LSP counts it: a leading U+FEFF is one character of line 1
    /// and offsets inside it are not special. Offsets are bytes of the buffer text.
    #[must_use]
    pub fn new_buffer(text: &'a str) -> Self {
        Self::with_bom_len(text, 0)
    }

    fn with_bom_len(text: &'a str, bom_len: usize) -> Self {
        let bytes = text.as_bytes();
        let mut line_starts = vec![bom_len];
        let mut i = bom_len;
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
            line_starts,
        }
    }

    /// The indexed text.
    #[must_use]
    pub const fn text(&self) -> &'a str {
        self.text
    }

    /// Number of lines. A file that ends with a terminator has a final empty line.
    #[must_use]
    pub const fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Check that `offset` is valid for this text.
    pub fn check_offset(&self, offset: usize) -> Result<(), PositionError> {
        let bytes = self.text.as_bytes();
        if offset > bytes.len() {
            return Err(PositionError::PastEnd(offset));
        }
        if offset > 0 && offset < self.bom_len {
            return Err(PositionError::InsideBom(offset));
        }
        if !self.text.is_char_boundary(offset) {
            return Err(PositionError::InsideCodePoint(offset));
        }
        if offset > 0 && bytes[offset - 1] == b'\r' && bytes.get(offset) == Some(&b'\n') {
            return Err(PositionError::InsideCrlf(offset));
        }
        Ok(())
    }

    /// Check that both ends of `span` are valid and in order.
    pub fn check_span(&self, span: Span) -> Result<(), PositionError> {
        if span.start > span.end {
            return Err(PositionError::Reversed {
                start: span.start,
                end: span.end,
            });
        }
        self.check_offset(span.start)?;
        self.check_offset(span.end)
    }

    /// One-based line and column of a valid offset, counted in `unit`.
    pub fn line_col_in(&self, offset: usize, unit: ColumnUnit) -> Result<LineCol, PositionError> {
        self.check_offset(offset)?;
        let line_idx = self
            .line_starts
            .partition_point(|&s| s <= offset)
            .saturating_sub(1);
        let start = self.line_starts[line_idx];
        let prefix = &self.text[start..offset.max(start)];
        let width = match unit {
            ColumnUnit::Utf8 => prefix.len(),
            ColumnUnit::Utf16 => prefix.encode_utf16().count(),
            ColumnUnit::CodePoint => prefix.chars().count(),
        };
        Ok(LineCol {
            line: line_idx + 1,
            column: width + 1,
        })
    }

    /// One-based line and column in code points.
    pub fn line_col(&self, offset: usize) -> Result<LineCol, PositionError> {
        self.line_col_in(offset, ColumnUnit::CodePoint)
    }
}
