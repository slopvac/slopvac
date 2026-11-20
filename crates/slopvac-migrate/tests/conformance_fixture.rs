//! The conformance fixture `migrated-2x.toml` is what `migrate` writes for its 2.x source.

use slopvac_migrate::{Options, RenameTable, migrate_config};

/// The 2.x config that the fixture claims to come from.
const SOURCE_2X: &str = r#"# Shape that `slopvac migrate` writes for a typical 2.x config.
profile = "normal"
exclude = [
  "**/node_modules/**",
  "packages/slopvac-lint/tests/fixtures/**",
  "CHANGELOG.md",
  ".beads/**",
]

[thresholds]
max_errors = 0

[categories.ste-descriptive]
severity = "suggestion"

[categories.ste-safety]
severity = "error"
"#;

const FIXTURE: &str = include_str!("../../../schemas/v3/conformance/config/valid/migrated-2x.toml");

#[test]
fn the_fixture_is_the_migrate_output_for_its_2x_source() {
    let written = migrate_config(
        SOURCE_2X,
        "slopvac.toml",
        &RenameTable::bundled(),
        Options::default(),
    )
    .expect("the 2.x source migrates")
    .text;
    assert_eq!(written, FIXTURE);
}
