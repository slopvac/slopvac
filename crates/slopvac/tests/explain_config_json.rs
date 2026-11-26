//! `slopvac lint --explain-config --format json` prints the resolved settings of each input as
//! one JSON document.
//!
//! The field names follow the proposal on the pull request for this change and stay
//! provisional until it is ruled on.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::fs;

use serde_json::{json, Value};
use snapbox::cmd::{cargo_bin, Command};

/// A project locale, and a `[[overrides]]` block that gives `docs/**` another one.
const CONFIG: &str = "schema_version = 3\nlocale = \"en-GB\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\nlocale = \"en-US\"\n";

/// A temp project with `slopvac.toml` set to [`CONFIG`] and a document at `docs/a.md`.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("slopvac.toml"), CONFIG).expect("write config");
    fs::create_dir(dir.path().join("docs")).expect("docs dir");
    fs::write(dir.path().join("docs/a.md"), "# Title\n\nText.\n").expect("write document");
    dir
}

fn slopvac(dir: &std::path::Path) -> Command {
    let mut cmd = Command::new(cargo_bin!("slopvac"));
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd = cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1")
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .current_dir(dir)
}

#[test]
fn explain_config_prints_the_config_and_each_setting_of_a_file_as_json() {
    let dir = project();
    let assert = slopvac(dir.path())
        .args(["lint", "--explain-config", "--format", "json", "docs/a.md"])
        .assert()
        .code(0)
        .stderr_eq("");
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    let doc: Value = serde_json::from_str(&stdout).expect("stdout is one JSON document");
    assert_eq!(doc["schema_version"], json!(3), "{stdout}");
    let inputs = doc["inputs"].as_array().expect("an `inputs` list");
    assert_eq!(inputs.len(), 1, "{stdout}");
    let input = &inputs[0];
    assert_eq!(input["path"], json!("docs/a.md"), "{stdout}");
    assert_eq!(input["config_root"], json!("."), "{stdout}");
    assert_eq!(
        input["config_sources"],
        json!([
            {"layer": "core"},
            {"layer": "project", "file": "slopvac.toml"},
        ]),
        "{stdout}"
    );
    assert_eq!(input["profile"], json!("normal"), "{stdout}");
    let locale = input["settings"]
        .as_array()
        .expect("a `settings` list")
        .iter()
        .find(|s| s["key"] == json!("locale"))
        .expect("a `locale` entry");
    assert_eq!(
        locale,
        &json!({
            "key": "locale",
            "value": "en-US",
            "source": {"layer": "override", "file": "slopvac.toml", "line": 6, "column": 1},
            "overridden": [
                {
                    "value": "en-GB",
                    "source": {"layer": "project", "file": "slopvac.toml", "line": 2, "column": 1},
                },
            ],
        }),
        "{stdout}"
    );
}
