//! The RST adapter against a real docutils.
//!
//! These tests need a `python3` or `python` on `PATH` that imports docutils 0.21 or later,
//! and return early without one. The always-on tests are in `rst_fake.rs`.
#![allow(
    missing_docs,
    clippy::assert_is_empty,
    clippy::cloned_ref_to_slice_refs
)]

use std::fs;

use slopvac_engine::rst::{RstAdapter, RstOptions, RstParse};
use slopvac_model::Span;

/// An adapter over the interpreter on `PATH`, or `None` after a note that the test skipped.
fn installed() -> Option<RstAdapter> {
    let adapter = RstAdapter::new(RstOptions::default());
    if let Err(error) = adapter.interpreter() {
        eprintln!("skipped: {error}");
        return None;
    }
    Some(adapter)
}

fn slice(source: &str, span: Span) -> &str {
    &source[span.start()..span.end()]
}

fn texts(parsed: &RstParse) -> Vec<String> {
    parsed
        .document
        .blocks
        .iter()
        .filter_map(|b| b.projection.as_ref().map(|p| p.text().to_owned()))
        .collect()
}

#[test]
fn docutils_output_maps_back_to_the_source() {
    let Some(adapter) = installed() else {
        return;
    };
    let source = "Title\n=====\n\nSome *emphasis*, **strong** words and ``code``.\n";
    let parsed = adapter.parse(source).unwrap();
    let blocks: Vec<_> = parsed
        .document
        .blocks
        .iter()
        .map(|b| slice(source, b.span))
        .collect();
    assert_eq!(
        blocks,
        ["Title", "Some *emphasis*, **strong** words and ``code``."]
    );
    assert_eq!(
        texts(&parsed),
        ["Title", "Some emphasis, strong words and \0."]
    );
    let spans: Vec<_> = parsed
        .document
        .code_spans
        .iter()
        .map(|&s| slice(source, s))
        .collect();
    assert_eq!(spans, ["``code``"]);
    assert!(parsed.notices.is_empty(), "{:?}", parsed.notices);
}

#[test]
fn include_and_raw_directives_read_nothing() {
    let Some(adapter) = installed() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let secret = dir.path().join("secret.txt");
    fs::write(&secret, "INCLUDESENTINEL\n").unwrap();
    let source = format!(
        "Title\n=====\n\nBefore.\n\n.. include:: {}\n\n.. raw:: html\n\n   <p>RAWSENTINEL</p>\n\nAfter.\n",
        secret.display()
    );
    let parsed = adapter.parse(&source).unwrap();
    assert_eq!(texts(&parsed), ["Title", "Before.", "After."]);
}

#[test]
fn a_comment_that_repeats_the_next_paragraph_does_not_take_its_position() {
    let Some(adapter) = installed() else {
        return;
    };
    let source = "Intro text.\n\n.. Repeated paragraph words.\n\nRepeated paragraph words.\n";
    let parsed = adapter.parse(source).unwrap();
    let last = &parsed.document.blocks[1];
    assert_eq!(last.span.start(), source.rfind("Repeated").unwrap());
}
