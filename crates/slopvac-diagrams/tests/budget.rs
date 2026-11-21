//! The caps on one extraction: what hits them, what they report and how fast they stop.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use std::fmt::Write as _;
use std::io::Write as _;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use flate2::Compression;
use flate2::write::DeflateEncoder;
use slopvac_diagrams::{
    Extraction, ExtractionStatus, Format, MAX_FRAGMENTS, MAX_RETAINED_BYTES, extract,
};

const MIB: usize = 1024 * 1024;

fn hit(ex: &Extraction) -> (&'static str, u64) {
    match ex.status {
        ExtractionStatus::BudgetExhausted(hit) => (hit.cap, hit.limit),
        other => panic!("expected a budget status, got {other:?}"),
    }
}

/// An Excalidraw scene of `count` text elements. Each text is `chars` newlines written as
/// the two-byte escape `\n`, so the converted text is half the size of the source text.
fn escaped_scene(count: usize, chars: usize) -> String {
    let text = "\\n".repeat(chars);
    let mut src = String::from("{\"elements\":[");
    for i in 0..count {
        if i > 0 {
            src.push(',');
        }
        write!(
            src,
            "{{\"id\":\"e{i}\",\"type\":\"text\",\"text\":\"{text}\"}}"
        )
        .unwrap();
    }
    src.push_str("]}");
    src
}

#[test]
fn converted_text_counts_toward_the_retained_byte_budget() {
    // 16 texts of 1 MiB converted bytes each. Fragment cost is the logical name plus the
    // text: `excalidraw/text[e0]` is 19 bytes (20 from e10), so e0..e9 cost 1_048_595 and
    // e10.. cost 1_048_596. Fifteen fragments total 15_728_930, within 16_777_216. The
    // sixteenth would reach 16_777_526, so it is dropped.
    let ex = extract(Format::Excalidraw, &escaped_scene(16, MIB), 0);
    assert_eq!(ex.fragments.len(), 15);
    let retained: usize = ex
        .fragments
        .iter()
        .map(|f| f.logical.len() + f.text.as_ref().map_or(0, String::len))
        .sum();
    assert_eq!(retained, 15_728_930);
    assert!(retained <= MAX_RETAINED_BYTES);
    assert_eq!(hit(&ex), ("retained-bytes", 16_777_216));
}

#[test]
fn a_malformed_element_after_the_budget_is_spent_keeps_the_budget_status() {
    // Sixteen texts overrun the byte budget at the sixteenth. The non-object entry after
    // them is never read, so the status stays the budget status and the fifteen kept
    // fragments survive.
    let mut src = escaped_scene(16, MIB);
    src.truncate(src.len() - 2);
    src.push_str(",42]}");
    let ex = extract(Format::Excalidraw, &src, 0);
    assert_eq!(ex.fragments.len(), 15);
    assert_eq!(hit(&ex), ("retained-bytes", 16_777_216));
}

#[test]
fn the_fragment_cap_reports_its_name_and_limit() {
    let mut src = String::from("flowchart TD\n");
    for i in 0..MAX_FRAGMENTS + 5 {
        writeln!(src, "  A{i}[L{i}]").unwrap();
    }
    let ex = extract(Format::Mermaid, &src, 0);
    assert_eq!(ex.fragments.len(), MAX_FRAGMENTS);
    assert_eq!(hit(&ex), ("fragments", 100_000));
}

#[test]
fn the_xml_node_cap_reports_its_name_and_limit() {
    let src = format!(
        "<mxGraphModel><root>{}</root></mxGraphModel>",
        "<mxCell/>".repeat(1_000_000)
    );
    let ex = extract(Format::Drawio, &src, 0);
    assert_eq!(hit(&ex), ("xml-nodes", 1_000_000));
}

#[test]
fn the_inflated_byte_cap_reports_its_name_and_limit() {
    // 33 MiB of spaces deflates to a few dozen KiB and inflates past the 32 MiB cap.
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::fast());
    enc.write_all(&vec![b' '; 33 * MIB]).unwrap();
    let page = STANDARD.encode(enc.finish().unwrap());
    let src = format!("<mxfile><diagram name=\"Z\">{page}</diagram></mxfile>");
    let ex = extract(Format::Drawio, &src, 0);
    assert_eq!(hit(&ex), ("inflated-bytes", 33_554_432));
}

