//! Every output format against the shared position vectors and injection cases.

use serde_json::{Value, json};
use slopvac_model::Report;
use slopvac_report::{Format, NoSources, RenderError, Sources, render};
use std::collections::HashMap;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schemas/v3");

fn read(path: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(format!("{ROOT}/{path}")).unwrap()).unwrap()
}

fn num(v: &Value) -> u64 {
    v.as_u64().unwrap()
}

/// A one-document report with one finding covering `span`.
fn report_for(span: Option<(u64, u64)>, coords: [u64; 4], message: &str) -> Report {
    let mut base = read("conformance/lint-report/valid/multi-document.json");
    let mut doc = base["documents"][0].clone();
    doc["path"] = json!("t.txt");
    let mut finding = json!({
        "rule_id": "core:test.rule", "pack": "core", "category": "orwell",
        "severity": "warning", "message": message, "line": coords[0], "column": coords[1],
        "contexts": ["prose"], "ai_signal": "none", "matched_text": "x",
    });
    if let Some((start, end)) = span {
        finding["span"] = json!({"start": start, "end": end});
        finding["end_line"] = json!(coords[2]);
        finding["end_column"] = json!(coords[3]);
    }
    doc["findings"] = json!([finding]);
    doc["unchecked"] = json!([]);
    doc["gate"] = json!({"passed": true, "reasons": []});
    base["documents"] = json!([doc]);
    serde_json::from_value(base).unwrap()
}

fn out(format: Format, report: &Report, text: &str) -> String {
    let sources: HashMap<String, String> = HashMap::from([("t.txt".into(), text.into())]);
    render(format, report, &sources).unwrap()
}

