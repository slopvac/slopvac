//! The network policy comes from the resolved config stack: `extends` and precedence apply, and
//! an unreadable config stops every command except `doctor`, which reports it.

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;

fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    for (path, body) in files {
        fs::write(dir.path().join(path), body).unwrap();
    }
    dir
}

fn slopvac(dir: &tempfile::TempDir) -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    for var in [
        "SLOPVAC_OFFLINE",
        "CI",
        "NO_UPDATE_NOTIFIER",
        "SLOPVAC_NO_UPDATE_CHECK",
        "DO_NOT_TRACK",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1").current_dir(dir.path());
    cmd
}

fn cache_with_newer_release() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("update-check.json"),
        r#"{"checked_at":4102444800,"latest":"99.0.0"}"#,
    )
    .unwrap();
    dir
}

fn notices(dir: &tempfile::TempDir, cache: &tempfile::TempDir) -> serde_json::Value {
    let out = slopvac(dir)
        .env("SLOPVAC_CACHE_DIR", cache.path())
        .args(["--version", "--json"])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice::<serde_json::Value>(&out).unwrap()["notices"].clone()
}

#[test]
fn update_check_false_in_an_extended_file_silences_the_notice() {
    let cache = cache_with_newer_release();
    let plain = project(&[("slopvac.toml", "schema_version = 3\n")]);
    assert_eq!(notices(&plain, &cache)[0]["x-latest"], "99.0.0");
    let extended = project(&[
        (
            "base.toml",
            "schema_version = 3\n[network]\nupdate_check = false\n",
        ),
        (
            "slopvac.toml",
            "schema_version = 3\nextends = [\"base.toml\"]\n",
        ),
    ]);
    assert_eq!(notices(&extended, &cache), serde_json::json!([]));
}

#[test]
fn the_project_overrides_update_check_from_its_base() {
    let cache = cache_with_newer_release();
    let on = project(&[
        (
            "base.toml",
            "schema_version = 3\n[network]\nupdate_check = false\n",
        ),
        (
            "slopvac.toml",
            "schema_version = 3\nextends = [\"base.toml\"]\n[network]\nupdate_check = true\n",
        ),
    ]);
    assert_eq!(notices(&on, &cache)[0]["x-latest"], "99.0.0");
}

#[test]
fn offline_from_an_extended_file_blocks_self_update_and_the_project_cannot_undo_it() {
    let dir = project(&[
        (
            "base.toml",
            "schema_version = 3\n[network]\noffline = true\n",
        ),
        (
            "slopvac.toml",
            "schema_version = 3\nextends = [\"base.toml\"]\n[network]\noffline = false\n",
        ),
    ]);
    // Make the test binary look like an installer install, so only offline mode can stop
    // `self update` before it asks the network.
    let exe = slopvac(&dir).get_program().to_owned();
    let prefix = std::path::Path::new(&exe).parent().unwrap().to_owned();
    let receipts = tempfile::tempdir().unwrap();
    fs::write(
        receipts.path().join("slopvac-receipt.json"),
        serde_json::json!({"install_prefix": prefix, "source": {"app_name": "slopvac"}})
            .to_string(),
    )
    .unwrap();
    slopvac(&dir)
        .env("AXOUPDATER_CONFIG_PATH", receipts.path())
        .args(["self", "update"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "error[offline-network-needed]: offline: self update needs the network",
        ));
}

#[test]
fn a_refused_config_does_not_stop_a_command_that_needs_no_network_and_doctor_reports_it() {
    let dir = project(&[("slopvac.toml", "mode = \"prose\"\n")]);
    slopvac(&dir)
        .arg("completions")
        .arg("bash")
        .assert()
        .code(0);
    slopvac(&dir)
        .args(["doctor", "--format", "json"])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("\"legacy\""));
}

#[test]
fn an_unreadable_config_is_exit_two_config_unreadable() {
    let dir = project(&[]);
    fs::create_dir(dir.path().join("slopvac.toml")).unwrap();
    slopvac(&dir)
        .arg("rules")
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains(
            "error[config-unreadable]: cannot read slopvac.toml: not a regular file",
        ));
}

#[test]
fn doctor_still_runs_when_the_config_is_unreadable() {
    let dir = project(&[]);
    fs::create_dir(dir.path().join("slopvac.toml")).unwrap();
    slopvac(&dir)
        .args(["doctor", "--format", "json"])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("\"unreadable\""));
}
