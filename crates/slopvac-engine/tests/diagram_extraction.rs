//! A diagram that cannot be read in full makes the run incomplete.

mod common;

use std::fmt::Write as _;

use common::{category, ruleset, settings};
use slopvac_diagrams::Format;
use slopvac_engine::{
    DIAGRAM_PACK, DiagramChecker, DiagramHit, DiagramInput, DiagramRule, Engine, Input,
    diagram_rule_set,
};
use slopvac_model::{Document, Notice, NoticeLevel, PackId, RuleId, Severity};

fn engine() -> Engine {
    let pack = diagram_rule_set(&PackId::new(DIAGRAM_PACK).unwrap()).unwrap();
    Engine::new(vec![ruleset(vec![category("cat", &[])]), pack], settings()).unwrap()
}

fn codes(doc: &Document) -> Vec<&str> {
    doc.unchecked.iter().map(|n| n.code.as_str()).collect()
}

fn assert_incomplete(doc: &Document, code: &str, reason: &str) {
    assert_eq!(codes(doc), [code]);
    let n = &doc.unchecked[0];
    assert_eq!(n.level, NoticeLevel::Warning);
    assert_eq!(
        n.message.as_str(),
        format!("{reason}; the diagram was not fully checked")
    );
    assert_eq!(n.path.as_deref(), Some(doc.path.as_str()));
    assert!(!doc.gate.is_passed());
    assert!(
        doc.gate.reasons()[0].starts_with("incomplete check:"),
        "{:?}",
        doc.gate.reasons()
    );
}

/// A draw.io file whose node count passes the one-million limit: the root, the model and the
/// `root` element, then one million cells, is more than the limit allows.
fn too_many_nodes() -> String {
    format!(
        "<mxGraphModel><root>{}</root></mxGraphModel>",
        "<mxCell/>".repeat(1_000_000)
    )
}

#[test]
fn a_diagram_that_cannot_be_read_is_an_incomplete_run() {
    let nodes = too_many_nodes();
    let cases: [(&str, &str, Format, &str, &str); 8] = [
        (
            "malformed excalidraw",
            "{",
            Format::Excalidraw,
            "extract-malformed",
            "invalid JSON",
        ),
        (
            "malformed draw.io xml",
            "<mxfile",
            Format::Drawio,
            "extract-malformed",
            "invalid XML",
        ),
        (
            "bad compressed draw.io page",
            "<mxfile><diagram name=\"A\">!!!notbase64</diagram></mxfile>",
            Format::Drawio,
            "extract-malformed",
            "invalid base64 page",
        ),
        (
            "draw.io node budget",
            &nodes,
            Format::Drawio,
            "extract-budget",
            "the xml-nodes limit of 1000000 was reached",
        ),
        (
            "mermaid front matter that never closes",
            "---\ntitle: X\nflowchart TD\n  A[Label] --> B[End]\n",
            Format::Mermaid,
            "extract-malformed",
            "front matter is not closed",
        ),
        (
            "plantuml that never ends",
            "@startuml\nAlice -> Bob : hello\n",
            Format::PlantUml,
            "extract-malformed",
            "missing @enduml",
        ),
        (
            "draw.io file with no pages",
            "<mxfile><mxGraphModel/></mxfile>",
            Format::Drawio,
            "extract-malformed",
            "no diagram pages",
        ),
        (
            "draw.io page with unrecognised content",
            "<mxfile><diagram name=\"A\"><unknown/></diagram></mxfile>",
            Format::Drawio,
            "extract-malformed",
            "unrecognised page content",
        ),
    ];
    for (name, source, format, code, reason) in cases {
        let doc = engine()
            .lint(&Input::diagram("d.file", source, format))
            .unwrap();
        assert!(doc.findings.is_empty(), "{name}");
        assert_incomplete(&doc, code, reason);
    }
}

