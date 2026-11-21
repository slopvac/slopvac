//! Catalog-backed notice construction.

use slopvac_messages::{Arg, Catalog, Entry, Kind};
use slopvac_model::{Notice, NoticeError, NoticeLevel, NoticeMessage};

#[derive(Debug, Catalog)]
enum Message {
    /// Documents checked by the test catalog.
    #[code("documents-checked")]
    #[notice("checked {count} documents")]
    Checked {
        #[count]
        count: usize,
    },
    /// An empty detail still has a nonempty summary.
    #[code("empty-detail")]
    #[notice("{detail}")]
    Detail { detail: String },
}

#[rstest::rstest]
#[case::info(NoticeLevel::Info)]
#[case::warning(NoticeLevel::Warning)]
fn constructs_a_notice_from_the_compiled_code_and_named_parameters(#[case] level: NoticeLevel) {
    let message = NoticeMessage::try_from_catalog(&Message::Checked { count: 2 }).unwrap();
    let notice = Notice::from_catalog(level, message);
    assert_eq!(notice.code.as_str(), "documents-checked");
    assert_eq!(notice.message.as_str(), "checked 2 documents");
    assert_eq!(notice.level, level);
    assert_eq!(notice.path, None);
    assert!(notice.data.is_empty());
}

#[test]
fn an_empty_rendering_uses_the_compiled_summary() {
    let notice = Notice::try_from_catalog(
        NoticeLevel::Info,
        &Message::Detail {
            detail: String::new(),
        },
    )
    .unwrap();
    assert_eq!(
        notice.message.as_str(),
        "An empty detail still has a nonempty summary."
    );
}

enum Handwritten {
    InvalidCode,
    EmptyMessage,
}

static HANDWRITTEN_ENTRIES: [Entry; 2] = [
    Entry::new(
        "manual-good",
        Kind::Notice,
        "Handwritten::InvalidCode",
        "message",
        &[],
        "A nonempty message.",
    ),
    Entry::new(
        "manual-empty",
        Kind::Notice,
        "Handwritten::EmptyMessage",
        "",
        &[],
        "",
    ),
];

impl Catalog for Handwritten {
    fn entries() -> &'static [Entry] {
        &HANDWRITTEN_ENTRIES
    }

    fn entry(&self) -> &'static Entry {
        match self {
            Self::InvalidCode => &HANDWRITTEN_ENTRIES[0],
            Self::EmptyMessage => &HANDWRITTEN_ENTRIES[1],
        }
    }

    fn args(&self) -> Vec<Arg> {
        Vec::new()
    }

    fn code(&self) -> &'static str {
        match self {
            Self::InvalidCode => "INVALID",
            Self::EmptyMessage => self.entry().code,
        }
    }
}

#[test]
fn rejects_a_handwritten_catalog_with_an_invalid_code_override() {
    let result = Notice::try_from_catalog(NoticeLevel::Info, &Handwritten::InvalidCode);
    assert!(matches!(result, Err(NoticeError::InvalidCode(_))));
}

#[test]
fn rejects_a_handwritten_catalog_with_empty_rendering_and_summary() {
    let result = Notice::try_from_catalog(NoticeLevel::Info, &Handwritten::EmptyMessage);
    assert!(matches!(result, Err(NoticeError::EmptyMessage(_))));
}
