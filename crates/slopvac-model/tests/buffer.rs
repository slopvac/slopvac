#![allow(missing_docs)]

use serde_json::Value;
use slopvac_model::{ColumnUnit, SourceIndex};

fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../../schemas/v3/conformance/positions/vectors.json"
    ))
    .unwrap()
}

fn n(v: &Value) -> usize {
    usize::try_from(v.as_u64().unwrap()).unwrap()
}

/// LSP positions are 0-based and count the buffer text the server received.
#[test]
fn buffer_index_matches_lsp_range_vectors() {
    let v = vectors();
    let cases = v["lsp_ranges"].as_array().unwrap();
    assert_ne!(cases.len(), 0);
    for case in cases {
        let text = v["documents"][case["document"].as_str().unwrap()]["text"]
            .as_str()
            .unwrap();
        let idx = SourceIndex::new_buffer(text);
        for (name, unit) in [
            ("utf-8", ColumnUnit::Utf8),
            ("utf-16", ColumnUnit::Utf16),
            ("utf-32", ColumnUnit::CodePoint),
        ] {
            for end in ["start", "end"] {
                let got = idx.line_col_in(n(&case["span"][end]), unit).unwrap();
                let want = &case["expected"][name][end];
                assert_eq!(
                    (got.line - 1, got.column - 1),
                    (n(&want["line"]), n(&want["character"])),
                    "{} {name} {end}",
                    case["id"]
                );
            }
        }
    }
}

#[test]
fn buffer_index_counts_a_leading_bom_as_a_character() {
    let idx = SourceIndex::new_buffer("\u{feff}Hi");
    assert_eq!(idx.line_col_in(3, ColumnUnit::Utf16).unwrap().column, 2);
    assert_eq!(idx.line_col_in(3, ColumnUnit::Utf8).unwrap().column, 4);
}

#[test]
fn file_index_still_excludes_the_bom() {
    let idx = SourceIndex::new("\u{feff}Hi");
    assert_eq!(idx.line_col_in(3, ColumnUnit::Utf16).unwrap().column, 1);
    assert!(idx.check_offset(1).is_err());
}

#[test]
fn buffer_index_rejects_an_offset_inside_the_bom_character() {
    let idx = SourceIndex::new_buffer("\u{feff}Hi");
    assert!(idx.check_offset(0).is_ok());
    assert!(idx.check_offset(1).is_err());
}
