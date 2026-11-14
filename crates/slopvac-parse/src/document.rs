//! The parse result.

use slopvac_model::{PositionError, Span};

use crate::Projection;

/// How an admonition was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmonitionSyntax {
    /// `::: type title`, including `MyST` `:::{type} title`.
    Colon,
    /// `!!! type "title"` or the collapsible `??? type "title"`, with an indented body.
    Bang,
    /// A `MyST` fenced directive, such as a `{note}` code fence.
    Directive,
    /// A GFM alert or Obsidian callout: `> [!NOTE] title`.
    Alert,
}

/// An admonition, masked out of the prose. Its children are separate blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admonition {
    /// Lowercase type, such as `note` or `warning`.
    pub kind: String,
    /// The title text, when the author wrote one.
    pub title: Option<String>,
    /// Source bytes of the title text, without quotes.
    pub title_span: Option<Span>,
    /// The syntax that introduced it.
    pub syntax: AdmonitionSyntax,
}

/// What a block is.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlockKind {
    /// A paragraph, including the text of a tight list item.
    Paragraph,
    /// A heading. Attributes such as `{#id .class}` are not part of its prose.
    Heading {
        /// Level, 1 to 6.
        level: u8,
    },
    /// A table. Its cells follow as separate blocks.
    Table,
    /// A table cell.
    TableCell {
        /// Row, where 0 is the header row.
        row: usize,
        /// Column, from 0.
        column: usize,
    },
    /// An admonition container. The blocks inside it follow.
    Admonition(Admonition),
    /// A code block.
    CodeBlock {
        /// First word of the info string, when there is one.
        lang: Option<String>,
        /// The whole info string, trimmed.
        info: Option<String>,
        /// Source bytes of the body, without the fence lines.
        body: Span,
        /// Whether the block is fenced. An indented block has no info string.
        fenced: bool,
    },
}

/// A container that holds a block: a list item or a block quote.
///
/// A table cell and an admonition are not listed here. A cell is a [`BlockKind::TableCell`]
/// and an admonition is a [`BlockKind::Admonition`] block whose span contains its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Container {
    /// A list item.
    ListItem {
        /// Whether the list is ordered.
        ordered: bool,
        /// How many lists enclose the item, counting its own: 1 for a top-level list.
        depth: usize,
    },
    /// A block quote.
    BlockQuote,
}

/// One block and where it sits in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// What the block is.
    pub kind: BlockKind,
    /// Source bytes of the whole block, without its final line terminator.
    pub span: Span,
    /// The prose of a paragraph, heading or table cell.
    pub projection: Option<Projection>,
    /// The lists and block quotes that hold the block, outermost first. Empty at the top
    /// level. A parser that cannot tell leaves it empty.
    pub containers: Vec<Container>,
}

/// A parsed document.
///
/// Blocks are in source order. A container such as an admonition or table comes before
/// the blocks inside it, and its span contains theirs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    /// Blocks in source order.
    pub blocks: Vec<Block>,
    /// Inline code spans, backticks included, in source order. Rules skip these ranges.
    pub code_spans: Vec<Span>,
}

/// Why a document did not parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// The body span does not fit the source.
    #[error("invalid body span: {0}")]
    Body(#[from] PositionError),
    /// MDX syntax error.
    #[error("MDX syntax error: {0}")]
    Mdx(String),
    /// The document needs more work than the parse budget allows. The check is incomplete.
    #[error("the document needs more work than the parse budget allows")]
    Budget,
    /// The document is longer than the parser accepts. The check is incomplete.
    #[error("the document is longer than the parser accepts")]
    TooLarge,
}
