#![allow(missing_docs, clippy::cast_possible_truncation)]

use serde_json::Value;
use slopvac_model::{
    ColumnUnit, Edit, Finding, LineCol, PackId, PositionEncoding, RuleId, Severity, SourceIndex,
    Span, apply_edits,
};

fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../../schemas/v3/conformance/positions/vectors.json"
    ))
    .unwrap()
}

fn doc(v: &Value, name: &str) -> String {
    v["documents"][name]["text"].as_str().unwrap().to_owned()
}

fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}

fn span(v: &Value) -> Span {
    Span::new(n(&v["start"]), n(&v["end"])).unwrap()
}

const UNITS: [(&str, ColumnUnit); 3] = [
    ("utf8", ColumnUnit::Utf8),
    ("utf16", ColumnUnit::Utf16),
    ("utf32", ColumnUnit::CodePoint),
];

#[test]
fn positions_match_vectors_in_every_unit() {
    let v = vectors();
    for p in v["positions"].as_array().unwrap() {
        let text = doc(&v, p["document"].as_str().unwrap());
        let idx = SourceIndex::disk(&text);
        let off = n(&p["offset"]);
        for (key, unit) in UNITS {
            let got = idx.line_col_in(off, unit).unwrap();
            assert_eq!(
                (got.line(), got.column()),
                (n(&p["line"]), n(&p[key])),
                "{p}"
            );
        }
    }
}

#[test]
fn invalid_offsets_are_rejected() {
    let v = vectors();
    for p in v["invalid_offsets"].as_array().unwrap() {
        let Some(off) = p["offset"].as_u64() else {
            continue;
        };
        let text = doc(&v, p["document"].as_str().unwrap());
        let idx = SourceIndex::disk(&text);
        assert!(idx.check_offset(off as usize).is_err(), "{p}");
    }
}

#[test]
fn spans_match_vectors() {
    let v = vectors();
    for s in v["spans"].as_array().unwrap() {
        let text = doc(&v, s["document"].as_str().unwrap());
        let idx = SourceIndex::disk(&text);
        let sp = span(&s["span"]);
        for (key, unit) in UNITS {
            let (a, b) = idx.locate_span(sp, unit).unwrap();
            let id = &s["id"];
            assert_eq!(
                (a.line(), a.column()),
                (n(&s["start"]["line"]), n(&s["start"][key])),
                "{id} start {key}"
            );
            assert_eq!(
                (b.line(), b.column()),
                (n(&s["end"]["line"]), n(&s["end"][key])),
                "{id} end {key}"
            );
        }
    }
}

#[test]
fn code_climate_inclusive_end_matches() {
    let v = vectors();
    for s in v["spans"].as_array().unwrap() {
        let text = doc(&v, s["document"].as_str().unwrap());
        let idx = SourceIndex::disk(&text);
        let cc = &s["formats"]["codeclimate"];
        let last = idx
            .locate_last_included(span(&s["span"]), ColumnUnit::CodePoint)
            .unwrap();
        let id = &s["id"];
        assert_eq!(last.line(), n(&cc["lines"]["end"]), "{id}");
        assert_eq!(last.column(), n(&cc["positions"]["end"]["column"]), "{id}");
    }
}

#[test]
fn invalid_spans_are_rejected_by_construction_or_by_the_file() {
    let v = vectors();
    for s in v["invalid_spans"].as_array().unwrap() {
        let text = doc(&v, s["document"].as_str().unwrap());
        let (start, end) = (n(&s["span"]["start"]), n(&s["span"]["end"]));
        let rejected = match Span::new(start, end) {
            Err(_) => true,
            Ok(sp) => SourceIndex::disk(&text).check_span(sp).is_err(),
        };
        assert!(rejected, "{}", s["id"]);
    }
}

#[test]
fn lsp_ranges_match_per_encoding_over_buffer_text() {
    let v = vectors();
    for c in v["lsp_ranges"].as_array().unwrap() {
        let text = doc(&v, c["document"].as_str().unwrap());
        let idx = SourceIndex::buffer(&text);
        let sp = span(&c["span"]);
        for (name, enc) in [
            ("utf-8", PositionEncoding::Utf8),
            ("utf-16", PositionEncoding::Utf16),
            ("utf-32", PositionEncoding::Utf32),
        ] {
            let want = &c["expected"][name];
            let pair = |p: &Value| (n(&p["line"]), n(&p["character"]));
            let id = &c["id"];
            assert_eq!(
                idx.lsp_position(sp.start(), enc).unwrap(),
                pair(&want["start"]),
                "{id} {name}"
            );
            assert_eq!(
                idx.lsp_position(sp.end(), enc).unwrap(),
                pair(&want["end"]),
                "{id} {name}"
            );
        }
    }
}

