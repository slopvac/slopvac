//! The core pack: the rules, styles, genres and taggers compiled into the binary.

use crate::source::{EmbeddedSource, FileTable};

/// Embed the files `<root><name>.<ext>` as `<dir>/<name>.<ext>`.
macro_rules! embed {
    ($dir:literal, $root:literal, $ext:literal: $($name:literal),+ $(,)?) => {
        &[$((
            concat!($dir, "/", $name, ".", $ext),
            include_str!(concat!($root, $name, ".", $ext)),
        )),+]
    };
}

// STUB(slopvac-8l4.12.27): the category files are the converted 2.x rules that sit in the
// `slopvac-rules` test fixtures. That bead moves them to their permanent home.
const CATEGORIES: FileTable = embed!(
    "categories", "../../slopvac-rules/tests/fixtures/core-v3/", "json":
    "ai-residue", "ai-tells-agentic", "ai-tells-content-shape", "ai-tells-figurative",
    "ai-tells-formatting", "ai-tells-register", "ai-tells-structure", "docs-discipline",
    "orwell", "prose-agency", "prose-craft", "prose-discipline", "prose-format",
    "prose-inclusive", "prose-inflation", "prose-promotion", "prose-scope", "ste-descriptive",
    "ste-nouns", "ste-practices", "ste-procedural", "ste-punctuation", "ste-safety",
    "ste-sentences", "ste-verbs", "ste-words",
);

const STYLES: FileTable = embed!(
    "styles", "../../slopvac-styles/presets/", "toml": "ste", "google",
);

/// The core pack as embedded in the binary. Genre and tagger files come from
/// `slopvac-context`, which registers the same files for its own built-ins.
#[must_use]
pub fn core_source() -> EmbeddedSource {
    EmbeddedSource::new(
        include_str!("../core/slopvac-plugin.toml"),
        &[
            CATEGORIES,
            STYLES,
            slopvac_context::GENRE_FILES,
            slopvac_context::TAGGER_FILES,
        ],
    )
}
