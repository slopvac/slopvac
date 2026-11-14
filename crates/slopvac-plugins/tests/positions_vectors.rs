//! Fixer-edit validation against the shared position vectors in
//! `schemas/v3/conformance/positions` (docs/contract/positions.md).

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use slopvac_plugins::hooks::{Finding, Span, parse_edits};

struct Vectors {
    root: Value,
}

impl Vectors {
    fn load() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../schemas/v3/conformance/positions/vectors.json");
        Self {
            root: serde_json::from_slice(&fs::read(path).unwrap()).unwrap(),
        }
    }

    fn texts(&self) -> HashMap<String, String> {
        self.root["documents"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v["text"].as_str().unwrap().to_owned()))
            .collect()
    }

    fn list(&self, key: &str) -> &Vec<Value> {
        let list = self.root[key].as_array().unwrap();
        assert!(!list.is_empty(), "no {key} vectors");
        list
    }
}

/// Run the edits through the public parser against a finding that spans the whole document.
fn check(text: &str, edits: &Value) -> bool {
    let findings = [Finding {
        rule: "core:x".into(),
        severity: "warning".into(),
        span: Span {
            start: 0,
            end: text.len() as u64,
        },
        message: "m".into(),
    }];
    let edits: Vec<Value> = edits
        .as_array()
        .unwrap()
        .iter()
        .map(|e| json!({"finding": 0, "span": e["span"], "text": e["text"]}))
        .collect();
    parse_edits(&json!({ "edits": edits }), text, &findings).is_ok()
}

fn insert_at(offset: &Value) -> Value {
    json!([{"span": {"start": offset, "end": offset}, "text": "x"}])
}

#[test]
fn every_valid_edit_vector_is_accepted() {
    let v = Vectors::load();
    let texts = v.texts();
    for e in v.list("edits") {
        let doc = e["document"].as_str().unwrap();
        assert!(check(&texts[doc], &e["edits"]), "{}", e["id"]);
    }
}

#[test]
fn every_invalid_edit_vector_is_rejected() {
    let v = Vectors::load();
    let texts = v.texts();
    for e in v.list("invalid_edits") {
        let doc = e["document"].as_str().unwrap();
        assert!(!check(&texts[doc], &e["edits"]), "{}", e["id"]);
    }
}

#[test]
fn every_valid_span_vector_is_accepted_as_an_edit_span() {
    let v = Vectors::load();
    let texts = v.texts();
    for s in v.list("spans") {
        let doc = s["document"].as_str().unwrap();
        let edits = json!([{"span": s["span"], "text": "x"}]);
        assert!(check(&texts[doc], &edits), "{}", s["id"]);
    }
}

#[test]
fn every_invalid_span_vector_is_rejected_as_an_edit_span() {
    let v = Vectors::load();
    let texts = v.texts();
    for s in v.list("invalid_spans") {
        let doc = s["document"].as_str().unwrap();
        let edits = json!([{"span": s["span"], "text": "x"}]);
        assert!(!check(&texts[doc], &edits), "{}", s["id"]);
    }
}

#[test]
fn every_valid_position_offset_is_accepted_as_an_insertion_point() {
    let v = Vectors::load();
    let texts = v.texts();
    for p in v.list("positions") {
        let doc = p["document"].as_str().unwrap();
        assert!(
            check(&texts[doc], &insert_at(&p["offset"])),
            "{doc} {}",
            p["offset"]
        );
    }
}

#[test]
fn every_invalid_offset_is_rejected_as_an_insertion_point() {
    let v = Vectors::load();
    let texts = v.texts();
    for o in v.list("invalid_offsets") {
        let doc = o["document"].as_str().unwrap();
        assert!(
            !check(&texts[doc], &insert_at(&o["offset"])),
            "{doc} {}: {}",
            o["offset"],
            o["reason"]
        );
    }
}
