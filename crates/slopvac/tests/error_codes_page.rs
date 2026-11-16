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
