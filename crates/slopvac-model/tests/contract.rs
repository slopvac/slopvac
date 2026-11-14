#![allow(missing_docs)]

use serde_json::Value;
use slopvac_model::{
    ColumnUnit, Edit, Finding, PackId, RuleId, Severity, SourceIndex, Span, apply_edits,
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
    usize::try_from(v.as_u64().unwrap()).unwrap()
}

fn span(v: &Value) -> Span {
    Span::new(n(&v["start"]), n(&v["end"]))
}

#[test]
fn positions_match_vectors_in_every_unit() {
    let v = vectors();
    for p in v["positions"].as_array().unwrap() {
        let text = doc(&v, p["document"].as_str().unwrap());
        let idx = SourceIndex::new(&text);
        let off = n(&p["offset"]);
        for (key, unit) in [
            ("utf8", ColumnUnit::Utf8),
            ("utf16", ColumnUnit::Utf16),
            ("utf32", ColumnUnit::CodePoint),
        ] {
            let got = idx.line_col_in(off, unit).unwrap();
            assert_eq!((got.line, got.column), (n(&p["line"]), n(&p[key])), "{p}");
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
        let idx = SourceIndex::new(&text);
        assert!(
            idx.check_offset(usize::try_from(off).unwrap()).is_err(),
            "{p}"
        );
    }
}

#[test]
fn spans_match_vectors() {
    let v = vectors();
    for s in v["spans"].as_array().unwrap() {
        let text = doc(&v, s["document"].as_str().unwrap());
        let idx = SourceIndex::new(&text);
        let sp = span(&s["span"]);
        idx.check_span(sp).unwrap();
        for (end, off) in [("start", sp.start), ("end", sp.end)] {
            for (key, unit) in [
                ("utf8", ColumnUnit::Utf8),
                ("utf16", ColumnUnit::Utf16),
                ("utf32", ColumnUnit::CodePoint),
            ] {
                let got = idx.line_col_in(off, unit).unwrap();
                assert_eq!(
                    (got.line, got.column),
                    (n(&s[end]["line"]), n(&s[end][key])),
                    "{} {end} {key}",
                    s["id"]
                );
            }
        }
    }
}

#[test]
fn invalid_spans_are_rejected() {
    let v = vectors();
    for s in v["invalid_spans"].as_array().unwrap() {
        let text = doc(&v, s["document"].as_str().unwrap());
        assert!(
            SourceIndex::new(&text)
                .check_span(span(&s["span"]))
                .is_err(),
            "{}",
            s["id"]
        );
    }
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
fn finding_without_span_omits_span_and_end() {
    let f = Finding {
        rule_id: RuleId::new("slop:x").unwrap(),
        pack: PackId::new("slop").unwrap(),
        category: "metrics".into(),
        severity: Severity::Warning,
        message: "m".into(),
        span: None,
        line: 1,
        column: 1,
        end_line: None,
        end_column: None,
        contexts: vec!["prose".into()],
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
