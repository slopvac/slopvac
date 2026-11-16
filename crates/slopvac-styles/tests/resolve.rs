//! Behavior of style resolution through the public API.

use serde_json::{Value, json};
use slopvac_config::resolve::Contribution;
use slopvac_styles::{MapSource, Resolution, StyleError, resolve};

fn get<'a>(r: &'a Resolution, path: &[&str]) -> &'a Value {
    r.resolved
        .get(path)
        .unwrap_or_else(|| panic!("no value at {path:?}"))
}

fn history(r: &Resolution, path: &[&str]) -> Vec<(String, Value)> {
    r.resolved
        .history(path)
        .iter()
        .map(|c: &Contribution| (c.source.file.clone().unwrap_or_default(), c.value.clone()))
        .collect()
}

fn names(n: &[&str]) -> Vec<String> {
    n.iter().map(|s| (*s).to_owned()).collect()
}

const BASE: &str = r#"
schema_version = 3
[rules]
"core:a" = "warning"
"core:b" = "error"
[categories]
ste-words = "error"
[categories.ste-nouns]
severity = "warning"
weight = 2.0
"#;

#[test]
fn last_listed_style_wins_per_rule() {
    let src = MapSource::new()
        .with("one", BASE)
        .with("two", "schema_version = 3\n[rules]\n\"core:a\" = \"off\"\n");
    let r = resolve(&names(&["one", "two"]), &src, false).unwrap();
    assert_eq!(get(&r, &["rules", "core:a"]), &json!("off"));
    assert_eq!(get(&r, &["rules", "core:b"]), &json!("error"));
}

#[test]
fn listing_order_decides_the_winner() {
    let src = MapSource::new()
        .with("one", BASE)
        .with("two", "schema_version = 3\n[rules]\n\"core:a\" = \"off\"\n");
    let r = resolve(&names(&["two", "one"]), &src, false).unwrap();
    assert_eq!(get(&r, &["rules", "core:a"]), &json!("warning"));
}

#[test]
fn extends_resolve_before_the_style_that_names_them() {
    let src = MapSource::new().with("parent", BASE).with(
        "child",
        "schema_version = 3\nextends = [\"parent\"]\n[rules]\n\"core:a\" = \"suggestion\"\n",
    );
    let r = resolve(&names(&["child"]), &src, false).unwrap();
    assert_eq!(r.order, ["parent", "child"]);
    assert_eq!(get(&r, &["rules", "core:a"]), &json!("suggestion"));
}

#[test]
fn a_shared_parent_applies_once() {
    let child = |p: &str| format!("schema_version = 3\nextends = [\"{p}\"]\n");
    let src = MapSource::new()
        .with("root", BASE)
        .with("left", &child("root"))
        .with("right", &child("root"));
    let r = resolve(&names(&["left", "right"]), &src, false).unwrap();
    assert_eq!(r.order, ["root", "left", "right"]);
}

#[test]
fn extends_cycle_is_an_error() {
    let ext = |p: &str| format!("schema_version = 3\nextends = [\"{p}\"]\n");
    let src = MapSource::new().with("a", &ext("b")).with("b", &ext("a"));
    let e = resolve(&names(&["a"]), &src, false).unwrap_err();
    assert_eq!(e, StyleError::Cycle(names(&["a", "b", "a"])));
}

#[test]
fn category_dials_merge_per_field() {
    let src = MapSource::new().with("one", BASE).with(
        "two",
        "schema_version = 3\n[categories.ste-nouns]\nweight = 0.5\n",
    );
    let r = resolve(&names(&["one", "two"]), &src, false).unwrap();
    // `two` sets only the weight, so severity from `one` survives and the
    // weight is replaced; ste-words is untouched by `two`.
    assert_eq!(
        get(&r, &["categories", "ste-nouns", "severity"]),
        &json!("warning")
    );
    assert_eq!(get(&r, &["categories", "ste-nouns", "weight"]), &json!(0.5));
    assert_eq!(
        get(&r, &["categories", "ste-words", "severity"]),
        &json!("error")
    );
    // Nothing the styles left unset appears.
    for (cat, field) in [
        ("ste-nouns", "minimum_severity"),
        ("ste-nouns", "max_per_100_words"),
        ("ste-words", "minimum_severity"),
        ("ste-words", "max_per_100_words"),
        ("ste-words", "weight"),
    ] {
        assert_eq!(
            r.resolved.get(&["categories", cat, field]),
            None,
            "{cat}.{field}"
        );
    }
    assert_eq!(history(&r, &["categories", "ste-nouns", "weight"]).len(), 2);
    assert_eq!(
        history(&r, &["categories", "ste-nouns", "severity"]).len(),
        1
    );
}

#[test]
fn explicit_listing_beats_ancestor_position() {
    let src = MapSource::new().with("parent", BASE).with(
        "child",
        "schema_version = 3\nextends = [\"parent\"]\n[rules]\n\"core:a\" = \"suggestion\"\n",
    );
    let r = resolve(&names(&["child", "parent"]), &src, false).unwrap();
    assert_eq!(r.order, ["child", "parent"]);
    assert_eq!(get(&r, &["rules", "core:a"]), &json!("warning"));
}

