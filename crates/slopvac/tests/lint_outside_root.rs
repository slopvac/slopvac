//! A lint target outside the directory of the config file, which is the project root, is
//! linted and matches no root-relative pattern. It never crashes the run.

use std::fs;

use snapbox::cmd::{Command, cargo_bin};

/// Prose with several slop-lexicon words, so the run fails its gate with exit 1.
const SLOPPY: &str = "# Title\n\nWe leverage a robust, seamless solution to delve into the data. It is important to note that this is a game-changer.\n";

#[test]
fn a_target_outside_the_config_directory_is_linted_not_a_panic() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fixtures = dir.path().join("fixtures");
    fs::create_dir(&fixtures).expect("create fixtures");
    // The exclude pattern would drop the target if it applied outside the root.
    let config = fixtures.join("slopvac.toml");
    fs::write(&config, "schema_version = 3\nexclude = [\"*.md\"]\n").expect("write config");
    let target = dir.path().join("README.md");
    fs::write(&target, SLOPPY).expect("write document");

    let mut cmd = Command::new(cargo_bin!("slopvac"));
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd = cmd.env_remove(var);
    }
    let assert = cmd
        .env("NO_COLOR", "1")
        .current_dir(dir.path())
        .args(["lint", "--config"])
        .arg(&config)
        .arg(&target)
        .assert()
        .code(1);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
}