#[test]
fn spans_render_as_the_vectors_say() {
    let v = read("conformance/positions/vectors.json");
    let spans = v["spans"].as_array().unwrap();
    assert!(spans.len() >= 15);
    for case in spans {
        let id = case["id"].as_str().unwrap();
        let text = v["documents"][case["document"].as_str().unwrap()]["text"]
            .as_str()
            .unwrap();
        let f = &case["formats"];
        let (s, e) = (&case["start"], &case["end"]);
        let coords = [
            num(&s["line"]),
            num(&s["utf32"]),
            num(&e["line"]),
            num(&e["utf32"]),
        ];
        let span = (num(&case["span"]["start"]), num(&case["span"]["end"]));
        let report = report_for(Some(span), coords, "msg");
        let (line, col) = (coords[0], coords[1]);

        let sarif: Value = serde_json::from_str(&out(Format::Sarif, &report, text)).unwrap();
        assert_eq!(sarif["runs"][0]["columnKind"], "utf16CodeUnits", "{id}");
        let region = &sarif["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["region"];
        assert_eq!(region, &f["sarif"], "sarif {id}");

        let rd: Value = serde_json::from_str(out(Format::Rdjsonl, &report, text).trim()).unwrap();
        let range = &rd["location"]["range"];
        assert_eq!(range["start"], f["rdjsonl"]["start"], "rdjsonl start {id}");
        assert_eq!(range["end"], f["rdjsonl"]["end"], "rdjsonl end {id}");

        let cc: Value = serde_json::from_str(&out(Format::CodeClimate, &report, text)).unwrap();
        let loc = &cc[0]["location"];
        assert_eq!(
            loc["lines"], f["codeclimate"]["lines"],
            "codeclimate lines {id}"
        );
        assert!(
            loc.get("positions").is_none(),
            "codeclimate emits lines only {id}"
        );
        assert!(f["codeclimate"].get("positions").is_none(), "{id}");

        let checkstyle = out(Format::Checkstyle, &report, text);
        assert!(
            checkstyle.contains(&format!("line=\"{line}\" column=\"{col}\"")),
            "checkstyle {id}"
        );
        assert_eq!(
            f["checkstyle"],
            json!({"line": line, "column": col}),
            "{id}"
        );
        assert!(
            out(Format::Junit, &report, text)
                .contains(&format!("name=\"core:test.rule {line}:{col}\"")),
            "junit {id}"
        );
        assert!(
            out(Format::Github, &report, text)
                .contains(&format!("file=t.txt,line={line},col={col},")),
            "github {id}"
        );
        let want = format!("t.txt:{line}:{col} warning core:test.rule msg");
        assert!(
            out(Format::Text, &report, text).contains(&want),
            "text {id}"
        );
        assert!(
            out(Format::Markdown, &report, text).contains(&format!("| {line}:{col} |")),
            "markdown {id}"
        );
        assert!(
            out(Format::Html, &report, text).contains(&format!("<td>{line}:{col}</td>")),
            "html {id}"
        );
        assert_eq!(f["text"], format!("{line}:{col}"), "{id}");
    }
}

#[test]
fn formats_without_byte_columns_work_without_source_text() {
    let report = report_for(Some((0, 3)), [1, 1, 1, 4], "msg");
    for format in [
        Format::Text,
        Format::Json,
        Format::Github,
        Format::Junit,
        Format::Checkstyle,
        Format::Markdown,
        Format::Html,
    ] {
        render(format, &report, &NoSources).unwrap_or_else(|e| panic!("{}: {e}", format.name()));
    }
    for format in [Format::Sarif, Format::Rdjsonl, Format::CodeClimate] {
        let err = render(format, &report, &NoSources).unwrap_err();
        assert!(
            matches!(err, RenderError::MissingSource { .. }),
            "{}",
            format.name()
        );
    }
}

#[test]
fn findings_without_a_span_need_no_source_anywhere() {
    let report = report_for(None, [3, 7, 0, 0], "doc-wide");
    for format in Format::ALL {
        render(format, &report, &NoSources).unwrap_or_else(|e| panic!("{}: {e}", format.name()));
    }
    let sarif: Value =
        serde_json::from_str(&render(Format::Sarif, &report, &NoSources).unwrap()).unwrap();
    let region = &sarif["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["region"];
    assert_eq!(region, &json!({"startLine": 3}));
}

#[test]
fn a_span_past_the_text_is_a_position_error() {
    let report = report_for(Some((0, 99)), [1, 1, 1, 2], "msg");
    let sources: HashMap<String, String> = HashMap::from([("t.txt".into(), "abc".into())]);
    let err = render(Format::Sarif, &report, &sources).unwrap_err();
    assert!(matches!(err, RenderError::Position { .. }), "{err}");
}

#[test]
fn hostile_messages_stay_inert_in_markup_formats() {
    let nasty = "<script>alert(1)</script> & \"q\" | `x` \u{1}";
    let report = report_for(Some((0, 1)), [1, 1, 1, 2], nasty);
    for format in [Format::Html, Format::Junit, Format::Checkstyle] {
        let rendered = out(format, &report, "a");
        assert!(!rendered.contains("<script>"), "{}", format.name());
        assert!(!rendered.contains('\u{1}'), "{}", format.name());
    }
    let md = out(Format::Markdown, &report, "a");
    assert!(!md.contains("<script>") && md.contains("\\|"));
}

#[test]
fn github_annotations_escape_newlines() {
    let report = report_for(Some((0, 1)), [1, 1, 1, 2], "a\nb::c");
    let rendered = out(Format::Github, &report, "a");
    assert_eq!(rendered.lines().count(), 1);
    assert!(rendered.contains("a%0Ab::c"));
}

#[test]
fn sarif_carries_fix_regions_in_utf16() {
    let mut report = report_for(Some((1, 5)), [1, 2, 1, 3], "msg");
    let finding = &mut report.documents[0].findings[0];
    finding.fix = Some(
        serde_json::from_value(json!({"edits": [{"span": {"start": 1, "end": 5}, "text": "X"}]}))
            .unwrap(),
    );
    let sarif: Value = serde_json::from_str(&out(Format::Sarif, &report, "a\u{1F600}b\n")).unwrap();
    let rep = &sarif["runs"][0]["results"][0]["fixes"][0]["artifactChanges"][0]["replacements"][0];
    assert_eq!(
        rep["deletedRegion"],
        json!({"startLine": 1, "startColumn": 2, "endLine": 1, "endColumn": 4})
    );
    assert_eq!(rep["insertedContent"]["text"], "X");
}

#[test]
fn rdjsonl_suggestions_use_byte_columns() {
    let mut report = report_for(Some((1, 5)), [1, 2, 1, 3], "msg");
    report.documents[0].findings[0].fix = Some(
        serde_json::from_value(json!({"edits": [{"span": {"start": 1, "end": 5}, "text": "X"}]}))
            .unwrap(),
    );
    let rd: Value =
        serde_json::from_str(out(Format::Rdjsonl, &report, "a\u{1F600}b\n").trim()).unwrap();
    assert_eq!(
        rd["suggestions"][0]["range"],
        json!({"start": {"line": 1, "column": 2}, "end": {"line": 1, "column": 6}})
    );
}

#[test]
fn every_format_name_round_trips() {
    for format in Format::ALL {
        assert_eq!(Format::from_name(format.name()), Some(format));
    }
    assert_eq!(Format::from_name("xml"), None);
}

#[test]
fn outputs_end_with_one_newline() {
    let report = report_for(Some((0, 1)), [1, 1, 1, 2], "m");
    for format in Format::ALL {
        let rendered = out(format, &report, "a");
        assert!(
            rendered.ends_with('\n') && !rendered.ends_with("\n\n"),
            "{}",
            format.name()
        );
    }
    let _: &dyn Sources = &NoSources;
}

fn sarif_validator() -> jsonschema::Validator {
    let schema: Value =
        serde_json::from_str(include_str!("fixtures/sarif-schema-2.1.0.json")).unwrap();
    jsonschema::draft4::new(&schema).expect("official SARIF schema compiles")
}

#[test]
fn sarif_output_validates_against_the_official_schema() {
    let validator = sarif_validator();
    let v = read("conformance/positions/vectors.json");
    for case in v["spans"].as_array().unwrap() {
        let text = v["documents"][case["document"].as_str().unwrap()]["text"]
            .as_str()
            .unwrap();
        let (s, e) = (&case["start"], &case["end"]);
        let coords = [
            num(&s["line"]),
            num(&s["utf32"]),
            num(&e["line"]),
            num(&e["utf32"]),
        ];
        let span = (num(&case["span"]["start"]), num(&case["span"]["end"]));
        let mut report = report_for(Some(span), coords, "msg");
        report.documents[0].findings[0].fix = Some(
            serde_json::from_value(
                json!({"edits": [{"span": {"start": span.0, "end": span.1}, "text": "X"}]}),
            )
            .unwrap(),
        );
        let sarif: Value = serde_json::from_str(&out(Format::Sarif, &report, text)).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&sarif)
            .map(|e| e.to_string())
            .collect();
        assert!(errors.is_empty(), "{}: {errors:?}", case["id"]);
    }
}

#[test]
fn sarif_with_notices_and_no_findings_validates() {
    let report: Report =
        serde_json::from_value(read("conformance/lint-report/valid/multi-document.json")).unwrap();
    let mut report = report;
    for doc in &mut report.documents {
        doc.findings.clear();
    }
    let sarif: Value =
        serde_json::from_str(&render(Format::Sarif, &report, &NoSources).unwrap()).unwrap();
    assert!(sarif_validator().is_valid(&sarif));
    let notes = &sarif["runs"][0]["invocations"][0]["toolExecutionNotifications"];
    assert!(notes.as_array().unwrap().len() >= 2);
}
