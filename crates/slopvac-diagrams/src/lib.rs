//! Extract label text from diagram sources.
//!
//! Experimental. Each extractor turns a diagram into [`Fragment`]s: short runs of
//! label text with the byte range they came from. The rules lint a fragment as a
//! label, not as a sentence. The shape mirrors the `Segment` of the plugin
//! protocol (`span`, `context`, `text`) so the same extractors can sit behind
//! `extractor/extract` later.

mod fences;
mod mermaid;

pub use fences::{DiagramBlock, find_markdown_blocks};

use std::ops::Range;

/// What a fragment is in the diagram. Rules use it for exceptions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// A node, state, participant, entity or action.
    Node,
    /// A label on an edge, transition, relationship or flow.
    Edge,
    /// A message between sequence participants.
    Message,
    /// A note or annotation attached to a component.
    Note,
    /// A subgraph, swimlane, package or other container title.
    Group,
    /// The title or description of the whole diagram.
    Title,
    /// A free text box, such as a legend.
    Text,
    /// A class member or an entity attribute.
    Member,
}

impl Role {
    /// The role name used in logical locations.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Role::Node => "node",
            Role::Edge => "edge",
            Role::Message => "message",
            Role::Note => "note",
            Role::Group => "group",
            Role::Title => "title",
            Role::Text => "text",
            Role::Member => "member",
        }
    }
}

/// The context tag every diagram fragment carries.
pub const CONTEXT: &str = "diagram-label";

/// One run of label text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment {
    /// Half-open byte range in the host file.
    pub span: Range<usize>,
    /// The converted text when it differs from the bytes in `span`.
    pub text: Option<String>,
    /// What the label is.
    pub role: Role,
    /// A stable name for SARIF logical locations, such as `mermaid.flowchart/node[A]`.
    pub logical: String,
}

impl Fragment {
    /// Whether an edit can replace `span` byte for byte.
    #[must_use]
    pub fn fixable(&self) -> bool {
        self.text.is_none()
    }
}

/// The result of extracting one diagram.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extraction {
    /// The diagram type, such as `flowchart`. `None` when it is unsupported.
    pub diagram: Option<String>,
    /// Fragments in source order.
    pub fragments: Vec<Fragment>,
}

/// A diagram source format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Mermaid text.
    Mermaid,
}

impl Format {
    /// Pick a format from a fenced-code language word.
    #[must_use]
    pub fn from_lang(lang: &str) -> Option<Format> {
        match lang.to_ascii_lowercase().as_str() {
            "mermaid" => Some(Format::Mermaid),
            _ => None,
        }
    }

    /// Pick a format from a file name, or `None` when it is not a diagram.
    #[must_use]
    pub fn from_path(path: &str) -> Option<Format> {
        let lower = path.to_ascii_lowercase();
        match lower.rsplit('.').next()? {
            "mmd" | "mermaid" => Some(Format::Mermaid),
            _ => None,
        }
    }
}

/// Extract the fragments of a diagram source whose first byte sits at `base` in the host file.
#[must_use]
pub fn extract(format: Format, source: &str, base: usize) -> Extraction {
    match format {
        Format::Mermaid => mermaid::extract(source, base),
    }
}
