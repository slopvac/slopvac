//! Integration tests for expressions.
//! The MDX expression checker must never reject what markdown-rs accepts with plain
//! brace counting. These properties compare `parse_mdx` with markdown-rs and no hook.

use markdown::{Constructs, ParseOptions, to_mdast};
use proptest::prelude::*;
use slopvac_model::Span;
use slopvac_parse::parse_mdx;

/// Expression pieces: JSX, regexes, divisions, strings, templates, comments and braces,
/// including odd combinations.
fn piece() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("<p>text</p>".to_owned()),
        Just("<p>don't</p>".to_owned()),
        Just("<br/>".to_owned()),
        Just("<b>{x}</b>".to_owned()),
        Just("</i>".to_owned()),
        Just("<".to_owned()),
        Just("a < b".to_owned()),
        Just("/a+/g".to_owned()),
        Just("/[/']/".to_owned()),
        Just("/}/".to_owned()),
        Just("/\\\\/'/".to_owned()),
        Just("n / 2".to_owned()),
        Just("(n) / 2".to_owned()),
        Just("x++ / 2".to_owned()),
        Just("'a'".to_owned()),
        Just("\"b\"".to_owned()),
        Just("'{'".to_owned()),
        Just("\"}\"".to_owned()),
        Just("'it\\\\'s'".to_owned()),
        Just("`t ${x} u`".to_owned()),
        Just("`{`".to_owned()),
        Just("/* c */".to_owned()),
        Just("/* } */".to_owned()),
        Just("/* it's */".to_owned()),
        Just("// c\n".to_owned()),
        Just("// it's }\n".to_owned()),
        Just("{ a: 1 }".to_owned()),
        Just("{".to_owned()),
        Just("}".to_owned()),
        Just("'".to_owned()),
        Just("\"".to_owned()),
        Just("`".to_owned()),
        Just("/".to_owned()),
        Just("&&".to_owned()),
        Just("x".to_owned()),
        Just("return".to_owned()),
        Just("\n".to_owned()),
    ]
}

fn document() -> impl Strategy<Value = String> {
    prop::collection::vec(piece(), 0..8)
        .prop_map(|pieces| format!("a {{{}}} b\n", pieces.join(" ")))
}

fn naive(source: &str) -> bool {
    let options = ParseOptions {
        constructs: Constructs {
            gfm_table: true,
            gfm_strikethrough: true,
            gfm_task_list_item: true,
            ..Constructs::mdx()
        },
        ..ParseOptions::mdx()
    };
    to_mdast(source, &options).is_ok()
}

proptest! {
    #[test]
    fn the_hook_never_rejects_what_plain_counting_accepts(source in document()) {
        if naive(&source) {
            let body = Span::new(0, source.len()).unwrap();
            prop_assert!(parse_mdx(&source, body).is_ok(), "{source:?}");
        }
    }
}
