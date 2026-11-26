//! Installed-binary tests for `lint --stdin` (compat rows `behavior:stdin-*`,
//! `opt:lint:--stdin`, `opt:lint:--language`; policies.md 1.7).
//!
//! These tests cover input handling, option checks and name redaction. Findings are checked
//! in `lint_engine.rs`.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

mod support;

use assert_cmd::Command;
use predicates::prelude::*;

fn slopvac() -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    // Tests stay off the network: the update notice may refresh its cache after lint, doctor and prime.
    cmd.env("SLOPVAC_NO_UPDATE_CHECK", "1");
    cmd.current_dir(support::neutral_dir());
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1");
    cmd
}

fn stdin_lint(args: &[&str]) -> Command {
    let mut cmd = slopvac();
    cmd.arg("lint").arg("--stdin").args(args);
    cmd
}

#[test]
fn utf8_document_on_stdin_is_accepted() {
    stdin_lint(&[])
        .write_stdin("Plain words, nothing more.\n")
        .assert()
        .code(0);
}

#[test]
fn empty_stdin_is_accepted() {
    stdin_lint(&[]).write_stdin("").assert().code(0);
}

#[test]
fn markdown_and_mdx_languages_are_linted() {
    for language in ["markdown", "mdx"] {
        stdin_lint(&["--language", language])
            .write_stdin("words\n")
            .assert()
            .code(0);
    }
}

#[test]
fn rst_is_accepted_and_checked_when_a_converter_exists() {
    // Without docutils the run is incomplete (exit 2) but never a usage error.
    let out = stdin_lint(&["--language", "rst"])
        .write_stdin("words\n")
        .output()
        .expect("run");
    assert!(matches!(out.status.code(), Some(0 | 2)), "{out:?}");
    assert!(!String::from_utf8_lossy(&out.stderr).contains("error[usage]"));
}

// STUB(slopvac-8l4.3.84): the engine has no parser for plain text or HTML yet, so these inputs
// are accepted by the options and then reported as unsupported (exit 2), never skipped
// silently.
#[test]
fn inputs_without_an_engine_parser_are_an_incomplete_run() {
    let cases: [&[&str]; 2] = [&["--language", "text"], &["--language", "html"]];
    for args in cases {
        stdin_lint(args)
            .write_stdin("words\n")
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with(
                "error[input-unsupported]: <stdin>",
            ));
    }
}

#[test]
fn comments_on_stdin_are_linted_through_the_engine() {
    let out = stdin_lint(&[
        "--mode",
        "code-comments",
        "--language",
        "python",
        "--format",
        "json",
    ])
    .write_stdin("# Parse the input.\nx = 1\n")
    .output()
    .expect("run");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).expect("a JSON report");
    let codes: Vec<&str> = report["documents"][0]["unchecked"]
        .as_array()
        .expect("unchecked")
        .iter()
        .map(|n| n["code"].as_str().expect("code"))
        .collect();
    assert_eq!(codes, ["comment-rules-not-run"], "{report}");
}

#[test]
fn invalid_utf8_names_stdin_and_the_first_bad_byte() {
    // "abc" is three valid bytes, so the first bad byte sits at offset 3.
    stdin_lint(&[])
        .write_stdin(&b"abc\xff text.\n"[..])
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with(
            "error[input-not-utf8]: <stdin>: not valid UTF-8 (byte 3)",
        ))
        .stdout(predicate::str::is_empty());
}

#[test]
fn truncated_multibyte_sequence_is_rejected() {
    // U+00E9 is C3 A9; the lone lead byte at offset 1 is incomplete.
    stdin_lint(&[])
        .write_stdin(&b"a\xc3"[..])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("(byte 1)"));
}

#[test]
fn error_output_never_shows_a_temporary_directory() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let out = stdin_lint(&[])
        .env("TMPDIR", scratch.path())
        .env("SLOPVAC_LOG", "trace")
        .write_stdin(&b"\xff"[..])
        .output()
        .expect("run");
    let all = [out.stdout, out.stderr].concat();
    let all = String::from_utf8_lossy(&all);
    assert!(all.contains("<stdin>"), "{all}");
    assert!(!all.contains("slopvac-stdin"), "{all}");
    assert!(
        !all.contains(&scratch.path().display().to_string()),
        "{all}"
    );
}

#[test]
fn trace_logging_never_copies_stdin_text() {
    let sentinel = "ZX9-STDIN-SENTINEL-3302";
    let out = stdin_lint(&[])
        .env("SLOPVAC_LOG", "trace")
        .write_stdin(format!("# {sentinel}\n"))
        .output()
        .expect("run");
    let all = [out.stdout, out.stderr].concat();
    assert!(!String::from_utf8_lossy(&all).contains(sentinel));
}

#[test]
fn rejected_combinations_are_usage_errors() {
    let cases: &[(&[&str], &str)] = &[
        (&["README.md"], "PATH must be empty"),
        (
            &["--diff-base", "HEAD"],
            "--diff-base is not supported with --stdin.",
        ),
        (
            &["--diff-working-tree"],
            "--diff-working-tree is not supported with --stdin.",
        ),
        (
            &["--fix"],
            "--fix is not supported with --stdin because stdin has no file to write back to.",
        ),
        (&["--open"], "--open is not supported with --stdin."),
        (
            &["--mode", "code-comments"],
            "--language is required with --stdin --mode code-comments.",
        ),
        (
            &["--language", "unknown"],
            "--language \"unknown\" is not valid for mode prose. Valid values: html, markdown, mdx, rst, text.",
        ),
        (
            &["--mode", "prose", "--language", "python"],
            "not valid for mode prose",
        ),
        (
            &["--mode", "code-comments", "--language", "markdown"],
            "not valid for mode code-comments",
        ),
    ];
    for (args, message) in cases {
        stdin_lint(args).write_stdin("x\n").assert().code(2).stderr(
            predicate::str::starts_with("error[usage]: ").and(predicate::str::contains(*message)),
        );
    }
}

#[test]
fn language_without_stdin_is_a_usage_error() {
    slopvac()
        .args(["lint", "--language", "markdown", "README.md"])
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[usage]: "));
}

#[test]
fn bad_options_fail_before_stdin_is_read() {
    stdin_lint(&["--fix"])
        .write_stdin(&b"\xff"[..])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--fix is not supported"))
        .stderr(predicate::str::contains("UTF-8").not());
}
