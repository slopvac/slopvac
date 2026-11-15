//! Installed-binary tests for the command tree, exit codes and global settings.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

fn slopvac() -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "NO_COLOR",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

fn doc(contents: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("doc.md");
    fs::write(&path, contents).expect("write");
    (dir, path)
}

#[test]
fn bare_path_lints_like_the_lint_command() {
    let (_dir, path) = doc(b"# Title\n\nPlain text.\n");
    slopvac().arg(&path).assert().code(0);
    slopvac().arg("lint").arg(&path).assert().code(0);
}

#[test]
fn bare_path_works_after_global_flags() {
    let (_dir, path) = doc(b"hello\n");
    slopvac()
        .args(["--offline", "--color", "never"])
        .arg(&path)
        .assert()
        .code(0);
}

#[test]
fn missing_file_is_an_incomplete_run() {
    slopvac()
        .arg("/nonexistent/slopvac/doc.md")
        .assert()
        .code(2);
}

#[test]
fn invalid_utf8_is_an_incomplete_run() {
    let (_dir, path) = doc(&[0x66, 0xff, 0x6f]);
    slopvac()
        .arg(&path)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("not valid UTF-8"));
}

#[test]
fn no_input_is_incomplete() {
    slopvac().arg("lint").assert().code(2);
}

#[test]
fn unknown_option_is_a_usage_error_with_exit_2() {
    slopvac().args(["lint", "--bogus"]).assert().code(2);
}

#[test]
fn version_and_help_exit_0() {
    slopvac()
        .arg("--version")
        .assert()
        .code(0)
        .stdout(predicate::str::starts_with("slopvac "));
    slopvac().arg("--help").assert().code(0);
}

#[test]
fn removed_options_point_at_the_replacement() {
    let (_dir, path) = doc(b"x\n");
    let cases = [
        (vec!["--profile", "strict"], "slopvac migrate"),
        (vec!["--rules-dir", "rules"], "slopvac migrate"),
        (vec!["--no-vale"], "Vale is gone"),
        (vec!["--comments"], "--mode code-comments"),
    ];
    for (args, expect) in cases {
        slopvac()
            .arg("lint")
            .args(&args)
            .arg(&path)
            .assert()
            .code(2)
            .stderr(predicate::str::contains(expect));
    }
}

#[test]
fn removed_commands_are_unknown() {
    for name in ["compile", "cache"] {
        let (_dir, path) = doc(b"x\n");
        // A removed command word is read as a path, so the lint fails to find it.
        slopvac().arg(name).arg(&path).assert().code(2);
    }
}

#[test]
fn help_offers_no_way_to_turn_offline_off() {
    let out = slopvac().arg("--help").output().expect("run");
    let help = String::from_utf8_lossy(&out.stdout).to_lowercase();
    assert!(help.contains("--offline"));
    for flag in ["--online", "--no-offline", "--allow-network"] {
        assert!(!help.contains(flag), "{flag}");
    }
}

#[test]
fn log_stays_off_without_slopvac_log() {
    let (_dir, path) = doc(b"x\n");
    slopvac()
        .arg(&path)
        .assert()
        .code(0)
        .stderr(predicate::str::is_empty());
}

#[test]
fn trace_logging_never_copies_document_text() {
    let sentinel = "ZX9-SENTINEL-4471";
    let (dir, path) = doc(format!("# {sentinel}\n\nBody {sentinel}\n").as_bytes());
    let out = slopvac()
        .env("SLOPVAC_LOG", "trace")
        .arg(&path)
        .output()
        .expect("run");
    let all = [out.stdout, out.stderr].concat();
    let all = String::from_utf8_lossy(&all);
    assert!(!all.contains(sentinel), "sentinel leaked: {all}");
    assert!(
        !all.contains(&dir.path().display().to_string()),
        "absolute path leaked: {all}"
    );
}

#[test]
fn logging_emits_on_stderr_when_enabled() {
    let (_dir, path) = doc(b"x\n");
    slopvac()
        .env("SLOPVAC_LOG", "debug")
        .arg(&path)
        .assert()
        .code(0)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("lint start"));
}

#[test]
fn unwired_commands_exit_2_naming_the_owner() {
    slopvac()
        .arg("migrate")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("not available in this build yet"));
}

#[test]
fn completions_cover_every_shell() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        slopvac()
            .args(["completions", shell])
            .assert()
            .code(0)
            .stdout(predicate::str::contains("slopvac"));
    }
}

#[test]
fn man_page_is_roff() {
    slopvac()
        .arg("man")
        .assert()
        .code(0)
        .stdout(predicate::str::contains(".TH"));
}

#[test]
fn man_out_dir_writes_a_page_per_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    slopvac()
        .args(["man", "--out-dir"])
        .arg(dir.path())
        .assert()
        .code(0);
    for page in ["slopvac.1", "slopvac-lint.1", "slopvac-doctor.1"] {
        assert!(dir.path().join(page).is_file(), "{page}");
    }
}

