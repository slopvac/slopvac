//! Document parsing: Markdown, MDX and HTML blocks with exact byte positions.
//!
//! [`parse_markdown`], [`parse_mdx`] and [`parse_html`] turn a document body into a
//! [`Document`]: a list of [`Block`]s in source order, each with a byte
//! [`slopvac_model::Span`] into the original source. Prose blocks carry a [`Projection`]
//! that maps the text rules read back to source bytes. See `docs/contract/positions.md`.
//!
//! Front matter detection is a separate concern. The caller passes the body span and the
//! parser reads only that range, but every position it returns is relative to the whole
//! source.
#![warn(clippy::indexing_slicing, clippy::string_slice)]

mod align;
mod code;
mod document;
mod html;
mod jsexpr;
mod markdown;
mod mask;
mod mdx;
mod projection;
#[cfg(test)]
mod work;

/// The span from `start` to `end`, with `end` raised to `start` when it falls short. Callers
/// pass offsets that already ascend, so the raise does not change anything.
pub(crate) fn span_between(start: usize, end: usize) -> slopvac_model::Span {
    slopvac_model::Span::new(start, end.max(start))
        .unwrap_or_else(|_| slopvac_model::Span::empty(start))
}

/// The part of `text` in `range`. Offsets in this crate come from the Markdown and HTML
/// scanners, from searches for ASCII delimiters, and from `char_indices`, so they lie on
/// character boundaries and the empty fallback does not occur.
pub(crate) fn sub<I: std::slice::SliceIndex<str, Output = str>>(text: &str, range: I) -> &str {
    text.get(range).unwrap_or_default()
}

pub use document::{
    Admonition, AdmonitionSyntax, Block, BlockKind, Container, Document, HtmlBlock, ParseError,
};
#[doc(hidden)]
pub use html::{
    MAX_HTML_BYTES, Stats, WORK_BASE, WORK_PER_BYTE, parse_html_budget, parse_html_segments_stats,
    parse_html_stats,
};
pub use html::{parse_html, parse_html_segments};
pub use markdown::parse_markdown;
pub use mdx::{mdx_syntax_spans, mdx_syntax_spans_counted, parse_mdx};
pub use projection::{Projection, Token, TokenKind};