#[test]
fn a_bad_later_draw_io_page_keeps_the_findings_of_the_earlier_page() {
    let source = "<mxfile><diagram name=\"One\"><mxGraphModel><root>\
        <mxCell id=\"a\" value=\"This label has far too many words in it\" vertex=\"1\"/>\
        </root></mxGraphModel></diagram>\
        <diagram name=\"Two\">!!!notbase64</diagram></mxfile>";
    let doc = engine()
        .lint(&Input::diagram("d.drawio", source, Format::Drawio))
        .unwrap();
    assert_eq!(doc.findings.len(), 1);
    assert_eq!(
        doc.findings[0].matched_text,
        "This label has far too many words in it"
    );
    assert_eq!(codes(&doc), ["extract-malformed"]);
    assert!(!doc.gate.is_passed());
}

#[test]
fn the_extract_code_family_is_final() {
    use slopvac_engine::extract_code::{BUDGET, MALFORMED, OUTPUT_INVALID, UNSUPPORTED};
    assert_eq!(
        [UNSUPPORTED, MALFORMED, BUDGET, OUTPUT_INVALID],
        [
            "extract-unsupported",
            "extract-malformed",
            "extract-budget",
            "extract-output-invalid"
        ]
    );
}

/// Each cap reports its name and value in the notice text.
#[test]
fn a_budget_notice_names_the_cap_and_its_limit() {
    let mut fragments = String::from("flowchart TD\n");
    for i in 0..100_005 {
        writeln!(fragments, "  A{i}[L{i}]").unwrap();
    }
    let nodes = too_many_nodes();
    // 16 texts of 1 MiB converted bytes, each written as 2 MiB of `\n` escapes.
    let text = "\\n".repeat(1024 * 1024);
    let elements: Vec<String> = (0..16)
        .map(|i| format!("{{\"id\":\"e{i}\",\"type\":\"text\",\"text\":\"{text}\"}}"))
        .collect();
    let scene = format!("{{\"elements\":[{}]}}", elements.join(","));
    for (name, source, format, cap, limit) in [
        (
            "fragments",
            &fragments,
            Format::Mermaid,
            "fragments",
            "100000",
        ),
        (
            "retained bytes",
            &scene,
            Format::Excalidraw,
            "retained-bytes",
            "16777216",
        ),
        ("xml nodes", &nodes, Format::Drawio, "xml-nodes", "1000000"),
    ] {
        let doc = engine()
            .lint(&Input::diagram("d.file", source, format))
            .unwrap();
        assert_eq!(codes(&doc), ["extract-budget"], "{name}");
        assert_eq!(
            doc.unchecked[0].message.as_str(),
            format!("the {cap} limit of {limit} was reached; the diagram was not fully checked"),
            "{name}"
        );
        assert!(!doc.gate.is_passed(), "{name}");
        let data = serde_json::Value::Object(doc.unchecked[0].data.clone());
        let limit: u64 = limit.parse().unwrap();
        assert_eq!(
            data,
            serde_json::json!({"budget": cap, "limit": limit}),
            "{name}"
        );
    }
}

#[test]
fn a_comment_only_mermaid_source_still_passes() {
    let doc = engine()
        .lint(&Input::diagram(
            "d.mmd",
            "%% only a comment\n",
            Format::Mermaid,
        ))
        .unwrap();
    assert_eq!(codes(&doc), Vec::<&str>::new());
    assert!(doc.gate.is_passed());
}

