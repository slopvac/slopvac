//! Extract label text from diagram sources.
//!
//! Experimental. Each extractor turns a diagram into [`Fragment`]s: short runs of
//! label text with the byte range they came from. The rules lint a fragment as a
//! label, not as a sentence. The shape mirrors the `Segment` of the plugin
//! protocol (`span`, `context`, `text`) so the same extractors can sit behind
//! `extractor/extract` later.
//!
//! # Input size
//!
//! The crate does not cap its input. The caller must pass at most
//! 64 MiB (`slopvac_fs::MAX_DOCUMENT_BYTES`) of text, and each slopvac entry point enforces
//! that cap before it calls in. The caps are listed in `docs/contract/policies.md`.
//!
//! # Limits
//!
//! A logical location repeats an identifier in every fragment, so the crate bounds what
//! one extraction retains:
//!
//! - An identifier or page name is cut to [`MAX_IDENT_CHARS`] characters (128) in a logical
//!   location, on a character boundary, and ends in `…` when it was cut.
//! - An extraction keeps at most [`MAX_FRAGMENTS`] fragments (100,000) and
//!   [`MAX_RETAINED_BYTES`] bytes (16 MiB) of `logical` plus `text` summed over its
//!   fragments. Past either cap, the extractor stops reading and the status is
//!   [`ExtractionStatus::BudgetExhausted`], which names the cap in a [`BudgetHit`].
//!
//! The retained bytes of one extraction therefore grow linearly with its input. draw.io
//! has its own limits on inflated bytes (32 MiB) and XML nodes (1,000,000) per file.

mod drawio;
mod excalidraw;
mod fences;
mod json;
mod mermaid;
mod plantuml;
mod signals;
mod sink;

pub use fences::{DiagramBlock, extract_fenced, find_markdown_blocks, find_mdx_blocks};
#[cfg(feature = "test-counters")]
#[doc(hidden)]
pub use signals::prose_analyses;
pub use signals::{
    PACK, ProseAnalysis, ProseBlock, Signal, SignalHit, check_labels, check_prose,
    check_prose_analysed,
};
pub use sink::{MAX_FRAGMENTS, MAX_IDENT_CHARS, MAX_RETAINED_BYTES};

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

/// The limit that stopped an extraction.
///
/// Both fields are fixed values, never source text, so a caller may show them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct BudgetHit {
    /// The cap name: `fragments`, `retained-bytes`, `inflated-bytes` or `xml-nodes`.
    pub cap: &'static str,
    /// The cap value: a count for `fragments` and `xml-nodes`, bytes for the others.
    pub limit: u64,
}

impl BudgetHit {
    pub(crate) const fn new(cap: &'static str, limit: u64) -> Self {
        BudgetHit { cap, limit }
    }
}

/// Whether an extraction read the whole diagram.
///
/// A reason is a fixed phrase. It never holds source text, so callers may show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ExtractionStatus {
    /// The diagram was read in full. It may hold no labels.
    #[default]
    Ok,
    /// The diagram type or container is not one this crate reads.
    Unsupported(&'static str),
    /// The input is not a well-formed diagram of its format.
    Malformed(&'static str),
    /// A size or work limit stopped the read.
    BudgetExhausted(BudgetHit),
}

impl ExtractionStatus {
    /// Whether the extraction cannot establish that the diagram is clean.
    #[must_use]
    pub const fn is_failure(self) -> bool {
        !matches!(self, ExtractionStatus::Ok)
    }

    /// The fixed reason of a failure, or `None` for [`ExtractionStatus::Ok`].
    ///
    /// For [`ExtractionStatus::BudgetExhausted`] it is a general phrase. The
    /// [`BudgetHit`] names the cap and its limit.
    #[must_use]
    pub const fn reason(self) -> Option<&'static str> {
        match self {
            ExtractionStatus::Ok => None,
            ExtractionStatus::Unsupported(r) | ExtractionStatus::Malformed(r) => Some(r),
            ExtractionStatus::BudgetExhausted(_) => Some("a size or work limit was reached"),
        }
    }
}

/// The result of extracting one diagram.
///
/// When `status` is a failure, `fragments` holds what was read before the failure and
/// the diagram is not fully checked.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extraction {
    /// The diagram type, such as `flowchart`. `None` when it is unsupported.
    pub diagram: Option<String>,
    /// Fragments in source order, up to the limits in the crate docs.
    pub fragments: Vec<Fragment>,
    /// Whether the diagram was read in full.
    pub status: ExtractionStatus,
}

impl Extraction {
    /// An extraction that read nothing and failed with `status`.
    pub(crate) fn failed(status: ExtractionStatus) -> Self {
        Extraction {
            status,
            ..Extraction::default()
        }
    }
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
    /// A draw.io file (`.drawio`, `.dio`), plain or compressed.
    Drawio,
    /// A `.drawio.svg` file that embeds the diagram in its `content` attribute.
    DrawioSvg,
    /// A `.drawio.png` file that embeds the diagram in a text chunk. Binary input.
    DrawioPng,
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
        let name = std::path::Path::new(path)
            .file_name()?
            .to_str()?
            .to_ascii_lowercase();
        if name.ends_with(".drawio.svg") {
            return Some(Format::DrawioSvg);
        }
        if name.ends_with(".drawio.png") {
            return Some(Format::DrawioPng);
        }
        let ext = std::path::Path::new(path).extension()?.to_str()?;
        match ext.to_ascii_lowercase().as_str() {
            "drawio" | "dio" => Some(Format::Drawio),
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
        });
        pos += terminator;
        start = pos;
    }
    if start < bytes.len() {
        out.push(SourceLine {
            start,
            text: &src[start..],
        });
    }
    out
}

/// Extract the fragments of a text diagram source whose first byte sits at `base` in the host file.
///
/// A binary format (`DrawioPng`) is unsupported here; use [`extract_bytes`].
#[must_use]
pub fn extract(format: Format, source: &str, base: usize) -> Extraction {
    match format {
        Format::Mermaid => mermaid::extract(source, base),
        Format::PlantUml => plantuml::extract(source, base),
        Format::Excalidraw => excalidraw::extract(source, base),
        Format::Drawio => drawio::extract_xml(source, base),
        Format::DrawioSvg => drawio::extract_svg(source, base),
        Format::DrawioPng => Extraction::failed(ExtractionStatus::Unsupported(
            "binary format needs byte input",
        )),
    }
}

/// Extract the fragments of a file given as bytes. Text formats must be UTF-8.
#[must_use]
pub fn extract_bytes(format: Format, bytes: &[u8], base: usize) -> Extraction {
    if format == Format::DrawioPng {
        return drawio::extract_png(bytes, base);
    }
    std::str::from_utf8(bytes).map_or_else(
        |_| Extraction::failed(ExtractionStatus::Malformed("input is not valid UTF-8")),
        |s| extract(format, s, base),
    )
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
            ("a.drawio", Some(Format::Drawio)),
            ("a.dio", Some(Format::Drawio)),
            ("a.drawio.svg", Some(Format::DrawioSvg)),
            ("a.DRAWIO.png", Some(Format::DrawioPng)),
            ("plain.svg", None),
            ("plain.png", None),
            ("dir.drawio.svg/readme", None),
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
            .map(|l| (l.start, l.text))
            .collect();
        assert_eq!(got, [(0, "a"), (2, "b"), (5, "c"), (7, "d")]);
        assert_eq!(lines("").len(), 0);
        assert_eq!(lines("a\n").len(), 1);
    }
}
