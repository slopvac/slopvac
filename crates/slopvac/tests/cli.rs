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
fn stdin_input_is_linted_and_named_stdin() {
    slopvac()
        .args(["lint", "--stdin"])
        .write_stdin("fine\n")
        .assert()
        .code(0);
    slopvac()
        .args(["lint", "--stdin"])
        .write_stdin(&[0xffu8][..])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("<stdin>: not valid UTF-8"));
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
    let committed = fs::read(path).expect("docs/reference/cli.md exists");
    assert!(
        generated == committed,
        "docs/reference/cli.md is stale; run `cargo run -p slopvac -- cli-reference > docs/reference/cli.md`"
    );
}
