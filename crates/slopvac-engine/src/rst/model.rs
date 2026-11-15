//! The JSON the driver writes, and the checks on it.

use serde::Deserialize;

/// What the driver printed.
#[derive(Debug, Deserialize)]
pub(super) struct Output {
    /// The docutils version that ran.
    pub(super) docutils: String,
    /// Prose, code and admonition nodes in source order.
    pub(super) nodes: Vec<Node>,
}

/// One node of the docutils tree.
#[derive(Debug, Deserialize)]
pub(super) struct Node {
    pub(super) tag: String,
    /// First source line, 1-based. `None` when docutils recorded none.
    pub(super) start: Option<usize>,
    /// Last source line.
    pub(super) end: Option<usize>,
    #[serde(default)]
    pub(super) code: bool,
    /// The text of a code block.
    #[serde(default)]
    pub(super) text: String,
    #[serde(default)]
    pub(super) lang: Option<String>,
    /// Admonition kind.
    #[serde(default)]
    pub(super) adm: Option<String>,
    #[serde(default)]
    pub(super) title: Option<String>,
    /// Heading level, for a section title.
    #[serde(default)]
    pub(super) level: Option<u8>,
    /// Table row, column and column span, for the text of a table cell.
    #[serde(default)]
    pub(super) cell: Option<(usize, usize, usize)>,
    /// Whether the node is a table. Its lines run from its first to its last cell.
    #[serde(default)]
    pub(super) table: bool,
    #[serde(default)]
    pub(super) pieces: Vec<Piece>,
    #[serde(default)]
    pub(super) containers: Vec<Container>,
}

/// A child of a prose node.
#[derive(Debug, Deserialize)]
pub(super) struct Piece {
    /// `t` text, `i` inline markup, `c` inline code, `o` markup with no prose.
    pub(super) k: String,
    #[serde(default, deserialize_with = "squash")]
    pub(super) t: String,
    /// The inline node as written in the source, with each whitespace run as one space.
    #[serde(default, deserialize_with = "squash")]
    pub(super) raw: String,
}

/// Collapse every whitespace run to one space, once, while the output is decoded. Matching
/// then never walks a long run of whitespace in driver output again.
fn squash<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let text = String::deserialize(deserializer)?;
    let mut out = String::with_capacity(text.len());
    let mut blank = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !blank {
                out.push(' ');
            }
            blank = true;
        } else {
            out.push(c);
            blank = false;
        }
    }
    Ok(out)
}

/// A list item or block quote around a node.
#[derive(Debug, Deserialize)]
#[serde(tag = "t")]
pub(super) enum Container {
    #[serde(rename = "li")]
    Item { ordered: bool, depth: usize },
    #[serde(rename = "bq")]
    Quote,
}
