#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]
#![allow(missing_docs, reason = "test crate: nothing public to document")]
//! MDX syntax spans: which source ranges are code, not prose.

use slopvac_model::Span;
use slopvac_parse::{mdx_syntax_spans, mdx_syntax_spans_counted};

fn whole(source: &str) -> Span {
    Span::new(0, source.len()).unwrap()
}

fn covered(source: &str, spans: &[Span], needle: &str) -> bool {
    let at = source.find(needle).unwrap();
    spans
        .iter()
        .any(|s| s.start() <= at && at + needle.len() <= s.end())
}

#[test]
fn a_template_literal_in_an_attribute_does_not_end_the_tag_early() {
    for source in [
        "<A b={`}>`} c=\"delta\" />\n",
        "<A b={/* }> */ 1} c=\"delta\" />\n",
        "<A b={1 // }>\n} c=\"delta\" />\n",
        "<A b={'}>'} c=\"delta\" />\n",
    ] {
        let spans = mdx_syntax_spans(source, whole(source)).unwrap();
        assert!(covered(source, &spans, "c=\"delta\""), "{source:?}");
    }
}

#[test]
fn repeated_elements_with_braces_in_templates_cost_linear_work() {
    let element = "<A b={`{`} />\n\n";
    let work = |n: usize| {
        let source = element.repeat(n);
        mdx_syntax_spans_counted(&source, whole(&source)).unwrap().1
    };
    let (small, large) = (work(50), work(100));
    assert!(
        large <= small * 2 + 64,
        "work grew from {small} to {large} when the input doubled"
    );
}
