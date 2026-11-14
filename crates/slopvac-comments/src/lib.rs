//! Comment and docstring extraction from source code, on tree-sitter.
//!
//! [`extract()`] parses a source file with the grammar for its [`Language`] and returns the
//! prose in its comments as [`Fragment`]s. Each fragment has the text a rule reads and a
//! [`Projection`] that maps every character back to its exact UTF-8 byte in the source.
//! The delimiters (`//`, `#`, `--`, `/* */`, a leading `*`, quotes) are gaps, as in
//! `docs/contract/positions.md`.
//!
//! # What is extracted
//!
//! - Every comment, as 2.x did. 2.x read comments through Vale's scopes and skipped
//!   nothing: not shebangs, license headers, `SPDX-` lines, `# type:` or
//!   `// eslint-disable` directives. Neither does this crate. A fragment records its
//!   [`CommentKind`] so a caller can tell a doc comment from a plain one.
//! - Docstrings: the string statement that documents a Python module, class or
//!   function. Code and other string literals are never extracted.
//! - Consecutive own-line comments with the same marker form one fragment per
//!   paragraph, joined by a synthetic [`TokenKind::Wrap`] space. A blank comment line
//!   starts a new paragraph. A comment after code on its line stands alone.
//!
//! # Hand-off
//!
//! Extraction is all this crate does. Running rules over fragments belongs to
//! `slopvac-engine`.

// STUB(slopvac-8l4.3.26): `slopvac-engine` does not exist yet. When it lands, hand each
// `Fragment` to its rule dispatch with comment-mode rule selection. Until then the only
// consumer is the tests in this crate.
mod docstring;
mod extract;
mod language;
mod span;
mod strip;

pub use extract::{ExtractError, Extraction, Limits, Stats, extract, extract_with};
pub use language::Language;
pub use span::{Projection, Span, SpanError, Token, TokenKind};

/// What a fragment was written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommentKind {
    /// A line comment (`//`, `#`, `--`) or a group of them.
    Line,
    /// A block comment (`/* */`).
    Block,
    /// A documentation comment (`///`, `//!`, `/** */`, `/*! */`).
    Doc,
    /// A docstring: a string literal that documents a definition.
    Docstring,
}

/// One paragraph of comment text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment {
    kind: CommentKind,
    comment: Span,
    projection: Projection,
}

impl Fragment {
    pub(crate) fn new(kind: CommentKind, comment: Span, projection: Projection) -> Self {
        Self {
            kind,
            comment,
            projection,
        }
    }

    /// How the text was written.
    #[must_use]
    pub fn kind(&self) -> CommentKind {
        self.kind
    }

    /// The text a rule reads: delimiters removed, lines joined by one space.
    #[must_use]
    pub fn text(&self) -> &str {
        self.projection.text()
    }

    /// The mapping from [`Fragment::text`] back to source bytes.
    #[must_use]
    pub fn projection(&self) -> &Projection {
        &self.projection
    }

    /// The source bytes of the text, from its first character to its last.
    #[must_use]
    pub fn text_span(&self) -> Span {
        let tokens = self.projection.tokens();
        match (tokens.first(), tokens.last()) {
            (Some(first), Some(last)) => {
                Span::new(first.src.start(), last.src.end()).unwrap_or(self.comment)
            }
            _ => Span::empty(self.comment.start()),
        }
    }

    /// The whole comment, delimiters included. Fragments cut from one comment share it.
    #[must_use]
    pub fn comment_span(&self) -> Span {
        self.comment
    }
}