#[test]
fn checked_in_cli_reference_is_current() {
    let generated = slopvac().arg("cli-reference").output().expect("run").stdout;
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/reference/cli.md");
    // A Windows checkout may convert the committed file to CRLF.
    let committed = fs::read_to_string(path)
        .expect("docs/reference/cli.md exists")
        .replace("\r\n", "\n");
    assert!(
        String::from_utf8(generated).expect("utf-8") == committed,
        "docs/reference/cli.md is stale; run `cargo run -p slopvac -- cli-reference > docs/reference/cli.md`"
    );
}

#[test]
fn failures_carry_stable_bracketed_codes() {
    let (_dir, bad) = doc(&[0xff]);
    let cases: Vec<(Vec<std::ffi::OsString>, &str)> = vec![
        (vec!["lint".into()], "error[no-input]: "),
        (
            vec![bad.clone().into_os_string()],
            "error[input-not-utf8]: ",
        ),
        (
            vec!["/nonexistent/x.md".into()],
            "error[input-unreadable]: ",
        ),
        (
            vec!["lint".into(), "--no-vale".into(), "x".into()],
            "error[removed-option]: ",
        ),
        (vec!["migrate".into()], "error[command-not-wired]: "),
        (vec!["lint".into(), "--bogus".into()], "error[usage]: "),
    ];
    for (args, prefix) in cases {
        slopvac()
            .args(&args)
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with(prefix));
    }
}

