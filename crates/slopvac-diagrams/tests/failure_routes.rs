//! Inputs that cannot be read at all: the status and the empty result they report.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use std::io::Write as _;
use std::ops::Range;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use flate2::Compression;
use flate2::write::DeflateEncoder;
use rstest::rstest;
use slopvac_diagrams::{
    Extraction, ExtractionStatus, Format, extract, extract_bytes, extract_fenced,
};

fn assert_read_nothing(ex: &Extraction, status: ExtractionStatus) {
    assert_eq!(ex.status, status);
    assert_eq!(ex.fragments, []);
    assert_eq!(ex.diagram, None);
}

#[test]
fn a_png_given_to_the_text_path_is_unsupported() {
    let ex = extract(Format::DrawioPng, "\u{89}PNG", 0);
    assert_read_nothing(
        &ex,
        ExtractionStatus::Unsupported("binary format needs byte input"),
    );
}

#[rstest]
#[case::mermaid(Format::Mermaid)]
#[case::plantuml(Format::PlantUml)]
#[case::excalidraw(Format::Excalidraw)]
#[case::drawio(Format::Drawio)]
#[case::drawio_svg(Format::DrawioSvg)]
fn bytes_that_are_not_utf8_are_malformed_for_a_text_format(#[case] format: Format) {
    let ex = extract_bytes(format, b"A -> B : \xff\xfe", 0);
    assert_read_nothing(&ex, ExtractionStatus::Malformed("input is not valid UTF-8"));
}

#[rstest]
#[case::past_the_end(10..20)]
#[case::reversed(Range { start: 3, end: 1 })]
#[case::inside_a_character(1..2)]
fn a_fence_range_outside_the_source_is_malformed(#[case] body: Range<usize>) {
    let ex = extract_fenced(Format::Mermaid, "é\nflowchart TD\n", body);
    assert_read_nothing(
        &ex,
        ExtractionStatus::Malformed("block range is outside the source"),
    );
}

/// A draw.io file with one compressed page holding `payload` after base64.
fn drawio_page(payload: &[u8]) -> String {
    format!(
        "<mxfile><diagram name=\"A\">{}</diagram></mxfile>",
        STANDARD.encode(payload)
    )
}

fn deflate(text: &[u8]) -> Vec<u8> {
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::fast());
    enc.write_all(text).unwrap();
    enc.finish().unwrap()
}

#[test]
fn a_drawio_page_of_valid_base64_but_corrupt_deflate_is_malformed() {
    // 0x07 sets the final-block bit and the reserved block type 3.
    let ex = extract(Format::Drawio, &drawio_page(&[0x07, 0xff, 0xff]), 0);
    assert_eq!(
        ex.status,
        ExtractionStatus::Malformed("invalid compressed page")
    );
    assert_eq!(ex.fragments, []);
}

#[rstest]
#[case::bad_hex(b"%ZZ".as_slice())]
#[case::truncated(b"abc%4".as_slice())]
fn a_drawio_page_with_invalid_percent_encoding_is_malformed(#[case] inflated: &[u8]) {
    let ex = extract(Format::Drawio, &drawio_page(&deflate(inflated)), 0);
    assert_eq!(
        ex.status,
        ExtractionStatus::Malformed("invalid percent encoding")
    );
    assert_eq!(ex.fragments, []);
}
