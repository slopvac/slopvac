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
fn unknown_names_are_dropped() {
    let cases = [
        ("[rules]\n\"Not A Rule\" = \"off\"\n", "rules.Not A Rule"),
        (
            "[rules]\n\"prose-craft.no-such-rule\" = \"off\"\n",
            "rules.prose-craft.no-such-rule",
        ),
        (
            "[categories]\nno-such-category = \"off\"\n",
            "categories.no-such-category",
        ),
    ];
    for (text, key) in cases {
        assert_eq!(dropped(text), vec![(key.to_owned(), 2)], "{text}");
    }
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
fn a_config_the_version_gate_accepts_or_finds_newer_is_already_migrated() {
    for text in ["schema_version = 4\n", "schema_version = \"3\"\n"] {
        assert!(
            matches!(migrate(text), Err(Error::AlreadyMigrated { .. })),
            "{text}"
        );
    }
}

#[test]
fn a_2x_config_migrates_to_one_the_version_gate_accepts() {
    for text in ["profile = \"strict\"\n", "schema_version = 2\n"] {
        let out = migrate(text).unwrap();
        let root = slopvac_config::tree::parse_toml(&out).unwrap();
        assert!(
            slopvac_config::version::check_version(&root, &out).is_ok(),
            "{out}"
        );
    }
}

#[test]
fn guidance_and_thresholds_copy() {
    let src = "[thresholds]\nmax_errors = 2\nmin_score = 80.0\n[guidance]\nwrite = \"auto\"\nreview_depth = \"self\"\n";
    let out = migrate(src).unwrap();
    assert!(out.contains(src), "{out}");
}

#[test]
fn schema_version_two_is_rewritten_to_three() {
    let out = migrate("schema_version = 2\nprofile = \"strict\"\n").unwrap();
    assert_eq!(out, "schema_version = 3\nprofile = \"strict\"\n");
}

#[test]
fn other_schema_versions_are_not_a_2x_config() {
    assert_eq!(
        dropped("schema_version = 1\nprofile = \"strict\"\n"),
        vec![("schema_version".to_owned(), 1)]
    );
}

#[test]
fn vocabulary_comments_follow_the_path_to_blocklists() {
    let out =
        migrate("# Our word list\n[vocabulary]\n# the file\npath = \"words.toml\" # relative\n")
            .unwrap();
    assert_eq!(
        out,
        "schema_version = 3\n\n[terms]\n# Our word list\n# the file\nblocklists = [\"words.toml\"] # relative\n"
    );
}

#[test]
fn locale_comments_follow_their_values() {
    let out = migrate(
        "# Spelling\n[locale]\n# target\ndefault = \"en-GB\" # UK\n# skip these\nallow = [\"x\"]\n",
    )
    .unwrap();
    assert!(
        out.contains("# Spelling\n# target\nlocale = \"en-GB\" # UK\n"),
        "{out}"
    );
    assert!(out.contains("# skip these\nallow = [\"x\"]"), "{out}");
}
