//! A lint target outside the directory of the config file, which is the project root, is
//! linted and matches no root-relative pattern. It never crashes the run.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::fs;

use snapbox::cmd::{Command, cargo_bin};

/// Prose with several slop-lexicon words, so the run fails its gate with exit 1.
const SLOPPY: &str = "# Title\n\nWe leverage a robust, seamless solution to delve into the data. It is important to note that this is a game-changer.\n";

/// A temp project: `fixtures/slopvac.toml` excludes `*.md`, and `README.md` beside `fixtures`
/// holds `SLOPPY`.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let fixtures = dir.path().join("fixtures");
    fs::create_dir(&fixtures).expect("create fixtures");
    // The exclude pattern would drop the target if it applied outside the root.
    fs::write(
        fixtures.join("slopvac.toml"),
        "schema_version = 3\nexclude = [\"*.md\"]\n",
    )
    .expect("write config");
    fs::write(dir.path().join("README.md"), SLOPPY).expect("write document");
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
    cmd.env("NO_COLOR", "1").current_dir(dir)
}

#[test]
fn a_target_outside_the_config_directory_is_linted_not_a_panic() {
    let dir = project();
    let assert = slopvac(dir.path())
        .args(["lint", "--config"])
        .arg(dir.path().join("fixtures/slopvac.toml"))
        .arg(dir.path().join("README.md"))
        .assert()
        .code(1);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
}

#[test]
fn a_relative_target_outside_the_config_directory_is_linted_not_excluded() {
    let dir = project();
    let assert = slopvac(dir.path())
        .args(["lint", "--config", "fixtures/slopvac.toml", "README.md"])
        .assert()
        .code(1);
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(stdout.contains("README.md"), "stdout: {stdout}");
}
