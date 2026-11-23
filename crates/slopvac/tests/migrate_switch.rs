//! A profile switch to the same 2.x profile keeps a moved word blocklist working. The switch
//! block that `--preserve-behavior` writes must not pin the vocabulary rules off, because the
//! companion style sets their level and a rule entry in an override would outrank it.
//!
//! The written config is resolved per file through the resolver that `slopvac lint` uses. The
//! command line cannot run this layout end to end yet. Expected levels are the 2.x levels of the
//! four vocabulary rules: `error` under strict and `suggestion` under normal.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::path::Path;

use rstest::rstest;
use slopvac_cli::lint_settings::{Overrides, build, resolve};
use slopvac_cli::rule_catalog::Catalog;
use slopvac_config::load;
use slopvac_engine::Level;
use slopvac_migrate::{Options, RenameTable, migrate_config};
use slopvac_model::{RuleId, Severity};
use slopvac_styles::{BuiltinSource, StylePresets};

const RULES: [&str; 4] = [
    "core:ste-verbs.verb-form-not-listed",
    "core:ste-words.verb-or-adjective-form-not-permitted",
    "core:ste-words.word-outside-controlled-vocabulary",
    "core:ste-words.word-used-in-wrong-part-of-speech",
];

/// The level of each vocabulary rule for `file` in the migrated config.
fn levels(config: &str, file: &str) -> Vec<Option<Level>> {
    let rewrite = migrate_config(
        config,
        "slopvac.toml",
        &RenameTable::bundled(),
        Options {
            preserve_behavior: true,
        },
    )
    .expect("the layout migrates");
    // The migrated config lists the switch block and the original block with the same files,
    // which the loader refuses until the migration folds them together. The original block
    // holds only the `profile` key, which the switch block already applies, so it is dropped.
    let mut table: toml::Table = rewrite.text.parse().expect("the migrated config parses");
    let overrides = table["overrides"].as_array_mut().expect("overrides");
    assert_eq!(overrides.len(), 2, "{overrides:?}");
    overrides.truncate(1);
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("slopvac.toml");
    std::fs::write(&path, table.to_string()).expect("config");
    let loaded = load(&path, &StylePresets(BuiltinSource)).expect("the config loads");
    let resolved = resolve(&loaded, Path::new(file));
    let catalog = Catalog::load().expect("the rules load");
    let settings = build(&catalog, &resolved, Vec::new(), &Overrides::default()).expect("settings");
    RULES
        .iter()
        .map(|id| {
            settings
                .rule_levels
                .get(&RuleId::new(*id).expect("id"))
                .copied()
        })
        .collect()
}

#[rstest]
#[case::normal_to_normal("normal", Severity::Suggestion)]
#[case::strict_to_strict("strict", Severity::Error)]
fn a_switch_to_the_same_profile_keeps_the_vocabulary_levels(
    #[case] profile: &str,
    #[case] severity: Severity,
) {
    let config = format!(
        "schema_version = 2\nprofile = \"{profile}\"\n[vocabulary]\npath = \"deploy.toml\"\n\n\
         [[overrides]]\nfiles = [\"docs/**\"]\nprofile = \"{profile}\"\n"
    );
    let want = vec![Some(Level::On(severity)); RULES.len()];
    assert_eq!(levels(&config, "docs/in.md"), want, "inside the override");
    assert_eq!(levels(&config, "out.md"), want, "outside the override");
}
