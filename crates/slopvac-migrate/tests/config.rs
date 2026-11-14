//! Config rewrite behavior.

use slopvac_migrate::{Error, Options, RenameTable, migrate_config};

fn migrate(text: &str) -> Result<String, Error> {
    migrate_config(
        text,
        "slopvac.toml",
        &RenameTable::bundled(),
        Options::default(),
    )
    .map(|r| r.text)
}

fn dropped(text: &str) -> Vec<(String, usize)> {
    match migrate(text) {
        Err(Error::Dropped(d)) => d.into_iter().map(|d| (d.key, d.line)).collect(),
        other => panic!("expected Dropped, got {other:?}"),
    }
}

#[test]
fn adds_schema_version_and_keeps_comments() {
    let out = migrate("# Project gate\nprofile = \"strict\" # tier\nmode = \"prose\"\n").unwrap();
    assert!(out.contains("# Project gate\nprofile = \"strict\" # tier\nmode = \"prose\"\n"));
    assert!(out.contains("schema_version = 3"));
}

#[test]
fn renames_categories_and_collapses_severity_only_tables() {
    let out = migrate(
        "[categories]\n# keep quiet\nprose-format = \"off\"\n[categories.prose-craft]\nseverity = \"warning\"\n[categories.ai-residue]\nweight = 2.0\nmax_per_100_words = 1.5\n",
    )
    .unwrap();
    assert!(
        out.contains("# keep quiet\n\"core:prose-format\" = \"off\""),
        "{out}"
    );
    assert!(out.contains("\"core:prose-craft\" = \"warning\""), "{out}");
    assert!(out.contains("[categories.\"core:ai-residue\"]"), "{out}");
    assert!(out.contains("weight = 2.0"), "{out}");
}

#[test]
fn rules_table_form_becomes_bare_level() {
    let out = migrate("[rules.\"prose-format.no-unicode-dash\"]\nseverity = \"off\"\n").unwrap();
    assert!(
        out.contains("\"core:prose-format.no-unicode-dash\" = \"off\""),
        "{out}"
    );
    assert!(!out.contains("[rules."), "{out}");
}

#[test]
fn locale_and_vocabulary_move() {
    let out = migrate(
        "[locale]\ndefault = \"en-GB\"\nallow = [\"colour\"]\n[vocabulary]\npath = \"words.toml\"\n",
    )
    .unwrap();
    assert!(out.contains("locale = \"en-GB\""), "{out}");
    assert!(out.contains("[terms]"), "{out}");
    assert!(out.contains("allow = [\"colour\"]"), "{out}");
    assert!(out.contains("blocklists = [\"words.toml\"]"), "{out}");
    assert!(
        !out.contains("[vocabulary]") && !out.contains("[locale]"),
        "{out}"
    );
}

#[test]
fn override_locale_moves_to_override_terms() {
    let out = migrate(
        "[[overrides]]\nfiles = [\"docs/**\", \"!docs/keep.md\"]\n[overrides.locale]\ndefault = \"en-US\"\nallow = [\"x\"]\n",
    )
    .unwrap();
    assert!(
        out.contains("files = [\"docs/**\", \"!docs/keep.md\"]"),
        "{out}"
    );
    assert!(out.contains("locale = \"en-US\""), "{out}");
    assert!(out.contains("[overrides.terms]"), "{out}");
}

#[test]
fn disabled_vale_table_is_deleted() {
    let out = migrate("[vale]\nenabled = false\nbinary = \"vale\"\n").unwrap();
    assert!(!out.contains("vale"), "{out}");
}

#[test]
fn enabled_vale_stops_the_migration_with_line_numbers() {
    let d = dropped("profile = \"normal\"\n[vale]\nenabled = true\nbinary = \"x\"\n");
    assert_eq!(
        d,
        vec![
            ("vale.enabled".to_owned(), 3),
            ("vale.binary".to_owned(), 4)
        ]
    );
}

#[test]
fn vale_without_enabled_false_is_dropped() {
    assert_eq!(
        dropped("[vale]\nstyles = [\"a\"]\n"),
        vec![("vale.styles".to_owned(), 2)]
    );
}

#[test]
fn reports_every_dropped_setting_at_once() {
    let d = dropped(
        "bogus = 1\n[thresholds]\nmax_errors = 0\nweird = 2\n[[overrides]]\nfiles = [\"a\"]\n[overrides.guidance]\nwrite = \"off\"\n",
    );
    assert_eq!(
        d,
        vec![
            ("bogus".to_owned(), 1),
            ("thresholds.weird".to_owned(), 4),
            ("overrides[0].guidance".to_owned(), 7),
        ]
    );
}

#[test]
fn unknown_rule_name_is_dropped() {
    assert_eq!(
        dropped("[rules]\n\"Not A Rule\" = \"off\"\n"),
        vec![("rules.Not A Rule".to_owned(), 2)]
    );
}

#[test]
fn already_migrated_is_detected() {
    assert!(matches!(
        migrate("schema_version = 3\n"),
        Err(Error::AlreadyMigrated { .. })
    ));
}

#[test]
fn migration_is_idempotent() {
    let once = migrate("profile = \"strict\"\n[categories]\nprose-craft = \"off\"\n").unwrap();
    assert!(matches!(migrate(&once), Err(Error::AlreadyMigrated { .. })));
}

#[test]
fn guidance_and_thresholds_copy() {
    let src = "[thresholds]\nmax_errors = 2\nmin_score = 80.0\n[guidance]\nwrite = \"auto\"\nreview_depth = \"self\"\n";
    let out = migrate(src).unwrap();
    assert!(out.contains(src), "{out}");
}

#[test]
fn preserve_behavior_adds_the_ste_style() {
    let rw = migrate_config(
        "profile = \"normal\"\n",
        "slopvac.toml",
        &RenameTable::bundled(),
        Options {
            preserve_behavior: true,
        },
    )
    .unwrap();
    assert!(rw.text.contains("styles = [\"core:ste\"]"), "{}", rw.text);
}
