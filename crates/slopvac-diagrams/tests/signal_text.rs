//! The sentence of each signal hit, as the caller reads it.

use slopvac_diagrams::{Format, ProseBlock, Signal, check_labels, check_prose, extract};

fn label_hits(source: &str) -> Vec<(Signal, String)> {
    let found = extract(Format::Mermaid, source, 0);
    check_labels(source, &found.fragments, Format::Mermaid)
        .into_iter()
        .map(|hit| (hit.signal, hit.message))
        .collect()
}

#[test]
fn a_run_on_label_reports_its_word_count() {
    assert_eq!(
        label_hits("flowchart TD\n  A[one two three four five six seven]\n"),
        [(
            Signal::LabelRunOn,
            "This label has 7 words. Use a short noun phrase and move the detail to the text."
                .to_owned()
        )]
    );
}

#[test]
fn prose_that_points_at_the_diagram_is_told_to_say_what_the_system_does() {
    let source = "flowchart TD\n  A[One] --> B[Two]\n";
    let found = extract(Format::Mermaid, source, 0);
    let text = "Requests fan out, as shown in the diagram.";
    let prose = [ProseBlock {
        span: 0..text.len(),
        text,
    }];
    let hits = check_prose(source, &found.fragments, &prose);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].signal, Signal::MetaReference);
    assert_eq!(
        hits[0].message,
        "This sentence points at the diagram. Say what the system does and let the diagram show the shape."
    );
}
