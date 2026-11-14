//! Extract label text from diagram sources.
//!
//! Experimental. Each extractor turns a diagram into [`Fragment`]s: short runs of
//! label text with the byte range they came from. The rules lint a fragment as a
//! label, not as a sentence. The shape mirrors the `Segment` of the plugin
//! protocol (`span`, `context`, `text`) so the same extractors can sit behind
//! `extractor/extract` later.

mod excalidraw;
mod fences;
mod json;
mod mermaid;
mod plantuml;

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
    /// `PlantUML` text.
    PlantUml,
    /// An Excalidraw JSON scene.
    Excalidraw,
}

impl Format {
    /// Pick a format from a fenced-code language word.
    #[must_use]
    pub fn from_lang(lang: &str) -> Option<Format> {
        match lang.to_ascii_lowercase().as_str() {
            "mermaid" => Some(Format::Mermaid),
            "plantuml" | "puml" => Some(Format::PlantUml),
            _ => None,
        }
    }

    /// Pick a format from a file name, or `None` when it is not a diagram.
    #[must_use]
    pub fn from_path(path: &str) -> Option<Format> {
        let ext = std::path::Path::new(path).extension()?.to_str()?;
        match ext.to_ascii_lowercase().as_str() {
            "mmd" | "mermaid" => Some(Format::Mermaid),
            "puml" | "plantuml" | "pu" | "wsd" | "iuml" => Some(Format::PlantUml),
            "excalidraw" => Some(Format::Excalidraw),
            _ => None,
        }
    }
}

/// A source line without its terminator.
pub(crate) struct SourceLine<'a> {
    /// Byte offset of the line start.
    pub(crate) start: usize,
    /// The line text, terminator excluded.
    pub(crate) text: &'a str,
    /// Byte offset just after the terminator.
    pub(crate) end: usize,
}

/// Split text into lines at LF, CRLF and a CR that does not precede LF
/// (positions contract, section "Line terminators"). Offsets address the original bytes.
pub(crate) fn lines(src: &str) -> Vec<SourceLine<'_>> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut pos = 0;
    while pos < bytes.len() {
        let terminator = match bytes[pos] {
            b'\r' if bytes.get(pos + 1) == Some(&b'\n') => 2,
            b'\n' | b'\r' => 1,
            _ => {
                pos += 1;
                continue;
            }
        };
        out.push(SourceLine {
            start,
            text: &src[start..pos],
            end: pos + terminator,
        });
        pos += terminator;
        start = pos;
    }
    if start < bytes.len() {
        out.push(SourceLine {
            start,
            text: &src[start..],
            end: bytes.len(),
        });
    }
    out
}

/// Extract the fragments of a diagram source whose first byte sits at `base` in the host file.
#[must_use]
pub fn extract(format: Format, source: &str, base: usize) -> Extraction {
    match format {
        Format::Mermaid => mermaid::extract(source, base),
        Format::PlantUml => plantuml::extract(source, base),
        Format::Excalidraw => excalidraw::extract(source, base),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_path_matches_extensions_case_insensitively() {
        for (path, want) in [
            ("flow.mmd", Some(Format::Mermaid)),
            ("dir/Flow.MMD", Some(Format::Mermaid)),
            ("x.mermaid", Some(Format::Mermaid)),
            ("mmd", None),
            (".mmd", None),
            ("dir.mmd/readme", None),
            ("flow.mmd.bak", None),
            ("seq.puml", Some(Format::PlantUml)),
            ("Seq.PlantUML", Some(Format::PlantUml)),
            ("board.excalidraw", Some(Format::Excalidraw)),
            ("notes.md", None),
            ("", None),
        ] {
            assert_eq!(Format::from_path(path), want, "{path:?}");
        }
    }

    #[test]
    fn from_lang_is_case_insensitive() {
        assert_eq!(Format::from_lang("Mermaid"), Some(Format::Mermaid));
        assert_eq!(Format::from_lang("rust"), None);
        assert_eq!(Format::from_lang("PlantUML"), Some(Format::PlantUml));
        assert_eq!(Format::from_lang(""), None);
    }

    #[test]
    fn lines_split_on_lf_crlf_and_lone_cr() {
        let got: Vec<_> = lines("a\nb\r\nc\rd")
            .iter()
            .map(|l| (l.start, l.text, l.end))
            .collect();
        assert_eq!(got, [(0, "a", 2), (2, "b", 5), (5, "c", 7), (7, "d", 8)]);
        assert_eq!(lines("").len(), 0);
        assert_eq!(lines("a\n").len(), 1);
    }
}
