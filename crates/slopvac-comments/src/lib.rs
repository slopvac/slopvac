//! Comment extraction from source code.
//!
//! [`extract()`] finds the comments in a source file and returns their prose as
//! [`Fragment`]s. Each fragment has the text a rule reads and a [`Projection`] that maps
//! every character back to its exact UTF-8 byte in the source. The delimiters (`//`,
//! `#`, `--`, `/* */`, a leading `*`) are gaps, as in `docs/contract/positions.md`.
//!
//! # Backends
//!
//! The default is a pure-Rust lexer ([`extract_lexical`]) that ports the 2.x projection:
//! one pass that knows block comments and strings, so a `//` inside a string is not a
//! comment. It links no C code. A tree-sitter grammar reads a language only when the
//! grammar has no external scanner and its `lang-*` feature is on: C, Go, Java and
//! Proto. Scanner-bearing grammars ship C code that reads and writes raw memory,
//! and the F#, Kotlin and Swift scanners hung or corrupted memory on hostile input, so
//! none is linked. [`Language::backend`] reports which backend a language uses.
//!
//! # What is extracted
//!
//! - Every comment, as 2.x did. 2.x skipped nothing: not shebangs, license headers,
//!   `SPDX-` lines, `# type:` or `// eslint-disable` directives. Neither does this
//!   crate. A fragment records its [`CommentKind`] so a caller can tell a doc comment
//!   from a plain one.
//! - Strings are code, as in 2.x, with one exception. A Python string that is the first
//!   statement of a module, class or function body is extracted as a
//!   [`CommentKind::Doc`] fragment, with its prefix and quotes as gaps. 2.x blanked
//!   these. Byte strings and f-strings are not docstrings in Python and stay code.
//!   Elixir and Julia docstrings are still strings.
//! - Consecutive own-line comments with the same marker form one fragment per
//!   paragraph, joined by a synthetic [`TokenKind::Wrap`] space. A blank comment line
//!   starts a new paragraph. A comment after code on its line stands alone.
//!
//! # Hand-off
//!
//! Extraction is all this crate does. `slopvac-engine` runs the rules over the fragments:
//! see `Input::comments` there.
#![warn(clippy::indexing_slicing, clippy::string_slice)]
#![allow(
    clippy::indexing_slicing,
    clippy::string_slice,
    reason = "TEMPORARY: burn-down in slopvac-8l4.3.50"
)]

mod docstring;
mod extract;
mod language;
mod lexer;
mod span;
mod strip;
#[cfg(any(
    feature = "lang-c",
    feature = "lang-go",
    feature = "lang-java",
    feature = "lang-proto"
))]
mod treesitter;

pub use extract::{
    ExtractError, Extraction, Limits, Stats, extract, extract_lexical, extract_with,
};
pub use language::{Backend, Language};
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
