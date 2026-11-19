//! Uniqueness checks on long lists stay linear.

use std::time::{Duration, Instant};

use slopvac_rules::UniqueVec;

fn json_list(count: usize) -> String {
    let items: Vec<String> = (0..count).map(|i| format!("\"w{i:07}\"")).collect();
    format!("[{}]", items.join(","))
}

/// A quadratic check makes about 2e10 comparisons for this list, minutes of work. The bound is
/// far above what a linear check needs, so it fails only on a regression to the quadratic one.
#[test]
fn a_long_list_of_distinct_items_is_checked_in_linear_time() {
    let text = json_list(200_000);
    let start = Instant::now();
    let list: UniqueVec<String, 1> = serde_json::from_str(&text).unwrap();
    assert_eq!(list.len(), 200_000);
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn a_repeat_at_the_end_of_a_long_list_names_its_index() {
    let mut text = json_list(50_000);
    text.truncate(text.len() - 1);
    text.push_str(",\"w0000007\"]");
    let err = serde_json::from_str::<UniqueVec<String, 1>>(&text).unwrap_err();
    assert!(
        err.to_string()
            .contains("items must be unique, item 50000 repeats an earlier one"),
        "{err}"
    );
}
