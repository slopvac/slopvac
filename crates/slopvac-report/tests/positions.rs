//! Runs the shared position vectors (schemas/v3/conformance/positions).

use serde_json::Value;
use slopvac_report::positions::{BomMode, ByteSpan, LineCol, SourceMap, Unit, negotiate_encoding};

const UNITS: [(&str, Unit); 3] = [
    ("utf8", Unit::Utf8),
    ("utf16", Unit::Utf16),
    ("utf32", Unit::Utf32),
];

fn vectors() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../schemas/v3/conformance/positions/vectors.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("vectors file")).expect("json")
}

fn map<'v>(v: &'v Value, case: &Value, mode: BomMode) -> SourceMap<'v> {
    let name = case["document"].as_str().expect("document name");
    SourceMap::new(v["documents"][name]["text"].as_str().expect("text"), mode)
}

fn num(value: &Value) -> u32 {
    u32::try_from(value.as_u64().expect("unsigned int")).expect("fits u32")
}

fn span(value: &Value) -> ByteSpan {
    ByteSpan {
        start: usize::try_from(value["start"].as_u64().expect("start")).expect("usize"),
        end: usize::try_from(value["end"].as_u64().expect("end")).expect("usize"),
    }
}

#[test]
fn positions_match_in_every_unit() {
    let v = vectors();
    for case in v["positions"].as_array().unwrap() {
        let map = map(&v, case, BomMode::Disk);
        let offset = usize::try_from(case["offset"].as_u64().unwrap()).unwrap();
        for (key, unit) in UNITS {
            let want = LineCol {
                line: num(&case["line"]),
                column: num(&case[key]),
            };
            assert_eq!(map.locate(offset, unit).unwrap(), want, "{case} in {key}");
        }
    }
}

#[test]
fn invalid_offsets_are_rejected() {
    let v = vectors();
    for case in v["invalid_offsets"].as_array().unwrap() {
        let map = map(&v, case, BomMode::Disk);
        let Some(offset) = case["offset"].as_u64() else {
            continue;
        };
        let offset = usize::try_from(offset).unwrap();
        assert!(map.check_offset(offset).is_err(), "{case}");
    }
}

#[test]
fn invalid_spans_are_rejected() {
    let v = vectors();
    for case in v["invalid_spans"].as_array().unwrap() {
        let map = map(&v, case, BomMode::Disk);
        assert!(
            map.check_span(span(&case["span"])).is_err(),
            "{}",
            case["id"]
        );
    }
}

#[test]
fn span_ends_match_in_every_unit() {
    let v = vectors();
    for case in v["spans"].as_array().unwrap() {
        let map = map(&v, case, BomMode::Disk);
        for (key, unit) in UNITS {
            let (s, e) = map.locate_span(span(&case["span"]), unit).unwrap();
            let id = &case["id"];
            assert_eq!(s.line, num(&case["start"]["line"]), "{id}");
            assert_eq!(s.column, num(&case["start"][key]), "{id} {key}");
            assert_eq!(e.line, num(&case["end"]["line"]), "{id}");
            assert_eq!(e.column, num(&case["end"][key]), "{id} {key}");
        }
    }
}

#[test]
fn code_climate_inclusive_end_matches() {
    let v = vectors();
    for case in v["spans"].as_array().unwrap() {
        let map = map(&v, case, BomMode::Disk);
        let cc = &case["formats"]["codeclimate"];
        let last = map
            .locate_last_included(span(&case["span"]), Unit::Utf32)
            .unwrap();
        let id = &case["id"];
        assert_eq!(last.line, num(&cc["lines"]["end"]), "{id}");
        assert_eq!(last.line, num(&cc["positions"]["end"]["line"]), "{id}");
        assert_eq!(last.column, num(&cc["positions"]["end"]["column"]), "{id}");
    }
}

#[test]
fn lsp_ranges_match_per_encoding() {
    let v = vectors();
    for case in v["lsp_ranges"].as_array().unwrap() {
        let map = map(&v, case, BomMode::Buffer);
        let sp = span(&case["span"]);
        for (name, unit) in [
            ("utf-8", Unit::Utf8),
            ("utf-16", Unit::Utf16),
            ("utf-32", Unit::Utf32),
        ] {
            let want = &case["expected"][name];
            let pair = |p: &Value| (num(&p["line"]), num(&p["character"]));
            let id = &case["id"];
            assert_eq!(
                map.lsp_position(sp.start, unit).unwrap(),
                pair(&want["start"]),
                "{id} {name}"
            );
            assert_eq!(
                map.lsp_position(sp.end, unit).unwrap(),
                pair(&want["end"]),
                "{id} {name}"
            );
        }
    }
}

#[test]
fn encoding_negotiation_matches() {
    let v = vectors();
    for case in v["lsp_negotiation"].as_array().unwrap() {
        let offered: Option<Vec<&str>> = case["client"]
            .as_array()
            .map(|a| a.iter().map(|x| x.as_str().unwrap()).collect());
        let (name, _) = negotiate_encoding(offered.as_deref());
        assert_eq!(name, case["server"].as_str().unwrap(), "{case}");
    }
}
