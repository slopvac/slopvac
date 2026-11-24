//! A stray scalar in a multiline `overrides` array is refused at its own line, in both
//! migration modes.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use rstest::rstest;
use snapbox::cmd::{Command, cargo_bin};

/// Lines: 1 `schema_version`, 2 `overrides = [`, 3 the first table, 4 the stray scalar, 5 `]`.
const CONFIG: &str =
    "schema_version = 2\noverrides = [\n  { files = [\"docs/**\"] },\n  \"stray\",\n]\n";

#[rstest]
#[case::plain(&[])]
#[case::preserve(&["--preserve-behavior"])]
fn a_stray_scalar_in_a_multiline_override_array_is_reported_at_its_own_line(#[case] mode: &[&str]) {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("slopvac.toml"), CONFIG).expect("config");
    let assert = Command::new(cargo_bin!("slopvac"))
        .env("NO_COLOR", "1")
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .arg("migrate")
        .arg(dir.path())
        .args(mode)
        .assert()
        .code(2);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        stderr.contains("`overrides[1]` on line 4 is not"),
        "{stderr}"
    );
}