#[test]
fn encoding_negotiation_matches_vectors() {
    let v = vectors();
    for c in v["lsp_negotiation"].as_array().unwrap() {
        let offered: Option<Vec<&str>> = c["client"]
            .as_array()
            .map(|a| a.iter().map(|x| x.as_str().unwrap()).collect());
        let got = PositionEncoding::negotiate(offered.as_deref());
        assert_eq!(got.name(), c["server"].as_str().unwrap(), "{c}");
    }
}

#[test]
fn buffer_mode_counts_the_bom_but_disk_mode_does_not() {
    let text = "\u{feff}Hi";
    let disk = SourceIndex::disk(text);
    let buffer = SourceIndex::buffer(text);
    assert_eq!(disk.line_col(3).unwrap().column(), 1);
    assert_eq!(buffer.line_col(3).unwrap().column(), 2);
    assert!(disk.check_offset(1).is_err());
}

fn edits(v: &Value) -> Vec<Edit> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|e| Edit {
            span: span(&e["span"]),
            text: e["text"].as_str().unwrap().to_owned(),
        })
        .collect()
}

#[test]
fn edits_produce_vector_results() {
    let v = vectors();
    for e in v["edits"].as_array().unwrap() {
        let text = doc(&v, e["document"].as_str().unwrap());
        let got = apply_edits(&text, &edits(&e["edits"])).unwrap();
        assert_eq!(got, e["result"].as_str().unwrap(), "{}", e["id"]);
    }
}

#[test]
fn invalid_edit_sets_are_rejected() {
    let v = vectors();
    for e in v["invalid_edits"].as_array().unwrap() {
        let text = doc(&v, e["document"].as_str().unwrap());
        assert!(
            apply_edits(&text, &edits(&e["edits"])).is_err(),
            "{}",
            e["id"]
        );
    }
}

#[test]
fn reversed_span_is_rejected_everywhere() {
    assert!(Span::new(3, 2).is_err());
    assert!(serde_json::from_str::<Span>(r#"{"start":3,"end":2}"#).is_err());
    assert_eq!(
        serde_json::from_str::<Span>(r#"{"start":2,"end":2}"#).unwrap(),
        Span::empty(2)
    );
}

#[test]
fn zero_line_or_column_is_rejected_everywhere() {
    assert!(LineCol::new(0, 1).is_err() && LineCol::new(1, 0).is_err());
    assert!(serde_json::from_str::<LineCol>(r#"{"line":0,"column":1}"#).is_err());
    assert!(serde_json::from_str::<LineCol>(r#"{"line":1,"column":0}"#).is_err());
    assert_eq!(LineCol::new(2, 3).unwrap().column(), 3);
}

#[test]
fn rule_id_accepts_and_rejects_per_schema() {
    for ok in ["slop:no-hedge", "a1:B.c_d-e"] {
        assert!(RuleId::new(ok).is_ok(), "{ok}");
    }
    for bad in ["nohedge", "Slop:x", ":x", "slop:", "slop:-x", "slop:a b"] {
        assert!(RuleId::new(bad).is_err(), "{bad}");
    }
}

#[test]
fn rule_id_splits_pack_and_local() {
    let id = RuleId::new("slop:no-hedge").unwrap();
    assert_eq!((id.pack(), id.local()), ("slop", "no-hedge"));
}

#[test]
fn severity_has_no_off_and_orders_by_weight() {
    assert!(serde_json::from_str::<Severity>("\"off\"").is_err());
    assert!(Severity::Error > Severity::Warning && Severity::Warning > Severity::Suggestion);
}

#[test]
fn consumer_side_serde_ignores_unknown_fields() {
    let json = r#"{"start":1,"end":2,"x-extra":true,"surprise":1}"#;
    assert_eq!(
        serde_json::from_str::<Span>(json).unwrap(),
        Span::new(1, 2).unwrap()
    );
}

#[test]
fn finding_without_span_omits_span_and_end() {
    let f = Finding {
        rule_id: RuleId::new("slop:x").unwrap(),
        pack: PackId::new("slop").unwrap(),
        category: "metrics".into(),
        severity: Severity::Warning,
        message: "m".into(),
        start: LineCol::new(1, 1).unwrap(),
        extent: None,
        contexts: slopvac_model::Contexts::new(vec![
            slopvac_model::ContextTag::new("prose").unwrap(),
        ])
        .unwrap(),
        ai_signal: slopvac_model::AiSignal::None,
        matched_text: String::new(),
        fix: None,
        provenance: None,
    };
    let json = serde_json::to_value(&f).unwrap();
    assert!(json.get("span").is_none() && json.get("end_line").is_none());
    assert_eq!(json["ai_signal"], "none");
    assert_eq!(serde_json::from_value::<Finding>(json).unwrap(), f);
}