#[test]
fn a_diagram_with_no_labels_still_passes() {
    for (name, source, format) in [
        ("mermaid header only", "flowchart TD\n", Format::Mermaid),
        (
            "closed plantuml without labels",
            "@startuml\n@enduml\n",
            Format::PlantUml,
        ),
        ("empty draw.io file", "<mxfile/>", Format::Drawio),
        (
            "empty draw.io page",
            "<mxfile><diagram name=\"A\"/></mxfile>",
            Format::Drawio,
        ),
        (
            "empty excalidraw scene",
            "{\"elements\":[]}",
            Format::Excalidraw,
        ),
    ] {
        let doc = engine()
            .lint(&Input::diagram("d.file", source, format))
            .unwrap();
        assert!(doc.unchecked.is_empty(), "{name}: {:?}", codes(&doc));
        assert!(doc.findings.is_empty(), "{name}");
        assert!(doc.gate.is_passed(), "{name}: {:?}", doc.gate.reasons());
    }
}
const LONG_LABEL: &str = "This label has far too many words in it";

#[test]
fn a_second_plantuml_block_is_checked_and_a_valid_two_block_file_passes() {
    let two = format!(
        "@startuml\nA -> B : ok\n@enduml\n@startuml\nrectangle \"{LONG_LABEL}\" as R\n@enduml\n"
    );
    let doc = engine()
        .lint(&Input::diagram("d.puml", &two, Format::PlantUml))
        .unwrap();
    assert_eq!(doc.findings.len(), 1);
    assert_eq!(doc.findings[0].matched_text, LONG_LABEL);
    assert!(doc.unchecked.is_empty(), "{:?}", codes(&doc));

    let valid = "@startuml\nA -> B : ok\n@enduml\n\n@startuml\nC -> D : fine\n@enduml\n";
    let doc = engine()
        .lint(&Input::diagram("d.puml", valid, Format::PlantUml))
        .unwrap();
    assert_eq!(doc.findings, []);
    assert!(doc.unchecked.is_empty(), "{:?}", codes(&doc));
    assert!(doc.gate.is_passed());
}

#[test]
fn an_unsupported_plantuml_block_after_a_supported_one_keeps_the_supported_findings() {
    let source = format!(
        "@startuml\nrectangle \"{LONG_LABEL}\" as R\n@enduml\n@startmindmap\n* root\n@endmindmap\n"
    );
    let (doc, notices) = engine()
        .lint_with_notices(&Input::diagram("d.puml", &source, Format::PlantUml))
        .unwrap();
    assert_eq!(doc.findings.len(), 1);
    assert_eq!(doc.findings[0].matched_text, LONG_LABEL);
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert_eq!(notices[0].code.as_str(), "extract-unsupported");
    assert_eq!(notices[0].level, NoticeLevel::Info);
    assert!(doc.unchecked.is_empty(), "{:?}", codes(&doc));
    assert!(
        doc.gate
            .reasons()
            .iter()
            .all(|r| !r.starts_with("incomplete check:")),
        "{:?}",
        doc.gate.reasons()
    );
}

#[test]
fn a_start_marker_inside_a_plantuml_comment_is_not_a_block() {
    for (name, comment) in [
        ("inside a block", "/'\n@startmindmap\n'/\n"),
        ("on one line", "/' @startmindmap '/\n"),
    ] {
        for (place, source) in [
            (
                "in",
                format!("@startuml\n{comment}rectangle \"{LONG_LABEL}\" as R\n@enduml\n"),
            ),
            (
                "after",
                format!("@startuml\nrectangle \"{LONG_LABEL}\" as R\n@enduml\n{comment}"),
            ),
        ] {
            let (doc, notices) = engine()
                .lint_with_notices(&Input::diagram("d.puml", &source, Format::PlantUml))
                .unwrap();
            assert_eq!(doc.findings.len(), 1, "{name} {place}");
            assert_eq!(doc.findings[0].matched_text, LONG_LABEL, "{name} {place}");
            assert!(notices.is_empty(), "{name} {place}: {notices:?}");
            assert!(
                doc.unchecked.is_empty(),
                "{name} {place}: {:?}",
                codes(&doc)
            );
        }
    }
}

#[test]
fn a_mismatched_plantuml_terminator_is_malformed() {
    for end in ["@endmindmap", "@endfoo"] {
        let source = format!("@startuml\nA -> B : hi\n{end}\n");
        let doc = engine()
            .lint(&Input::diagram("d.puml", &source, Format::PlantUml))
            .unwrap();
        assert_incomplete(&doc, "extract-malformed", "mismatched terminator");
    }
}

