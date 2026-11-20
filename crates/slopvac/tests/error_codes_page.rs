//! The generated error-code page lists each code with its kind, message and parameters,
//! and keeps retired codes.

#[path = "../examples/reference.rs"]
#[allow(dead_code)]
mod reference;

use slopvac_cli::messages::Retired;
use slopvac_messages::{Entry, Kind, Param, ParamKind};

const PARAMS: &[Param] = &[
    Param::new("path", ParamKind::Path),
    Param::new("n", ParamKind::Count),
];

#[test]
fn a_registered_code_gets_one_table_row() {
    let entries = [Entry::new(
        "file-unreadable",
        Kind::Error,
        "E::V",
        "cannot read {path} ({n} tries)",
        PARAMS,
        "The file could not | be read.",
    )];
    let page = reference::page(&[("reader", &entries)], &[]);
    let expected = "## reader\n\n\
        | Code | Kind | Message | Parameters | Summary |\n\
        | --- | --- | --- | --- | --- |\n\
        | `file-unreadable` | error | `cannot read {path} ({n} tries)` | path (path), n (count) | The file could not \\| be read. |\n";
    assert!(page.contains(expected), "{page}");
}

#[test]
fn a_text_entry_is_left_off_the_page_while_error_and_notice_entries_stay() {
    let entries = [
        Entry::new("a-error", Kind::Error, "E::A", "failed", &[], "Failure."),
        Entry::new("b-label", Kind::Text, "E::B", "Label", &[], "A label."),
        Entry::new("c-notice", Kind::Notice, "E::C", "heads up", &[], "A note."),
    ];
    let page = reference::page(&[("mixed", &entries)], &[]);
    let expected = "## mixed\n\n\
        | Code | Kind | Message | Parameters | Summary |\n\
        | --- | --- | --- | --- | --- |\n\
        | `a-error` | error | `failed` | none | Failure. |\n\
        | `c-notice` | notice | `heads up` | none | A note. |\n";
    assert!(page.contains(expected), "{page}");
    assert!(!page.contains("b-label"), "{page}");
}

#[test]
fn a_catalog_of_only_text_entries_gets_no_table() {
    let entries = [Entry::new(
        "b-label",
        Kind::Text,
        "E::B",
        "Label",
        &[],
        "A label.",
    )];
    let page = reference::page(&[("labels", &entries)], &[]);
    assert!(!page.contains("## labels"), "{page}");
    assert!(page.contains("No codes are registered yet.\n"), "{page}");
}

#[test]
fn retired_codes_stay_listed_with_their_note() {
    let retired = [Retired {
        code: "old-code",
        note: "Replaced by new-code.",
    }];
    let page = reference::page(&[], &retired);
    assert!(page.contains("## Retired codes"), "{page}");
    assert!(
        page.contains("| `old-code` | Replaced by new-code. |\n"),
        "{page}"
    );
}

#[test]
fn an_empty_registry_says_so() {
    assert!(reference::page(&[], &[]).contains("No codes are registered yet.\n"));
}
