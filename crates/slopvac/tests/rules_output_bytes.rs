//! Byte pins for `slopvac rules --format json` and `slopvac explain --format json`.
//!
//! The schema tests (`rules_output_schema.rs`) check what the documents accept. These pins
//! check the exact bytes the real binary prints: member order, spacing and the trailing
//! newline. A change to how the documents are built must leave every snapshot unchanged.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use snapbox::cmd::{Command, cargo_bin};

/// The raw stdout of one run from `/`, where no config is found, so the rules are as shipped.
fn raw(args: &[&str]) -> String {
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
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .env("NO_COLOR", "1")
        .current_dir("/")
        .args(args)
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).expect("utf-8");
    // insta ignores trailing newlines, so the pin checks the end itself: one newline.
    assert!(
        text.ends_with("}\n") && !text.ends_with("\n\n"),
        "the document ends with one newline"
    );
    text
}

#[test]
fn the_listing_of_one_core_rule() {
    insta::assert_snapshot!(raw(&[
        "rules",
        "--category",
        "ai-residue",
        "--format",
        "json"
    ]));
}

#[test]
fn explain_of_a_core_rule_with_a_suppression() {
    insta::assert_snapshot!(raw(&[
        "explain",
        "core:ai-residue.chat-leakage",
        "--format",
        "json"
    ]));
}

#[test]
fn explain_of_a_core_rule_without_a_suppression() {
    insta::assert_snapshot!(raw(&["explain", "core:terms.casing", "--format", "json"]));
}

#[test]
fn explain_of_a_rule_whose_severity_a_config_changed() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("slopvac.toml");
    std::fs::write(
        &file,
        "schema_version = 3\n\n[rules]\n\"ai-residue.chat-leakage\" = \"off\"\n",
    )
    .expect("write config");
    insta::assert_snapshot!(raw(&[
        "explain",
        "core:ai-residue.chat-leakage",
        "--format",
        "json",
        "--config",
        file.to_str().expect("utf-8 path"),
    ]));
}
