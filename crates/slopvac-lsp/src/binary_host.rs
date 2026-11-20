//! Binary container documents, which the editor does not lint.

use slopvac_messages::{Catalog, render};
use slopvac_model::{KebabName, NonEmptyString, Notice, NoticeLevel};

use crate::messages::LspMessage;

/// Extensions of binary containers, matched case-insensitively at the end of the path:
/// PNG with embedded source, OOXML and ODF documents and their templates.
// TODO(8l4.20.1): replace with the shared BinaryFormat detection once it exists.
const BINARY_SUFFIXES: &[&str] = &[
    ".drawio.png",
    ".excalidraw.png",
    ".docx",
    ".docm",
    ".dotx",
    ".dotm",
    ".pptx",
    ".pptm",
    ".potx",
    ".potm",
    ".ppsx",
    ".ppsm",
    ".xlsx",
    ".xlsm",
    ".xltx",
    ".xltm",
    ".odt",
    ".ott",
    ".odp",
    ".otp",
    ".ods",
    ".ots",
];

/// Whether `path` names a binary container.
pub(crate) fn is_binary_host(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    BINARY_SUFFIXES.iter().any(|s| lower.ends_with(s))
}

/// The one notice for a binary container. The `-unchecked` suffix follows the other
/// notices that mark a document as not fully checked.
pub(crate) fn unchecked_notice() -> Option<Notice> {
    Some(Notice {
        code: KebabName::new(LspMessage::BinaryHostUnchecked.code()).ok()?,
        level: NoticeLevel::Warning,
        message: NonEmptyString::new(render(&LspMessage::BinaryHostUnchecked)).ok()?,
        path: None,
        data: serde_json::Map::new(),
    })
}
