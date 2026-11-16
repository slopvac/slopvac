use crate::span::{LineCol, PositionError, Span};

const BOM: &str = "\u{feff}";

/// How many indexes have been built in this process. An index scans every byte of its text,
/// so a test that counts builds shows whether a caller builds one per document or one per
/// fragment.
#[cfg(feature = "test-counters")]
static INDEXES_BUILT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The number of [`SourceIndex`] values built so far in this process.
#[cfg(feature = "test-counters")]
#[doc(hidden)]
#[must_use]
pub fn indexes_built() -> usize {
    INDEXES_BUILT.load(std::sync::atomic::Ordering::Relaxed)
}

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

/// Whether a leading BOM sits outside line 1 (files) or inside it (LSP buffers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BomMode {
    /// A file on disk: the BOM belongs to no line and to no column.
    Disk,
    /// An LSP buffer: U+FEFF is an ordinary character of line 1.
    Buffer,
}

/// The `positionEncoding` an LSP server answers with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionEncoding {
    /// `utf-8`.
    Utf8,
    /// `utf-16`, the protocol default.
    Utf16,
    /// `utf-32`.
    Utf32,
}

impl PositionEncoding {
    /// Pick the server encoding from the client's offer: `utf-8`, then `utf-32`, then
    /// `utf-16`. No offer, or an empty one, means `utf-16`.
    #[must_use]
    pub fn negotiate(offered: Option<&[&str]>) -> Self {
        let offered = offered.unwrap_or(&[]);
        if offered.contains(&"utf-8") {
            Self::Utf8
        } else if offered.contains(&"utf-32") {
            Self::Utf32
        } else {
            Self::Utf16
        }
    }

    /// The LSP name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::Utf16 => "utf-16",
            Self::Utf32 => "utf-32",
        }
    }

    /// The column unit this encoding counts in.
    #[must_use]
    pub const fn unit(self) -> ColumnUnit {
        match self {
            Self::Utf8 => ColumnUnit::Utf8,
            Self::Utf16 => ColumnUnit::Utf16,
            Self::Utf32 => ColumnUnit::CodePoint,
        }
    }
}

/// Line table over one source text. It maps validated byte offsets to line and column.
///
/// Line terminators are LF, CRLF and a CR that does not precede LF. In
/// [`BomMode::Disk`] a leading BOM counts in offsets and never in columns. In
/// [`BomMode::Buffer`] it is a character of line 1.
#[derive(Debug, Clone)]
pub struct SourceIndex<'a> {
    text: &'a str,
    bom_len: usize,
    mode: BomMode,
    line_starts: Vec<usize>,
}

impl<'a> SourceIndex<'a> {
    /// Index a file read from disk.
    #[must_use]
    pub fn disk(text: &'a str) -> Self {
        Self::new(text, BomMode::Disk)
    }

    /// Index an LSP buffer.
    #[must_use]
    pub fn buffer(text: &'a str) -> Self {
        Self::new(text, BomMode::Buffer)
    }

    /// Index `text`. Offsets are byte offsets into this exact string.
    #[must_use]
    pub fn new(text: &'a str, mode: BomMode) -> Self {
        #[cfg(feature = "test-counters")]
        INDEXES_BUILT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let bom_len = if text.starts_with(BOM) { BOM.len() } else { 0 };
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
        if self.mode == BomMode::Disk && offset > 0 && offset < self.bom_len {
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

    /// Check that both ends of `span` are valid for this text.
    pub fn check_span(&self, span: Span) -> Result<(), PositionError> {
        self.check_offset(span.start())?;
        self.check_offset(span.end())
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
        #[expect(
            clippy::expect_used,
            clippy::unwrap_in_result,
            reason = "line_idx + 1 and width + 1 are at least one, which LineCol::new accepts"
        )]
        let at = LineCol::new(line_idx + 1, width + 1).expect("line and column start at one");
        Ok(at)
    }

    /// One-based line and column in code points.
    pub fn line_col(&self, offset: usize) -> Result<LineCol, PositionError> {
        self.line_col_in(offset, ColumnUnit::CodePoint)
    }

    /// The start and exclusive end of `span` in `unit`.
    pub fn locate_span(
        &self,
        span: Span,
        unit: ColumnUnit,
    ) -> Result<(LineCol, LineCol), PositionError> {
        self.check_span(span)?;
        Ok((
            self.line_col_in(span.start(), unit)?,
            self.line_col_in(span.end(), unit)?,
        ))
    }

    /// The position of the last character a non-empty span includes.
    ///
    /// For a span that ends after CRLF this is the CR. An empty span returns its own
    /// position. Code Climate uses it for inclusive ends.
    pub fn locate_last_included(
        &self,
        span: Span,
        unit: ColumnUnit,
    ) -> Result<LineCol, PositionError> {
        self.check_span(span)?;
        if span.is_empty() {
            return self.line_col_in(span.start(), unit);
        }
        let before = &self.text[..span.end()];
        let mut last = before
            .char_indices()
            .next_back()
            .map_or(span.start(), |(i, _)| i);
        if before.ends_with("\r\n") {
            last -= 1;
        }
        self.line_col_in(last.max(span.start()), unit)
    }

    /// A zero-based LSP position `(line, character)` in `encoding`.
    pub fn lsp_position(
        &self,
        offset: usize,
        encoding: PositionEncoding,
    ) -> Result<(usize, usize), PositionError> {
        let p = self.line_col_in(offset, encoding.unit())?;
        Ok((p.line() - 1, p.column() - 1))
    }
}
