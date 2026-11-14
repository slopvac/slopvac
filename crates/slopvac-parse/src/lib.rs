//! Document parsing: Markdown and MDX blocks with exact byte positions.
//!
//! [`parse_markdown`] and [`parse_mdx`] turn a document body into a [`Document`]: a list
//! of [`Block`]s in source order, each with a byte [`Span`] into the original source.
//! Prose blocks carry a [`Projection`] that maps the text rules read back to source
//! bytes. See `docs/contract/positions.md`.
//!
//! Front matter detection is a separate concern. The caller passes the body span and the
//! parser reads only that range, but every position it returns is relative to the whole
//! source.

mod align;
mod code;
mod document;
mod markdown;
mod mask;
mod mdx;
mod projection;

pub use document::{Admonition, AdmonitionSyntax, Block, BlockKind, Document, ParseError};
pub use markdown::parse_markdown;
pub use mdx::parse_mdx;
pub use projection::{Projection, Token, TokenKind};
