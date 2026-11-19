//! A repeat at the end of a long list is still reported by index.

use slopvac_rules::UniqueVec;

fn json_list(count: usize) -> String {
    let items: Vec<String> = (0..count).map(|i| format!("\"w{i:07}\"")).collect();
    format!("[{}]", items.join(","))
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