#[test]
fn file_errors_never_show_absolute_paths() {
    let (dir, path) = doc(&[0xff]);
    let out = slopvac().arg(&path).output().expect("run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("<external>/doc.md"), "{stderr}");
    assert!(
        !stderr.contains(&dir.path().display().to_string()),
        "{stderr}"
    );
}

#[test]
fn debug_logging_with_forced_colour_does_not_panic() {
    let (_dir, path) = doc(b"x\n");
    slopvac()
        .env("SLOPVAC_LOG", "debug")
        .args(["--color", "always"])
        .arg(&path)
        .assert()
        .code(0)
        .stderr(predicate::str::contains("\u{1b}["));
}

#[test]
fn colour_flag_colours_help_and_no_color_does_not() {
    slopvac()
        .args(["--color", "always", "--help"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("\u{1b}["));
    slopvac()
        .args(["--help", "--no-color"])
        .env("FORCE_COLOR", "1")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("\u{1b}[").not());
}

#[test]
fn man_pages_into_an_unwritable_directory_exit_2() {
    let (_dir, file) = doc(b"not a directory");
    slopvac()
        .args(["man", "--out-dir"])
        .arg(&file)
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[output-failed]"));
}

#[test]
fn usage_errors_keep_clap_styling_when_colour_is_forced() {
    slopvac()
        .args(["--color", "always", "lint", "--bogus"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("\u{1b}["))
        .stderr(predicate::str::contains("error[usage]:"));
    slopvac()
        .args(["--color", "never", "lint", "--bogus"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("\u{1b}[").not())
        .stderr(predicate::str::starts_with("error[usage]:"));
}

#[test]
fn version_json_prints_a_document_with_notices() {
    let cache = tempfile::tempdir().unwrap();
    let out = slopvac()
        .env("SLOPVAC_CACHE_DIR", cache.path())
        .env("SLOPVAC_OFFLINE", "1")
        .args(["--version", "--json"])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let doc: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(doc["name"], "slopvac");
    assert_eq!(doc["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(doc["notices"], serde_json::json!([]));
}

#[test]
fn self_update_on_an_unmanaged_install_names_the_upgrade_command() {
    // The test binary is not an installer install, so `self update` explains instead of fetching.
    slopvac()
        .env("SLOPVAC_OFFLINE", "1")
        .env("AXOUPDATER_CONFIG_PATH", "/nonexistent")
        .args(["self", "update"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[self-update-unmanaged]"));
}

/// A project directory with a `slopvac.toml` that holds `network_toml`.
fn project(network_toml: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("slopvac.toml"),
        format!("schema_version = 3\n[network]\n{network_toml}"),
    )
    .unwrap();
    dir
}

/// A command with every update opt-out variable removed.
fn update_enabled() -> Command {
    let mut cmd = slopvac();
    for var in [
        "CI",
        "NO_UPDATE_NOTIFIER",
        "SLOPVAC_NO_UPDATE_CHECK",
        "DO_NOT_TRACK",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

/// A cache directory that says 99.0.0 is out.
fn cache_with_newer_release() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("update-check.json"),
        r#"{"checked_at":4102444800,"latest":"99.0.0"}"#,
    )
    .unwrap();
    dir
}

fn version_json(cmd: &mut Command, cache: &std::path::Path) -> serde_json::Value {
    let out = cmd
        .env("SLOPVAC_CACHE_DIR", cache)
        .args(["--version", "--json"])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn project_update_check_false_suppresses_the_notice() {
    let cache = cache_with_newer_release();
    let with_notice = version_json(&mut update_enabled(), cache.path());
    assert_eq!(with_notice["notices"][0]["x-latest"], "99.0.0");

    let dir = project("update_check = false\n");
    let silenced = version_json(update_enabled().current_dir(dir.path()), cache.path());
    assert_eq!(silenced["notices"], serde_json::json!([]));
}

#[test]
fn project_offline_blocks_self_update_before_any_request() {
    // Make the test binary look like an installer install, so only offline mode can stop it.
    let exe = slopvac().get_program().to_owned();
    let prefix = std::path::Path::new(&exe).parent().unwrap().to_owned();
    let receipts = tempfile::tempdir().unwrap();
    fs::write(
        receipts.path().join("slopvac-receipt.json"),
        serde_json::json!({
            "install_prefix": prefix,
            "source": {"app_name": "slopvac"},
        })
        .to_string(),
    )
    .unwrap();
    let dir = project("offline = true\n");
    slopvac()
        .current_dir(dir.path())
        .env("AXOUPDATER_CONFIG_PATH", receipts.path())
        .args(["self", "update"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "error[offline-network-needed]: offline: self update needs the network",
        ));
}

#[test]
fn version_json_accepts_flags_in_any_order() {
    let cache = cache_with_newer_release();
    for args in [
        &["--json", "--version"][..],
        &["--color", "never", "--version", "--json"],
        &["--color=never", "--json", "-V"],
        &["--no-color", "--version", "--json"],
    ] {
        let out = update_enabled()
            .env("SLOPVAC_CACHE_DIR", cache.path())
            .args(args)
            .assert()
            .code(0)
            .get_output()
            .stdout
            .clone();
        let doc: serde_json::Value =
            serde_json::from_slice(&out).unwrap_or_else(|e| panic!("{args:?}: {e}"));
        assert_eq!(doc["name"], "slopvac", "{args:?}");
    }
}

#[test]
fn unreadable_project_config_exits_two_before_any_command_runs() {
    // A directory, an oversize file and (on unix) a FIFO all fail the same way and never hang.
    let mut cases: Vec<(&str, tempfile::TempDir)> = Vec::new();

    let dir_case = tempfile::tempdir().unwrap();
    fs::create_dir(dir_case.path().join("slopvac.toml")).unwrap();
    cases.push(("not a regular file", dir_case));

    let big = tempfile::tempdir().unwrap();
    fs::write(big.path().join("slopvac.toml"), vec![b'#'; 1024 * 1024 + 1]).unwrap();
    cases.push(("larger than 1048576 bytes", big));

    #[cfg(unix)]
    {
        let fifo_case = tempfile::tempdir().unwrap();
        let status = std::process::Command::new("mkfifo")
            .arg(fifo_case.path().join("slopvac.toml"))
            .status()
            .expect("mkfifo");
        assert!(status.success());
        cases.push(("not a regular file", fifo_case));
    }

    for (message, dir) in &cases {
        for args in [
            &["--version", "--json"][..],
            &["self", "update"],
            &["--help"],
        ] {
            slopvac()
                .current_dir(dir.path())
                .timeout(std::time::Duration::from_secs(20))
                .args(args)
                .assert()
                .code(2)
                .stderr(predicate::str::contains("error[config-unreadable]"))
                .stderr(predicate::str::contains(*message));
        }
    }
}

#[test]
fn config_read_errors_do_not_print_absolute_paths() {
    // policies.md 1.7: paths in errors are relative to the project root, or `<external>/<name>`.
    let base = tempfile::Builder::new()
        .prefix("sentinel-secret-")
        .tempdir()
        .unwrap();
    let sentinel = base
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    fs::create_dir(base.path().join("slopvac.toml")).unwrap();
    let sub = base.path().join("sub");
    fs::create_dir(&sub).unwrap();

    // The config sits above the working directory, so it is outside the root.
    let outside = slopvac()
        .current_dir(&sub)
        .arg("--version")
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    let outside = String::from_utf8(outside).unwrap();
    assert!(!outside.contains(&sentinel), "{outside}");
    assert!(
        outside.contains("error[config-unreadable]: <external>/slopvac.toml: not a regular file"),
        "{outside}"
    );

    // The config sits in the working directory: shown relative to it.
    let inside = slopvac()
        .current_dir(base.path())
        .arg("--version")
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    let inside = String::from_utf8(inside).unwrap();
    assert!(!inside.contains(&sentinel), "{inside}");
    assert!(
        inside.contains("error[config-unreadable]: slopvac.toml: not a regular file"),
        "{inside}"
    );
}
