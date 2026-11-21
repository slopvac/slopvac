//! The update notice after `lint`, `doctor` and `prime`: stderr only, once the command's own
//! output is done, and only where the update policy allows it.
//!
//! The network is never reached. A cache entry stamped far in the future counts as fresh, so
//! the notice comes from the cache and no refresh starts.

use std::fs;
use std::path::Path;

use rstest::rstest;
use snapbox::cmd::{Command, cargo_bin};

const NOTICE: &str = "notice[update-available]: slopvac 99.0.0 is available (you have ";

/// A cache directory that says 99.0.0 is out.
fn seeded_cache() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("update-check.json"),
        r#"{"checked_at":4102444800,"latest":"99.0.0"}"#,
    )
    .expect("seed the cache");
    dir
}

/// The binary in `work`, with home, cache and config directories inside `home`, and every
/// update opt-out removed.
fn slopvac(work: &Path, home: &Path, cache: &Path) -> Command {
    let mut cmd = Command::new(cargo_bin!("slopvac"));
    for var in [
        "CI",
        "NO_UPDATE_NOTIFIER",
        "SLOPVAC_NO_UPDATE_CHECK",
        "DO_NOT_TRACK",
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
        "XDG_CACHE_HOME",
        "XDG_CONFIG_HOME",
    ] {
        cmd = cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1")
        .env("HOME", home)
        .env("SLOPVAC_CACHE_DIR", cache)
        .env("AXOUPDATER_CONFIG_PATH", home.join("receipts"))
        .current_dir(work)
}

struct Fixture {
    work: tempfile::TempDir,
    home: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let work = tempfile::tempdir().expect("tempdir");
    fs::write(work.path().join("slopvac.toml"), "schema_version = 3\n").expect("config");
    fs::write(work.path().join("doc.md"), "# Title\n\nPlain words here.\n").expect("doc");
    Fixture {
        work,
        home: tempfile::tempdir().expect("tempdir"),
    }
}

fn run(fx: &Fixture, cache: &Path, args: &[&str]) -> std::process::Output {
    slopvac(fx.work.path(), fx.home.path(), cache)
        .args(args)
        .assert()
        .get_output()
        .clone()
}

#[rstest]
#[case::lint(&["lint", "doc.md"])]
#[case::doctor(&["doctor"])]
#[case::prime(&["prime"])]
fn a_cached_newer_release_is_noticed_on_stderr_after_the_output(#[case] args: &[&str]) {
    let fx = fixture();
    let cache = seeded_cache();
    let out = run(&fx, cache.path(), args);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stderr.contains(NOTICE), "stderr: {stderr}");
    assert!(!stdout.contains("update-available"), "stdout: {stdout}");
    let last = stderr.lines().last().unwrap_or_default();
    assert!(last.starts_with(NOTICE), "the notice comes last: {stderr}");
}

// `doctor` is left out: its report names the offline state this comparison turns on.
#[rstest]
#[case::lint(&["lint", "doc.md"])]
#[case::prime(&["prime"])]
fn the_notice_leaves_stdout_and_the_exit_code_alone(#[case] args: &[&str]) {
    let fx = fixture();
    let quiet = tempfile::tempdir().expect("tempdir");
    let with = run(&fx, seeded_cache().path(), args);
    let without = run(&fx, quiet.path(), &[args, &["--offline"]].concat());
    assert_eq!(with.status.code(), without.status.code());
    assert_eq!(with.stdout, without.stdout);
}

#[rstest]
#[case::ci("CI", "true")]
#[case::no_update_notifier("NO_UPDATE_NOTIFIER", "1")]
#[case::no_update_check("SLOPVAC_NO_UPDATE_CHECK", "1")]
#[case::do_not_track("DO_NOT_TRACK", "1")]
fn an_opt_out_variable_suppresses_the_notice(#[case] name: &str, #[case] value: &str) {
    let fx = fixture();
    let cache = seeded_cache();
    let out = slopvac(fx.work.path(), fx.home.path(), cache.path())
        .env(name, value)
        .args(["lint", "doc.md"])
        .assert()
        .get_output()
        .clone();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("update-available"), "stderr: {stderr}");
}

#[test]
fn update_check_false_in_the_project_config_suppresses_the_notice() {
    let fx = fixture();
    fs::write(
        fx.work.path().join("slopvac.toml"),
        "schema_version = 3\n[network]\nupdate_check = false\n",
    )
    .expect("config");
    let cache = seeded_cache();
    let out = run(&fx, cache.path(), &["lint", "doc.md"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("update-available"), "stderr: {stderr}");
}

#[rstest]
#[case::lint(&["lint", "doc.md"])]
#[case::doctor(&["doctor"])]
#[case::prime(&["prime"])]
fn offline_gives_no_notice_even_from_a_cache(#[case] args: &[&str]) {
    let fx = fixture();
    let cache = seeded_cache();
    let out = run(&fx, cache.path(), &[args, &["--offline"]].concat());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("update-available"), "stderr: {stderr}");
}

#[rstest]
#[case::lint(&["lint", "doc.md"])]
#[case::doctor(&["doctor"])]
#[case::prime(&["prime"])]
fn offline_starts_no_refresh_and_writes_no_cache(#[case] args: &[&str]) {
    let fx = fixture();
    let cache = tempfile::tempdir().expect("tempdir");
    run(&fx, cache.path(), &[args, &["--offline"]].concat());
    assert!(!cache.path().join("update-check.json").exists());
}

#[rstest]
#[case::version(&["--version"])]
#[case::rules(&["rules", "--help"])]
#[case::self_update_help(&["self", "update", "--help"])]
fn other_commands_print_no_notice(#[case] args: &[&str]) {
    let fx = fixture();
    let cache = seeded_cache();
    let out = run(&fx, cache.path(), args);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("update-available"), "stderr: {stderr}");
}
