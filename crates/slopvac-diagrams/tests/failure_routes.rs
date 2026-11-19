//! Inputs that cannot be read at all: the status and the empty result they report.

use std::ops::Range;

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
