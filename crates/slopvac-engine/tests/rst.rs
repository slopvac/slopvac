//! The RST adapter against a real docutils.
//!
//! These tests need `rst2html` or `rst2html.py` on `PATH` (`pip install docutils`) and
//! return early without it. The always-on tests are in `rst_fake.rs`.

use std::fs;
use std::path::PathBuf;

use slopvac_engine::rst::{RstError, RstOptions, RstParse, find_converter, parse_rst};
use slopvac_model::Span;

/// The converter on `PATH`, or `None` after a note that the test was skipped.
fn installed() -> Option<PathBuf> {
    let found = find_converter(None);
    if found.is_none() {
        eprintln!("skipped: rst2html is not on PATH");
    }
    found
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
fn an_empty_search_path_has_no_converter() {
    let empty = tempfile::tempdir().unwrap();
    let mut options = RstOptions::default();
    options.search_path = Some(empty.path().as_os_str().to_owned());
    let error = parse_rst("Title\n=====\n", &options).unwrap_err();
    assert!(matches!(error, RstError::ConverterMissing), "{error}");
}

#[test]
fn docutils_output_maps_back_to_the_source() {
    let Some(converter) = installed() else {
        return;
    };
    let source = "Title\n=====\n\nSome *emphasis*, **strong** words and ``code``.\n";
    let parsed = parse_rst(source, &RstOptions::default()).unwrap();
    assert_eq!(parsed.converter, converter);
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
}

#[test]
fn include_and_raw_directives_read_nothing() {
    if installed().is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let secret = dir.path().join("secret.txt");
    fs::write(&secret, "INCLUDESENTINEL\n").unwrap();
    let source = format!(
        "Title\n=====\n\nBefore.\n\n.. include:: {}\n\n.. raw:: html\n\n   <p>RAWSENTINEL</p>\n\nAfter.\n",
        secret.display()
    );
    let parsed = parse_rst(&source, &RstOptions::default()).unwrap();
    let all = texts(&parsed);
    assert_eq!(all, ["Title", "Before.", "After."]);
}
