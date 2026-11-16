//! Front-matter detection and parsing must not panic, and every range must index the document
//! at character boundaries.

#![no_main]

use std::ops::Range;

use frontmatter_spans::{FrontMatter, Node, Value};
use libfuzzer_sys::fuzz_target;

fn check_range(range: &Range<usize>, doc: &str) {
    assert!(range.start <= range.end, "range runs backwards: {range:?}");
    assert!(range.end <= doc.len(), "range past the end: {range:?}");
    assert!(
        doc.is_char_boundary(range.start),
        "start splits a character: {range:?}"
    );
    assert!(
        doc.is_char_boundary(range.end),
        "end splits a character: {range:?}"
    );
}

fn check(node: &Node, doc: &str) {
    check_range(&node.span.range, doc);
    match &node.value {
        Value::Array(items) => items.iter().for_each(|item| check(item, doc)),
        Value::Table(entries) => {
            for entry in entries {
                check_range(&entry.key_span.range, doc);
                check(&entry.value, doc);
            }
        }
        _ => {}
    }
}

fuzz_target!(|doc: &str| {
    if let Ok(Some(front)) = FrontMatter::parse(doc) {
        check_range(&front.block.outer, doc);
        check_range(&front.block.content, doc);
        assert!(front.block.body_start <= doc.len());
        assert!(doc.is_char_boundary(front.block.body_start));
        check(&front.root, doc);
    }
});
