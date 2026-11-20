//! Retained bytes of an extraction grow linearly with its input, not with identifier length
//! times member count.

use std::fmt::Write as _;

use rstest::rstest;
use slopvac_diagrams::{Extraction, ExtractionStatus, Format, Role, extract};

const MEMBERS: usize = 10_000;
/// An identifier of 100 KB.
const LONG: usize = 100 * 1024;
/// Upper bound on one `logical`: two clipped names of 128 characters plus an ellipsis
/// (3 bytes) each, and under 40 bytes of fixed text such as `drawio//node[]`.
const PER_FRAGMENT: usize = 2 * (128 + 3) + 40;

fn retained(ex: &Extraction) -> usize {
    ex.fragments
        .iter()
        .map(|f| f.logical.len() + f.text.as_ref().map_or(0, String::len))
        .sum()
}

fn assert_linear(ex: &Extraction, members: usize) {
    assert_eq!(ex.status, ExtractionStatus::Ok);
    assert_eq!(
        ex.fragments
            .iter()
            .filter(|f| f.role == Role::Member)
            .count(),
        members
    );
    assert!(
        ex.fragments
            .iter()
            .filter(|f| f.role == Role::Member)
            .all(|f| f.logical.contains('…')),
        "the long name is clipped in every member"
    );
    // Every fragment is a member or the class name, so the count is bounded by members + 1.
    let bound = (members + 1) * PER_FRAGMENT;
    let got = retained(ex);
    assert!(
        got <= bound,
        "retained {got} bytes, bound {bound} (unbounded would be about {})",
        members * LONG
    );
}

#[test]
fn a_long_mermaid_class_name_is_not_copied_into_every_member() {
    let name = "C".repeat(LONG);
    let mut src = format!("classDiagram\n  class {name} {{\n");
    for i in 0..MEMBERS {
        writeln!(src, "    +int m{i}").unwrap();
    }
    src.push_str("  }\n");
    assert_linear(&extract(Format::Mermaid, &src, 0), MEMBERS);
}

#[test]
fn a_long_plantuml_class_name_is_not_copied_into_every_member() {
    let name = "C".repeat(LONG);
    let mut src = format!("@startuml\nclass {name} {{\n");
    for i in 0..MEMBERS {
        writeln!(src, "  +int m{i}").unwrap();
    }
    src.push_str("}\n@enduml\n");
    assert_linear(&extract(Format::PlantUml, &src, 0), MEMBERS);
}

#[test]
fn a_long_drawio_page_name_is_not_copied_into_every_cell() {
    let page = "P".repeat(LONG);
    let mut src = format!("<mxfile><diagram name=\"{page}\"><mxGraphModel><root>");
    for i in 0..MEMBERS {
        write!(src, "<mxCell id=\"c{i}\" value=\"L{i}\" vertex=\"1\"/>").unwrap();
    }
    src.push_str("</root></mxGraphModel></diagram></mxfile>");
    let ex = extract(Format::Drawio, &src, 0);
    assert_eq!(ex.status, ExtractionStatus::Ok);
    assert_eq!(ex.fragments.len(), MEMBERS);
    let bound = MEMBERS * PER_FRAGMENT;
    assert!(retained(&ex) <= bound, "retained {}", retained(&ex));
}

#[test]
fn a_long_excalidraw_element_id_is_clipped() {
    let id = "E".repeat(200);
    let src = format!("{{\"elements\":[{{\"id\":\"{id}\",\"type\":\"text\",\"text\":\"t\"}}]}}");
    let ex = extract(Format::Excalidraw, &src, 0);
    let want = format!("excalidraw/text[{}…]", "E".repeat(128));
    assert_eq!(ex.fragments[0].logical, want);
}

/// Identifier lengths around the 128-character cap, in one-byte and multi-byte characters.
/// A name of 128 characters stays whole. A longer one keeps its first 128 characters, cut on
/// a character boundary, and ends in `…`.
#[rstest]
#[case::at_the_cap("x", 128, false)]
#[case::one_over("x", 129, true)]
#[case::multibyte_at_the_cap("é", 128, false)]
#[case::multibyte_over("é", 130, true)]
fn an_identifier_is_clipped_only_past_the_cap(
    #[case] unit: &str,
    #[case] chars: usize,
    #[case] cut: bool,
) {
    let name = unit.repeat(chars);
    let src = format!("flowchart TD\n  {name}[Label]\n");
    let ex = extract(Format::Mermaid, &src, 0);
    let shown = if cut {
        format!("{}…", unit.repeat(128))
    } else {
        name
    };
    assert_eq!(
        ex.fragments[0].logical,
        format!("mermaid.flowchart/node[{shown}]")
    );
}
