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
//!
//! # Input size
//!
//! The crate does not cap its input. The caller must pass at most
//! 64 MiB (`slopvac_fs::MAX_DOCUMENT_BYTES`) of text, and each slopvac entry point enforces
//! that cap before it calls in. The caps are listed in `docs/contract/policies.md`.

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
