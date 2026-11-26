//! `Resolved::explain_grouped`: the paths of the entries it returns.

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

// `two` wants `k.a`, which `one` holds. Its fallback `k.b` is held by `three`, its own path
// `k.two` is shown by `four`, and the first numbered spelling `k.two#2` is shown by `five`. The
// next numbered spelling, `k.two#3`, is free, so `two` prints there and all five entries stay.
#[test]
fn the_next_numbered_spelling_is_used_when_the_fallback_group_path_and_first_number_are_taken() {
    let names = ["one", "two", "three", "four", "five"];
    let resolved = Resolved::from_layers([names
        .iter()
        .zip(1..)
        .map(|(n, v)| Setting::bare(ConfigLayer::Project, &["k", n], json!(v)))
        .collect()]);
    let identify = |p: &[String], _: &slopvac_config::Contribution| match p[1].as_str() {
        "one" => GroupedKey::new(path(&["k", "one"]), path(&["k", "a"]), 0),
        "two" => GroupedKey::new(path(&["k", "two"]), path(&["k", "a"]), 0)
            .with_fallback(path(&["k", "b"])),
        "three" => GroupedKey::new(path(&["k", "three"]), path(&["k", "b"]), 0),
        "four" => GroupedKey::new(path(&["k", "four"]), path(&["k", "two"]), 0),
        _ => GroupedKey::new(path(&["k", "five"]), path(&["k", "two#2"]), 0),
    };

    let mut shown: Vec<(String, i64)> = resolved
        .explain_grouped(identify)
        .iter()
        .map(|e| (e.path.clone(), e.value.as_i64().expect("a number")))
        .collect();
    shown.sort();

    let expected: Vec<(String, i64)> = [
        ("k.a", 1),
        ("k.b", 3),
        ("k.two", 4),
        ("k.two#2", 5),
        ("k.two#3", 2),
    ]
    .iter()
    .map(|(p, v)| ((*p).to_owned(), *v))
    .collect();
    assert_eq!(shown, expected);
}