#[test]
fn an_unclosed_plantuml_block_before_an_unsupported_start_is_malformed_not_unsupported() {
    let source = "@startuml\nA -> B : hi\n@startmindmap\n* root\n@endmindmap\n";
    let (doc, notices) = engine()
        .lint_with_notices(&Input::diagram("d.puml", source, Format::PlantUml))
        .unwrap();
    assert_incomplete(&doc, "extract-malformed", "missing @enduml");
    assert!(notices.is_empty(), "{notices:?}");
}

#[test]
fn a_malformed_excalidraw_element_is_an_incomplete_run_that_keeps_earlier_findings() {
    let first = format!(
        "{{\"id\":\"r1\",\"type\":\"rectangle\"}},\
         {{\"id\":\"t1\",\"type\":\"text\",\"containerId\":\"r1\",\"text\":\"{LONG_LABEL}\"}}"
    );
    for (name, bad, reason) in [
        ("non-object entry", "7", "element is not an object"),
        (
            "text element without text",
            "{\"id\":\"t2\",\"type\":\"text\"}",
            "text element has no text",
        ),
        (
            "text element with a non-string text",
            "{\"id\":\"t2\",\"type\":\"text\",\"text\":5}",
            "text element has no text",
        ),
    ] {
        let source = format!("{{\"elements\":[{first},{bad}]}}");
        let doc = engine()
            .lint(&Input::diagram("d.excalidraw", &source, Format::Excalidraw))
            .unwrap();
        assert_eq!(doc.findings.len(), 1, "{name}");
        assert_incomplete(&doc, "extract-malformed", reason);
    }
}

#[test]
fn a_drawio_file_holding_only_comments_and_whitespace_stays_valid() {
    for (name, source) in [
        ("comment", "<mxfile><!-- nothing here --></mxfile>"),
        (
            "comment and whitespace",
            "<mxfile>\n  <!-- note -->\n</mxfile>",
        ),
    ] {
        let doc = engine()
            .lint(&Input::diagram("d.drawio", source, Format::Drawio))
            .unwrap();
        assert!(doc.unchecked.is_empty(), "{name}: {:?}", codes(&doc));
        assert_eq!(doc.findings, [], "{name}");
        assert!(doc.gate.is_passed(), "{name}: {:?}", doc.gate.reasons());
    }
}

#[test]
fn excalidraw_elements_that_hold_no_text_stay_valid() {
    let source = "{\"elements\":[\
        {\"id\":\"r\",\"type\":\"rectangle\"},\
        {\"id\":\"t\",\"type\":\"text\",\"isDeleted\":true},\
        {\"id\":\"f\",\"type\":\"frame\"}]}";
    let doc = engine()
        .lint(&Input::diagram("d.excalidraw", source, Format::Excalidraw))
        .unwrap();
    assert_eq!(doc.findings, []);
    assert!(doc.unchecked.is_empty(), "{:?}", codes(&doc));
    assert!(doc.gate.is_passed());
}

/// The run-level notice and the clean verdict for content that is valid but not read.
fn assert_unsupported_info(
    doc: &Document,
    notices: &[Notice],
    path: &str,
    reason: &str,
    kind: &str,
) {
    assert_eq!(notices.len(), 1, "{notices:?}");
    let n = &notices[0];
    assert_eq!(n.code.as_str(), "extract-unsupported");
    assert_eq!(n.level, NoticeLevel::Info);
    assert_eq!(
        n.message.as_str(),
        format!("{reason}; the diagram was not checked")
    );
    assert_eq!(n.path.as_deref(), Some(path));
    assert_eq!(
        serde_json::Value::Object(n.data.clone()),
        serde_json::json!({"document": path, "kind": kind})
    );
    assert!(doc.unchecked.is_empty(), "{:?}", codes(doc));
    assert_eq!(doc.findings, []);
    assert!(doc.gate.is_passed(), "{:?}", doc.gate.reasons());
}

