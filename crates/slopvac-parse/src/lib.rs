//! Document parsing: HTML blocks with exact byte positions.
//!
//! [`parse_html`] turns a document body into a [`Document`]: a list of [`Block`]s in
//! source order, each with a byte [`Span`] into the original source. Prose blocks carry a
//! [`Projection`] that maps the text rules read back to source bytes. See
//! `docs/contract/positions.md`.

mod document;
mod html;
mod projection;

pub use document::{Admonition, AdmonitionSyntax, Block, BlockKind, Document, ParseError};
pub use html::parse_html;
#[doc(hidden)]
pub use html::{
    MAX_HTML_BYTES, Stats, WORK_BASE, WORK_PER_BYTE, parse_html_budget, parse_html_stats,
};
pub use projection::{Projection, Token, TokenKind};
