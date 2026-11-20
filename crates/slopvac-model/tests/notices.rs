//! Catalog-backed notice construction.

use slopvac_messages::Catalog;
use slopvac_model::{Notice, NoticeLevel};

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
    let notice = Notice::from_catalog(level, &Message::Checked { count: 2 });
    assert_eq!(notice.code.as_str(), "documents-checked");
    assert_eq!(notice.message.as_str(), "checked 2 documents");
    assert_eq!(notice.level, level);
    assert_eq!(notice.path, None);
    assert!(notice.data.is_empty());
}

#[test]
fn an_empty_rendering_uses_the_compiled_summary() {
    let notice = Notice::from_catalog(
        NoticeLevel::Info,
        &Message::Detail {
            detail: String::new(),
        },
    );
    assert_eq!(
        notice.message.as_str(),
        "An empty detail still has a nonempty summary."
    );
}