#[test]
fn unsupported_content_in_a_diagram_file_is_an_info_notice_and_a_complete_run() {
    for (name, source, format, reason, kind) in [
        (
            "plantuml mind map",
            "@startmindmap\n* root\n** child\n@endmindmap\n",
            Format::PlantUml,
            "unsupported PlantUML diagram type",
            "plantuml",
        ),
        (
            "mermaid gitGraph",
            "gitGraph\n  commit\n  branch dev\n",
            Format::Mermaid,
            "unsupported Mermaid diagram type",
            "mermaid",
        ),
    ] {
        let (doc, notices) = engine()
            .lint_with_notices(&Input::diagram("d.file", source, format))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_unsupported_info(&doc, &notices, "d.file", reason, kind);
    }
}

#[test]
fn an_unsupported_fenced_diagram_is_an_info_notice_and_the_markdown_run_passes() {
    let source = "Intro.\n\n```mermaid\ngitGraph\n  commit\n```\n";
    let (doc, notices) = engine()
        .lint_with_notices(&Input::markdown("a.md", source))
        .unwrap();
    assert_unsupported_info(
        &doc,
        &notices,
        "a.md",
        "unsupported Mermaid diagram type",
        "mermaid",
    );
}

#[test]
fn an_unsupported_fenced_plantuml_type_is_info_and_a_broken_supported_type_is_incomplete() {
    let (doc, notices) = engine()
        .lint_with_notices(&Input::markdown(
            "a.md",
            "Intro.\n\n```plantuml\n@startmindmap\n* root\n@endmindmap\n```\n",
        ))
        .unwrap();
    assert_unsupported_info(
        &doc,
        &notices,
        "a.md",
        "unsupported PlantUML diagram type",
        "plantuml",
    );

    for (name, fence, reason) in [
        (
            "plantuml that never ends",
            "```plantuml\n@startuml\nA -> B : hi\n```\n",
            "missing @enduml",
        ),
        (
            "mermaid front matter that never closes",
            "```mermaid\n---\ntitle: X\nflowchart TD\n  A --> B\n```\n",
            "front matter is not closed",
        ),
    ] {
        let source = format!("Intro.\n\n{fence}");
        let (doc, notices) = engine()
            .lint_with_notices(&Input::markdown("a.md", &source))
            .unwrap();
        assert!(notices.is_empty(), "{name}: {notices:?}");
        assert_incomplete(&doc, "extract-malformed", reason);
    }
}

#[test]
fn a_drawio_svg_without_a_diagram_source_is_an_incomplete_run() {
    let source = "<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
    let (doc, notices) = engine()
        .lint_with_notices(&Input::diagram("d.drawio.svg", source, Format::DrawioSvg))
        .unwrap();
    assert!(notices.is_empty(), "{notices:?}");
    assert_incomplete(&doc, "extract-malformed", "no diagram source");
    assert_eq!(doc.unchecked[0].path.as_deref(), Some("d.drawio.svg"));
}

/// The path, code and level of each notice of one input.
type Seen<'a> = Vec<(String, &'a str, NoticeLevel)>;

/// A checker that returns a range past the end of the file for a source that holds `BOOM`,
/// so that one input of a batch fails with an engine error.
struct Boom;

impl DiagramChecker for Boom {
    fn rules(&self) -> Vec<DiagramRule> {
        vec![DiagramRule {
            id: RuleId::new("acme:diagram.boom").unwrap(),
            severity: Severity::Suggestion,
        }]
    }