/// A class with a 4 MiB name and more members than the fragment cap. Copying the name into
/// each member would cost about 400 GiB, so a run that finishes quickly shows that the name
/// is clipped once and that reading stops at the cap.
const HUGE_ID: usize = 4 * MIB;
const SLOW: Duration = Duration::from_secs(10);

fn assert_stops_quickly(format: Format, src: &str) {
    let start = Instant::now();
    let ex = extract(format, src, 0);
    let took = start.elapsed();
    assert!(took < SLOW, "took {took:?}");
    assert!(matches!(ex.status, ExtractionStatus::BudgetExhausted(_)));
    assert!(ex.fragments.len() <= MAX_FRAGMENTS);
    assert!(ex.fragments.len() > 1_000, "a cap before any real work");
}

#[test]
fn a_huge_mermaid_class_name_does_not_cost_its_length_per_member() {
    let mut src = format!("classDiagram\n  class {} {{\n", "C".repeat(HUGE_ID));
    for _ in 0..MAX_FRAGMENTS + 10 {
        src.push_str("    +int m\n");
    }
    assert_stops_quickly(Format::Mermaid, &src);
}

#[test]
fn a_huge_mermaid_entity_name_does_not_cost_its_length_per_attribute() {
    let mut src = format!("erDiagram\n  {} {{\n", "E".repeat(HUGE_ID));
    for _ in 0..MAX_FRAGMENTS + 10 {
        src.push_str("    string \"a\"\n");
    }
    assert_stops_quickly(Format::Mermaid, &src);
}

#[test]
fn a_huge_plantuml_class_name_does_not_cost_its_length_per_member() {
    let mut src = format!("@startuml\nclass {} {{\n", "C".repeat(HUGE_ID));
    for _ in 0..MAX_FRAGMENTS + 10 {
        src.push_str("  +int m\n");
    }
    src.push_str("}\n@enduml\n");
    assert_stops_quickly(Format::PlantUml, &src);
}

#[test]
fn escaped_mermaid_attribute_labels_count_toward_the_retained_byte_budget() {
    // Each label is a backslash pair plus 1_048_575 letters. The pair converts to one
    // backslash, so the converted text is exactly 1 MiB. The logical name is
    // `mermaid.flowchart/node[n0]`, 26 bytes (27 from n10). Fifteen fragments total
    // 10 * 1_048_602 + 5 * 1_048_603 = 15_729_035 bytes, within 16_777_216. The sixteenth
    // would reach 16_777_638, so it is dropped. A fragment counted before its converted
    // text is attached would let all twenty through.
    let letters = "x".repeat(MIB - 1);
    let mut src = String::from("flowchart LR\n");
    for i in 0..20 {
        writeln!(src, "  n{i}@{{ label: \"\\\\{letters}\" }}").unwrap();
    }
    let ex = extract(Format::Mermaid, &src, 0);
    assert_eq!(ex.fragments.len(), 15);
    let retained: usize = ex
        .fragments
        .iter()
        .map(|f| f.logical.len() + f.text.as_ref().map_or(0, String::len))
        .sum();
    assert_eq!(retained, 15_729_035);
    assert_eq!(hit(&ex), ("retained-bytes", 16_777_216));
}

#[test]
fn a_draw_io_page_after_an_exhausted_budget_is_never_read() {
    // The first page fills the fragment budget. The second page is malformed, and reading
    // it would turn the status into Malformed.
    let mut src = String::from("<mxfile><diagram name=\"One\"><mxGraphModel><root>");
    for i in 0..=MAX_FRAGMENTS {
        write!(src, "<mxCell id=\"c{i}\" value=\"L{i}\" vertex=\"1\"/>").unwrap();
    }
    src.push_str(
        "</root></mxGraphModel></diagram><diagram name=\"Two\">!!!notbase64</diagram></mxfile>",
    );
    let ex = extract(Format::Drawio, &src, 0);
    assert_eq!(ex.fragments.len(), MAX_FRAGMENTS);
    assert_eq!(hit(&ex), ("fragments", 100_000));
}
