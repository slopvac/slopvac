#![allow(missing_docs, clippy::cast_possible_truncation)]

use serde_json::Value;
use slopvac_model::Span;
use slopvac_parse::{Projection, TokenKind};

fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}

fn kind(name: &str) -> TokenKind {
    match name {
        "identity" => TokenKind::Identity,
        "entity" => TokenKind::Entity,
        "backslash" => TokenKind::Backslash,
        "opaque" => TokenKind::Opaque,
        "collapse" => TokenKind::Collapse,
        "masked" => TokenKind::Masked,
        "wrap" => TokenKind::Wrap,
        other => panic!("unknown token kind {other}"),
    }
}

#[test]
fn projected_ranges_map_to_the_vector_spans() {
    let v: Value = serde_json::from_str(include_str!(
        "../../../schemas/v3/conformance/positions/vectors.json"
    ))
    .unwrap();
    for case in v["projections"].as_array().unwrap() {
        let mut p = Projection::new();
        for t in case["tokens"].as_array().unwrap() {
            let src = Span::new(n(&t["src"][0]), n(&t["src"][1])).unwrap();
            p.push(
                kind(t["kind"].as_str().unwrap()),
                t["text"].as_str().unwrap(),
                src,
            );
        }
        assert_eq!(
            p.text(),
            case["projected"].as_str().unwrap(),
            "{}",
            case["id"]
        );
        for r in case["ranges"].as_array().unwrap() {
            let got = p.span_for_range(n(&r["projected"][0]), n(&r["projected"][1]));
            let want = Span::new(n(&r["span"]["start"]), n(&r["span"]["end"])).unwrap();
            assert_eq!(got, want, "{} {}", case["id"], r["projected"]);
        }
    }
}

#[test]
fn byte_offsets_of_projected_text_map_like_code_points() {
    let mut p = Projection::new();
    p.push(TokenKind::Identity, "é", Span::new(0, 2).unwrap());
    p.push(TokenKind::Entity, "&", Span::new(2, 7).unwrap());
    p.push(TokenKind::Identity, "x", Span::new(7, 8).unwrap());
    // Projected text "é&x": bytes 2..3 is the entity.
    assert_eq!(p.span_for_bytes(2, 3), Span::new(2, 7).unwrap());
    assert_eq!(p.span_for_bytes(0, 4), Span::new(0, 8).unwrap());
}