#[test]
fn blocklists_replace_rather_than_append() {
    let a = "schema_version = 3\n[terms]\nblocklists = [\"one.toml\", \"two.toml\"]\n";
    let b = "schema_version = 3\n[terms]\nblocklists = [\"three.toml\"]\n";
    let src = MapSource::new().with("a", a).with("b", b);
    let r = resolve(&names(&["a", "b"]), &src, false).unwrap();
    assert_eq!(get(&r, &["terms", "blocklists"]), &json!(["three.toml"]));
    let r = resolve(&names(&["b", "a"]), &src, false).unwrap();
    assert_eq!(
        get(&r, &["terms", "blocklists"]),
        &json!(["one.toml", "two.toml"])
    );
}

#[test]
fn list_conflicts_compare_items_not_joined_text() {
    let a = "schema_version = 3\nconflicts = [\"b\"]\n[terms]\nallow = [\"a,b\"]\n";
    let b = "schema_version = 3\n[terms]\nallow = [\"a\", \"b\"]\n";
    let src = MapSource::new().with("a", a).with("b", b);
    let r = resolve(&names(&["a", "b"]), &src, false).unwrap();
    assert_eq!(r.conflicts[0].keys, ["terms.allow"]);
}

#[test]
fn bare_category_level_is_severity() {
    let src = MapSource::new().with("one", BASE);
    let r = resolve(&names(&["one"]), &src, false).unwrap();
    assert_eq!(
        history(&r, &["categories", "ste-words", "severity"])[0].1,
        json!("error")
    );
}

#[test]
fn history_records_each_style_in_order() {
    let src = MapSource::new()
        .with("one", BASE)
        .with("two", "schema_version = 3\n[rules]\n\"core:a\" = \"off\"\n");
    let r = resolve(&names(&["one", "two"]), &src, false).unwrap();
    assert_eq!(
        history(&r, &["rules", "core:a"]),
        [
            ("preset:one".to_owned(), json!("warning")),
            ("preset:two".to_owned(), json!("off"))
        ]
    );
}

#[test]
fn terms_lists_replace() {
    let a = "schema_version = 3\n[terms]\nallow = [\"x\", \"y\"]\n";
    let b = "schema_version = 3\n[terms]\nallow = [\"z\"]\n";
    let src = MapSource::new().with("a", a).with("b", b);
    let r = resolve(&names(&["a", "b"]), &src, false).unwrap();
    assert_eq!(get(&r, &["terms", "allow"]), &json!(["z"]));
}

fn conflicting() -> MapSource {
    MapSource::new()
        .with(
            "ste",
            "schema_version = 3\nconflicts = [\"google\"]\n[rules]\n\"core:list\" = \"error\"\n",
        )
        .with(
            "google",
            "schema_version = 3\n[rules]\n\"core:list\" = \"off\"\n\"core:other\" = \"warning\"\n",
        )
}

#[test]
fn declared_conflict_names_the_later_style_as_winner() {
    let r = resolve(&names(&["ste", "google"]), &conflicting(), false).unwrap();
    assert_eq!(r.conflicts.len(), 1);
    let c = &r.conflicts[0];
    assert_eq!((c.loser.as_str(), c.winner.as_str()), ("ste", "google"));
    assert_eq!(c.keys, ["rules.core:list"]);
    assert_eq!(
        c.message(),
        "style `ste` conflicts with `google`; `google` wins on rules.core:list"
    );
}

#[test]
fn conflict_flips_with_order() {
    let r = resolve(&names(&["google", "ste"]), &conflicting(), false).unwrap();
    assert_eq!(r.conflicts[0].winner, "ste");
}

#[test]
fn strict_styles_turns_a_conflict_into_an_error() {
    let e = resolve(&names(&["ste", "google"]), &conflicting(), true).unwrap_err();
    assert_eq!(
        e,
        StyleError::Conflict {
            loser: "ste".into(),
            winner: "google".into()
        }
    );
}

#[test]
fn no_conflict_when_only_one_side_is_active() {
    let r = resolve(&names(&["ste"]), &conflicting(), true).unwrap();
    assert_eq!(r.conflicts.len(), 0);
}

#[test]
fn unknown_style_is_not_found() {
    let e = resolve(&names(&["nope"]), &MapSource::new(), false).unwrap_err();
    assert_eq!(e, StyleError::NotFound("nope".into()));
}

#[test]
fn unknown_key_is_an_error_but_x_keys_are_ignored() {
    let src = MapSource::new()
        .with("bad", "schema_version = 3\nfoo = 1\n")
        .with("ok", "schema_version = 3\nx-note = \"hi\"\n");
    let StyleError::Invalid { problems, .. } = resolve(&names(&["bad"]), &src, false).unwrap_err()
    else {
        panic!("expected an invalid-style error");
    };
    assert_eq!(
        problems,
        [("config-unknown-key".to_owned(), "/foo".to_owned())]
    );
    assert!(resolve(&names(&["ok"]), &src, false).is_ok());
}

