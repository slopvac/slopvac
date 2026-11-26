//! A default prose run never reports a rule kind or capitalization style that the engine
//! does not run. Both notices are warnings, so one would make every run incomplete.

use std::fs;

use serde_json::Value;
use snapbox::cmd::{Command, cargo_bin};

const PROSE: &str = "# Title\n\nThe cat sat on the mat.\n";

#[test]
fn a_default_prose_run_reports_no_rule_kind_the_engine_lacks() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("a.md"), PROSE).expect("write document");
    let mut cmd = Command::new(cargo_bin!("slopvac"));
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd = cmd.env_remove(var);
    }
    let out = cmd
        .env("NO_COLOR", "1")
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .current_dir(dir.path())
        .args(["lint", "--format", "json", "a.md"])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(0));
    let report: Value = serde_json::from_slice(&out.stdout).expect("a JSON report");
    let codes: Vec<&str> = report["documents"][0]["unchecked"]
        .as_array()
        .expect("unchecked")
        .iter()
        .map(|n| n["code"].as_str().expect("code"))
        .collect();
    for lacking in ["rule-kind-not-implemented", "rule-style-not-implemented"] {
        assert!(!codes.contains(&lacking), "{codes:?}");
    }
}
