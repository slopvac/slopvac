//! Front-matter detection and parsing must not panic, and every span must index the document.

#![no_main]

use frontmatter_spans::{FrontMatter, Node};
use libfuzzer_sys::fuzz_target;

fn check(node: &Node, doc: &str) {
    let range = &node.span.range;
    assert!(range.start <= range.end, "span runs backwards");
    assert!(range.end <= doc.len(), "span past the end of the document");
}

fuzz_target!(|doc: &str| {
    if let Ok(Some(front)) = FrontMatter::parse(doc) {
        check(&front.root, doc);
        assert!(front.block.content.end <= doc.len());
    }
});
