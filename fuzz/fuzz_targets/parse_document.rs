//! The Markdown, MDX and HTML parsers must not panic on any input or body range.
//!
//! The first byte picks the syntax and a body range; the rest is the document.

#![no_main]

use libfuzzer_sys::fuzz_target;
use slopvac_model::Span;
use slopvac_parse::{mdx_syntax_spans, parse_html, parse_markdown, parse_mdx};

fuzz_target!(|data: &[u8]| {
    let Some((&selector, rest)) = data.split_first() else {
        return;
    };
    let Ok(source) = std::str::from_utf8(rest) else {
        return;
    };
    // Mostly the whole document, sometimes a sub-range that may split a character.
    let start = if selector & 0b1000 == 0 {
        0
    } else {
        usize::from(selector >> 4) % (source.len() + 1)
    };
    let Ok(body) = Span::new(start, source.len()) else {
        return;
    };
    match selector & 0b11 {
        0 => drop(parse_markdown(source, body)),
        1 => drop(parse_mdx(source, body)),
        2 => drop(mdx_syntax_spans(source, body)),
        _ => drop(parse_html(source, body)),
    }
});
