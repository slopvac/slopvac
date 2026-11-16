//! The shared position vectors (`schemas/v3/conformance/positions/vectors.json`) against `LineIndex`.

use std::fs;
use std::path::Path;

use serde_json::Value;
use slopvac_config::tree::{LineIndex, parse_toml};

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn vectors() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/v3/conformance/positions/vectors.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn every_position_vector_gives_the_listed_line_and_code_point_column() {
    let v = vectors();
    let docs = v["documents"].as_object().unwrap();
    let positions = v["positions"].as_array().unwrap();
    assert!(positions.len() > 30, "the corpus is not empty");
    for p in positions {
        let name = p["document"].as_str().unwrap();
        let text = docs[name]["text"].as_str().unwrap();
        let offset = usize::try_from(p["offset"].as_u64().unwrap()).unwrap();
        let want = (
            usize::try_from(p["line"].as_u64().unwrap()).unwrap(),
            usize::try_from(p["utf32"].as_u64().unwrap()).unwrap(),
        );
        assert_eq!(
            LineIndex::new(text).line_col(offset),
            want,
            "{name} at offset {offset}"
        );
    }
}

#[test]
fn a_toml_file_with_a_byte_order_mark_keeps_positions_after_the_mark() {
    let text = "\u{feff}locale = \"en-GB\"\n";
    let root = parse_toml(text).unwrap();
    let locale = root.entry("locale").unwrap();
    assert_eq!(&text[locale.key_span.clone()], "locale");
    assert_eq!(LineIndex::new(text).line_col(locale.key_span.start), (1, 1));
}

#[test]
fn a_lone_cr_ends_a_line_and_crlf_ends_one() {
    let idx = LineIndex::new("a\rb\r\nc");
    assert_eq!(idx.line_col(2), (2, 1));
    assert_eq!(idx.line_col(5), (3, 1));
}
