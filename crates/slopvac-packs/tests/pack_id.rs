//! The two pack id types accept and reject the same strings (`common.schema.json` `PackId`).

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use slopvac_model::PackId;
use slopvac_packs::{from_lock_id, to_lock_id};
use slopvac_plugins::lock::PackId as LockPackId;

fn accepted_by_both(text: &str) -> (bool, bool) {
    (
        PackId::new(text).is_ok(),
        text.parse::<LockPackId>().is_ok(),
    )
}

#[rstest]
#[case::core("core")]
#[case::one_letter("a")]
#[case::digits_and_dashes("acme-docs-2")]
fn both_types_accept(#[case] text: &str) {
    assert_eq!(accepted_by_both(text), (true, true));
}

#[rstest]
#[case::empty("")]
#[case::uppercase("Core")]
#[case::underscore("Core_Pack")]
#[case::leading_digit("1a")]
#[case::leading_dash("-a")]
#[case::space("a b")]
#[case::colon("a:b")]
#[case::non_ascii("café")]
fn both_types_reject(#[case] text: &str) {
    assert_eq!(accepted_by_both(text), (false, false));
}

#[test]
fn both_types_agree_on_the_common_corpus() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3/conformance/common");
    let mut seen = 0;
    for (dir, want) in [("valid", true), ("invalid", false)] {
        for entry in fs::read_dir(root.join(dir)).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if !name.starts_with("PackId--") {
                continue;
            }
            let text: String = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(accepted_by_both(&text), (want, want), "{name}");
            seen += 1;
        }
    }
    assert_ne!(seen, 0, "no PackId cases found");
}

#[test]
fn an_id_converts_both_ways_and_keeps_its_text() {
    let model = PackId::new("acme-docs").unwrap();
    let lock = to_lock_id(&model).unwrap();
    assert_eq!(lock.as_str(), "acme-docs");
    assert_eq!(from_lock_id(&lock).unwrap(), model);
}

#[test]
fn an_id_longer_than_the_lock_cap_does_not_convert() {
    let long = PackId::new("a".repeat(5000)).unwrap();
    assert!(to_lock_id(&long).is_err());
}
