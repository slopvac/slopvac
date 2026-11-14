//! Behavior of style resolution through the public API.

use slopvac_styles::{Level, MapSource, StyleError, resolve};

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
    assert_eq!(r.patch.rules["core:a"], Level::Off);
    assert_eq!(r.patch.rules["core:b"], Level::Error);
}

#[test]
fn listing_order_decides_the_winner() {
    let src = MapSource::new()
        .with("one", BASE)
        .with("two", "schema_version = 3\n[rules]\n\"core:a\" = \"off\"\n");
    let r = resolve(&names(&["two", "one"]), &src, false).unwrap();
    assert_eq!(r.patch.rules["core:a"], Level::Warning);
}

#[test]
fn extends_resolve_before_the_style_that_names_them() {
    let src = MapSource::new().with("parent", BASE).with(
        "child",
        "schema_version = 3\nextends = [\"parent\"]\n[rules]\n\"core:a\" = \"suggestion\"\n",
    );
    let r = resolve(&names(&["child"]), &src, false).unwrap();
    assert_eq!(r.order, ["parent", "child"]);
    assert_eq!(r.patch.rules["core:a"], Level::Suggestion);
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
    let h = &r.history;
    assert_eq!(h["categories.ste-nouns.weight"].len(), 2);
    assert_eq!(h["categories.ste-nouns.severity"].len(), 1);
    assert_eq!(h["categories.ste-nouns.weight"][1].value, "0.5");
}

#[test]
fn bare_category_level_is_severity() {
    let src = MapSource::new().with("one", BASE);
    let r = resolve(&names(&["one"]), &src, false).unwrap();
    assert_eq!(r.history["categories.ste-words.severity"][0].value, "error");
}

#[test]
fn history_records_each_style_in_order() {
    let src = MapSource::new()
        .with("one", BASE)
        .with("two", "schema_version = 3\n[rules]\n\"core:a\" = \"off\"\n");
    let r = resolve(&names(&["one", "two"]), &src, false).unwrap();
    let h = &r.history["rules.core:a"];
    assert_eq!(
        h.iter()
            .map(|s| (s.style.as_str(), s.value.as_str()))
            .collect::<Vec<_>>(),
        [("one", "warning"), ("two", "off")]
    );
}

#[test]
fn terms_lists_replace() {
    let a = "schema_version = 3\n[terms]\nallow = [\"x\", \"y\"]\n";
    let b = "schema_version = 3\n[terms]\nallow = [\"z\"]\n";
    let src = MapSource::new().with("a", a).with("b", b);
    let r = resolve(&names(&["a", "b"]), &src, false).unwrap();
    assert_eq!(r.patch.terms.allow, Some(names(&["z"])));
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
    assert_eq!(
        resolve(&names(&["bad"]), &src, false).unwrap_err(),
        StyleError::UnknownKey {
            style: "bad".into(),
            key: "foo".into()
        }
    );
    assert!(resolve(&names(&["ok"]), &src, false).is_ok());
}

#[test]
fn wrong_schema_version_is_rejected() {
    let src = MapSource::new().with("old", "schema_version = 2\n");
    assert_eq!(
        resolve(&names(&["old"]), &src, false).unwrap_err(),
        StyleError::SchemaVersion {
            style: "old".into(),
            found: 2
        }
    );
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
