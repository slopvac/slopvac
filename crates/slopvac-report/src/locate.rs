//! Per-format coordinates for findings, derived from spans (docs/contract/positions.md).

use slopvac_model::{ColumnUnit, Document, Finding, SourceIndex, Span};

use crate::{RenderError, Sources};

/// A one-based line and column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    /// One-based line.
    pub line: usize,
    /// One-based column in the requested unit.
    pub column: usize,
}

/// Where a finding sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Located {
    /// Start.
    pub start: Pos,
    /// Exclusive end; `None` for a finding with no source range.
    pub end: Option<Pos>,
}

/// A document with its text indexed, when the caller supplied the text.
pub(crate) struct DocIndex<'a> {
    doc: &'a Document,
    index: Option<SourceIndex<'a>>,
    format: &'static str,
}

impl<'a> DocIndex<'a> {
    pub(crate) fn new(doc: &'a Document, sources: &'a dyn Sources, format: &'static str) -> Self {
        Self {
            doc,
            index: sources.text(doc.path.as_str()).map(SourceIndex::disk),
            format,
        }
    }

    fn need_index(&self) -> Result<&SourceIndex<'a>, RenderError> {
        self.index
            .as_ref()
            .ok_or_else(|| RenderError::MissingSource {
                format: self.format,
                path: self.doc.path.as_str().to_owned(),
            })
    }

    fn position_error(&self, source: slopvac_model::PositionError) -> RenderError {
        RenderError::Position {
            path: self.doc.path.as_str().to_owned(),
            reason: source.to_string(),
            source,
        }
    }

    /// Start and exclusive end of `finding` in `unit`.
    ///
    /// Code points come from the finding itself when no text is available.
    pub(crate) fn locate(
        &self,
        finding: &Finding,
        unit: ColumnUnit,
    ) -> Result<Located, RenderError> {
        let start = Pos {
            line: finding.start.line(),
            column: finding.start.column(),
        };
        let Some(extent) = finding.extent else {
            return Ok(Located { start, end: None });
        };
        if let (None, ColumnUnit::CodePoint) = (&self.index, unit) {
            return Ok(Located {
                start,
                end: Some(Pos {
                    line: extent.end.line(),
                    column: extent.end.column(),
                }),
            });
        }
        let span = extent.span;
        let (start, end) = self
            .need_index()?
            .locate_span(span, unit)
            .map_err(|e| self.position_error(e))?;
        Ok(Located {
            start: Pos {
                line: start.line(),
                column: start.column(),
            },
            end: Some(Pos {
                line: end.line(),
                column: end.column(),
            }),
        })
    }

    /// Position of the last character `span` includes, in code points.
    pub(crate) fn last_included(&self, span: Span) -> Result<Pos, RenderError> {
        let p = self
            .need_index()?
            .locate_last_included(span, ColumnUnit::CodePoint)
            .map_err(|e| self.position_error(e))?;
        Ok(Pos {
            line: p.line(),
            column: p.column(),
        })
    }

    /// Start and end of an arbitrary span in `unit`.
    pub(crate) fn locate_edit(
        &self,
        span: Span,
        unit: ColumnUnit,
    ) -> Result<(Pos, Pos), RenderError> {
        let (s, e) = self
            .need_index()?
            .locate_span(span, unit)
            .map_err(|e| self.position_error(e))?;
        Ok((
            Pos {
                line: s.line(),
                column: s.column(),
            },
            Pos {
                line: e.line(),
                column: e.column(),
            },
        ))
    }
}