#[test]
fn wrong_schema_version_is_rejected() {
    let src = MapSource::new().with("old", "schema_version = 2\n");
    let StyleError::Invalid { problems, .. } = resolve(&names(&["old"]), &src, false).unwrap_err()
    else {
        panic!("expected an invalid-style error");
    };
    assert_eq!(
        problems,
        [(
            "config-legacy-version".to_owned(),
            "/schema_version".to_owned()
        )]
    );
}

const FLOORED: &str =
    "schema_version = 3\n[categories.c]\nminimum_severity = \"warning\"\nmax_per_100_words = 2.5\n";

#[test]
fn minimum_severity_off_cancels_a_lower_floor() {
    let off = "schema_version = 3\n[categories.c]\nminimum_severity = \"off\"\n";
    let src = MapSource::new().with("floor", FLOORED).with("clear", off);
    let r = resolve(&names(&["floor", "clear"]), &src, false).unwrap();
    assert_eq!(
        get(&r, &["categories", "c", "minimum_severity"]),
        &json!("off")
    );
    assert_eq!(
        history(&r, &["categories", "c", "minimum_severity"])
            .into_iter()
            .map(|h| h.1)
            .collect::<Vec<_>>(),
        [json!("warning"), json!("off")]
    );
    let r = resolve(&names(&["clear", "floor"]), &src, false).unwrap();
    assert_eq!(
        get(&r, &["categories", "c", "minimum_severity"]),
        &json!("warning")
    );
}

#[test]
fn budget_none_clears_the_lower_limit_and_is_not_missing() {
    let none = "schema_version = 3\n[categories.c]\nmax_per_100_words = \"none\"\n";
    let untouched = "schema_version = 3\n[categories.c]\nweight = 1.0\n";
    let src = MapSource::new()
        .with("floor", FLOORED)
        .with("none", none)
        .with("other", untouched);
    let budget = ["categories", "c", "max_per_100_words"];
    let r = resolve(&names(&["floor", "none"]), &src, false).unwrap();
    // A budget of "none" is stored as null: set, and no limit.
    assert_eq!(get(&r, &budget), &Value::Null);
    assert_eq!(r.resolved.budget("c"), None);
    // A layer that does not mention the budget leaves the limit alone.
    let r = resolve(&names(&["floor", "other"]), &src, false).unwrap();
    assert_eq!(r.resolved.budget("c"), Some(2.5));
    // A later limit replaces an earlier "none".
    let r = resolve(&names(&["none", "floor"]), &src, false).unwrap();
    assert_eq!(r.resolved.budget("c"), Some(2.5));
    let r = resolve(&names(&["floor", "none"]), &src, false).unwrap();
    assert_eq!(
        history(&r, &budget)
            .into_iter()
            .map(|h| h.1)
            .collect::<Vec<_>>(),
        [json!(2.5), Value::Null]
    );
}

#[test]
fn budget_rejects_other_strings_and_negative_numbers() {
    let cases = [
        ("max_per_100_words = \"unlimited\"", "max_per_100_words"),
        ("max_per_100_words = -1", "max_per_100_words"),
        ("minimum_severity = \"fatal\"", "minimum_severity"),
    ];
    for (line, field) in cases {
        let text = format!("schema_version = 3\n[categories.c]\n{line}\n");
        let src = MapSource::new().with("bad", &text);
        let Err(StyleError::Invalid { problems, .. }) = resolve(&names(&["bad"]), &src, false)
        else {
            panic!("{line}: expected an invalid-style error");
        };
        assert_eq!(
            problems,
            [(
                "config-invalid-value".to_owned(),
                format!("/categories/c/{field}")
            )],
            "{line}"
        );
    }
}

#[test]
fn dir_source_loads_files_and_rejects_path_names() {
    use slopvac_styles::{DirSource, StyleSource};
    let dir = std::env::temp_dir().join(format!("slopvac-styles-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ste.toml"), BASE).unwrap();
    let src = DirSource::new(&dir);
    assert!(src.load("ste").is_ok());
    assert_eq!(
        src.load("../ste").unwrap_err(),
        StyleError::InvalidName("../ste".into())
    );
    assert_eq!(
        src.load("nope").unwrap_err(),
        StyleError::NotFound("nope".into())
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn conflict_keys_compare_numbers_by_value_and_lists_by_item() {
    let a = "schema_version = 3\nconflicts = [\"b\"]\n[categories.c]\nweight = 1\n[rules]\nx = \"off\"\n";
    let b = "schema_version = 3\n[categories.c]\nweight = 1.0\n[rules]\nx = \"error\"\n";
    let src = MapSource::new().with("a", a).with("b", b);
    let r = resolve(&names(&["a", "b"]), &src, false).unwrap();
    // 1 and 1.0 are the same weight, so only the rule level differs.
    assert_eq!(r.conflicts[0].keys, ["rules.x"]);
}