    fn check(&self, input: &DiagramInput<'_>) -> Vec<DiagramHit> {
        if !input.source.contains("BOOM") {
            return Vec::new();
        }
        vec![DiagramHit {
            rule: RuleId::new("acme:diagram.boom").unwrap(),
            span: input.source.len()..input.source.len() + 10,
            message: "bad".to_owned(),
            matched: None,
        }]
    }
}

#[test]
fn a_mixed_batch_keeps_input_order_notices_and_per_input_errors() {
    let nodes = too_many_nodes();
    let inputs = [
        Input::diagram("a.mmd", "gitGraph\n  commit\n", Format::Mermaid),
        Input::diagram(
            "b.mmd",
            "flowchart TD\n  A[BOOM] --> B[x]\n",
            Format::Mermaid,
        ),
        Input::diagram(
            "c.puml",
            "@startmindmap\n* r\n@endmindmap\n",
            Format::PlantUml,
        ),
        Input::diagram("d.excalidraw", "{", Format::Excalidraw),
        Input::markdown("e.md", "Intro.\n\n```mermaid\ngitGraph\n  commit\n```\n"),
        Input::diagram("f.drawio", &nodes, Format::Drawio),
        Input::diagram(
            "g.mmd",
            "flowchart TD\n  A[ok] --> B[ok]\n",
            Format::Mermaid,
        ),
    ];
    let engine = engine().with_diagram_checker(Box::new(Boom));
    let results = engine.lint_all_with_notices(&inputs);
    assert_eq!(results.len(), inputs.len());

    // The notices of each input, by position: path, code and level.
    let seen: Vec<Option<Seen<'_>>> = results
        .iter()
        .map(|r| {
            r.as_ref().ok().map(|(_, notices)| {
                notices
                    .iter()
                    .map(|n| (n.path.clone().unwrap(), n.code.as_str(), n.level))
                    .collect()
            })
        })
        .collect();
    let info = |path: &str| vec![(path.to_owned(), "extract-unsupported", NoticeLevel::Info)];
    assert_eq!(
        seen,
        [
            Some(info("a.mmd")),
            None,
            Some(info("c.puml")),
            Some(vec![]),
            Some(info("e.md")),
            Some(vec![]),
            Some(vec![]),
        ]
    );
    assert!(matches!(
        results[1],
        Err(slopvac_engine::EngineError::BadRange(_))
    ));

    // Documents follow the input order, and each verdict is its own.
    let verdicts: Vec<(&str, bool, Vec<&str>)> = results
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .map(|(doc, _)| (doc.path.as_str(), doc.gate.is_passed(), codes(doc)))
        .collect();
    assert_eq!(
        verdicts,
        [
            ("a.mmd", true, vec![]),
            ("c.puml", true, vec![]),
            ("d.excalidraw", false, vec!["extract-malformed"]),
            ("e.md", true, vec![]),
            ("f.drawio", false, vec!["extract-budget"]),
            ("g.mmd", true, vec![]),
        ]
    );
}

#[test]
fn lint_leaves_the_run_notices_out_of_the_document() {
    let doc = engine()
        .lint(&Input::diagram(
            "d.mmd",
            "gitGraph\n  commit\n",
            Format::Mermaid,
        ))
        .unwrap();
    assert_eq!(doc.unchecked, []);
    assert!(doc.gate.is_passed());
}

#[test]
fn malformed_and_over_budget_diagrams_give_no_run_notice_and_stay_incomplete() {
    let nodes = too_many_nodes();
    for (name, source, format) in [
        ("malformed", "{", Format::Excalidraw),
        ("over budget", nodes.as_str(), Format::Drawio),
    ] {
        let (doc, notices) = engine()
            .lint_with_notices(&Input::diagram("d.file", source, format))
            .unwrap();
        assert!(notices.is_empty(), "{name}: {notices:?}");
        assert!(!doc.gate.is_passed(), "{name}");
        assert_eq!(doc.unchecked.len(), 1, "{name}");
        assert_eq!(doc.unchecked[0].level, NoticeLevel::Warning, "{name}");
    }
}
