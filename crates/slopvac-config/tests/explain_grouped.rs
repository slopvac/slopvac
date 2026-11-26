//! `Resolved::explain_grouped`: the paths of the entries it returns.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a panic is a failed test"
)]

use serde_json::json;
use slopvac_config::{ConfigLayer, GroupedKey, Resolved, Setting};

fn path(segments: &[&str]) -> Vec<String> {
    segments.iter().map(|s| (*s).to_owned()).collect()
}

// Three groups. `one` and `two` both ask to show as `k.a`, and `three` already shows as `k.b`.
// `two` has `k.b` as its fallback, which `three` holds. The three entries print under three
// different paths, and the result is the same on every call.
#[test]
fn a_fallback_that_another_entry_already_shows_never_prints_two_entries_under_one_path() {
    let resolved = Resolved::from_layers([vec![
        Setting::bare(ConfigLayer::Project, &["k", "one"], json!(1)),
        Setting::bare(ConfigLayer::Project, &["k", "two"], json!(2)),
        Setting::bare(ConfigLayer::Project, &["k", "three"], json!(3)),
    ]]);
    let identify = |p: &[String], _: &slopvac_config::Contribution| match p[1].as_str() {
        "one" => GroupedKey::new(path(&["k", "one"]), path(&["k", "a"]), 0),
        "two" => GroupedKey::new(path(&["k", "two"]), path(&["k", "a"]), 0)
            .with_fallback(path(&["k", "b"])),
        _ => GroupedKey::new(path(&["k", "three"]), path(&["k", "b"]), 0),
    };

    let first = resolved.explain_grouped(identify);
    let second = resolved.explain_grouped(identify);

    let shown: Vec<(&str, i64)> = first
        .iter()
        .map(|e| (e.path.as_str(), e.value.as_i64().expect("a number")))
        .collect();
    let mut paths: Vec<&str> = shown.iter().map(|(p, _)| *p).collect();
    paths.sort_unstable();
    paths.dedup();
    assert_eq!(paths.len(), 3, "{shown:?}");
    assert!(shown.contains(&("k.a", 1)), "{shown:?}");
    assert!(shown.contains(&("k.b", 3)), "{shown:?}");
    assert!(shown.contains(&("k.two", 2)), "{shown:?}");
    assert_eq!(first, second);
}
