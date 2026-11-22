//! The installed binary escapes a hostile file name in the text of every lint refusal built
//! from a path (threat model O1 to O4): terminal controls, newlines and bidi overrides never
//! reach stderr raw, and the code, wording and exit code stay the same. Report notices keep
//! the raw text, which each report format escapes for itself.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use std::path::Path;
use std::time::Duration;

use rstest::rstest;
use snapbox::cmd::{Command, cargo_bin};

const CAP: u64 = 64 * 1024 * 1024;

/// An OSC title sequence: ESC, then text, then BEL.
const ESC_RAW: &str = "a\u{1b}]0;x\u{7}b";
const ESC_SAFE: &str = "a\\u{1b}]0;x\\u{7}b";
/// A newline that starts a forged GitHub workflow command.
const FORGE_RAW: &str = "a\n::error::forged";
const FORGE_SAFE: &str = "a\\u{a}::error::forged";
const BIDI_RAW: &str = "a\u{202e}b";
const BIDI_SAFE: &str = "a\\u{202e}b";

fn slopvac(dir: &Path) -> Command {
    let mut cmd = Command::new(cargo_bin!("slopvac"));
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
        "SLOPVAC_RST_CONVERTER",
    ] {
        cmd = cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1")
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .current_dir(dir)
        .timeout(Duration::from_secs(30))
}

/// Run `cmd` with `args`, expect exit 2, and return (stdout, stderr).
fn run_in(cmd: Command, args: &[&str]) -> (String, String) {
    let assert = cmd.args(args).assert().code(2);
    let out = assert.get_output();
    (
        String::from_utf8(out.stdout.clone()).expect("utf8 stdout"),
        String::from_utf8(out.stderr.clone()).expect("utf8 stderr"),
    )
}

/// Run `slopvac lint <args>` in an empty temp dir prepared by `setup`, and return stderr.
fn refusal(setup: impl FnOnce(&Path), args: &[&str]) -> String {
    let dir = tempfile::tempdir().expect("temp dir");
    setup(dir.path());
    let mut all = vec!["lint"];
    all.extend_from_slice(args);
    run_in(slopvac(dir.path()), &all).1
}

/// The text of `raw` as the escaper writes it, for the file names built in the tests below.
fn escaped(raw: &str) -> String {
    raw.replace('\u{1b}', "\\u{1b}")
        .replace('\u{7}', "\\u{7}")
        .replace('\n', "\\u{a}")
        .replace('\u{202e}', "\\u{202e}")
}

#[rstest]
#[case::esc(ESC_RAW, ESC_SAFE)]
#[case::newline(FORGE_RAW, FORGE_SAFE)]
#[case::bidi(BIDI_RAW, BIDI_SAFE)]
fn a_too_large_document_is_refused_with_an_escaped_name(#[case] raw: &str, #[case] safe: &str) {
    let file = format!("{raw}.md");
    let err = refusal(
        |dir| {
            std::fs::File::create(dir.join(&file))
                .and_then(|f| f.set_len(CAP + 1))
                .expect("sparse file");
        },
        &[&file],
    );
    assert_eq!(
        err,
        format!("error[document-too-large]: {safe}.md: larger than the 64 MiB document limit\n")
    );
}

#[rstest]
#[case::esc(ESC_RAW, ESC_SAFE)]
#[case::newline(FORGE_RAW, FORGE_SAFE)]
#[case::bidi(BIDI_RAW, BIDI_SAFE)]
fn a_non_utf8_document_is_refused_with_an_escaped_name(#[case] raw: &str, #[case] safe: &str) {
    let file = format!("{raw}.md");
    let err = refusal(
        |dir| std::fs::write(dir.join(&file), [0xff, 0xfe, b'\n']).expect("write"),
        &[&file],
    );
    assert_eq!(
        err,
        format!("error[input-not-utf8]: {safe}.md: not valid UTF-8\n")
    );
}

#[rstest]
#[case::esc(ESC_RAW, ESC_SAFE)]
#[case::newline(FORGE_RAW, FORGE_SAFE)]
#[case::bidi(BIDI_RAW, BIDI_SAFE)]
fn an_unsupported_file_type_is_refused_with_an_escaped_name(#[case] raw: &str, #[case] safe: &str) {
    let file = format!("{raw}.txt");
    let err = refusal(
        |dir| std::fs::write(dir.join(&file), "text\n").expect("write"),
        &[&file],
    );
    assert_eq!(
        err,
        format!("error[input-unsupported]: {safe}.txt: this file type has no engine parser yet\n")
    );
}

#[rstest]
#[case::esc(ESC_RAW, ESC_SAFE)]
#[case::newline(FORGE_RAW, FORGE_SAFE)]
#[case::bidi(BIDI_RAW, BIDI_SAFE)]
fn a_missing_target_is_refused_with_an_escaped_name(#[case] raw: &str, #[case] safe: &str) {
    let file = format!("{raw}.md");
    let err = refusal(|_| {}, &[&file]);
    assert_eq!(
        err,
        format!("error[input-unreadable]: {safe}.md: not found\n")
    );
}

/// Discovery refuses a FIFO before any read, so the run never blocks and says "not found".
#[cfg(unix)]
#[rstest]
#[case::esc(ESC_RAW, ESC_SAFE)]
#[case::newline(FORGE_RAW, FORGE_SAFE)]
#[case::bidi(BIDI_RAW, BIDI_SAFE)]
fn a_fifo_target_is_refused_without_blocking_and_with_an_escaped_name(
    #[case] raw: &str,
    #[case] safe: &str,
) {
    let file = format!("{raw}.md");
    let err = refusal(
        |dir| {
            assert!(
                std::process::Command::new("mkfifo")
                    .arg(dir.join(&file))
                    .status()
                    .expect("mkfifo runs")
                    .success()
            );
        },
        &[&file],
    );
    assert_eq!(
        err,
        format!("error[input-unreadable]: {safe}.md: not found\n")
    );
}

#[test]
fn an_invalid_glob_is_refused_with_an_escaped_pattern() {
    let err = refusal(|_| {}, &["a\u{1b}[b.md"]);
    assert_eq!(err, "error[input-unreadable]: invalid glob a\\u{1b}[b.md\n");
}

#[rstest]
#[case::esc(ESC_RAW, ESC_SAFE)]
#[case::newline(FORGE_RAW, FORGE_SAFE)]
#[case::bidi(BIDI_RAW, BIDI_SAFE)]
fn a_missing_config_is_refused_with_an_escaped_name(#[case] raw: &str, #[case] safe: &str) {
    let file = format!("{raw}.toml");
    let err = refusal(
        |dir| std::fs::write(dir.join("ok.md"), "text\n").expect("write"),
        &["--config", &file, "ok.md"],
    );
    assert_eq!(
        err,
        format!("error[config-not-found]: --config {safe}.toml: no such file\n")
    );
}

/// An existing config that is not valid UTF-8 is refused under its own name.
#[rstest]
#[case::esc(ESC_RAW, ESC_SAFE)]
#[case::newline(FORGE_RAW, FORGE_SAFE)]
#[case::bidi(BIDI_RAW, BIDI_SAFE)]
fn an_unreadable_config_is_refused_with_an_escaped_name(#[case] raw: &str, #[case] safe: &str) {
    let file = format!("{raw}.toml");
    let err = refusal(
        |dir| {
            std::fs::write(dir.join(&file), [0xff, 0xfe, b'\n']).expect("write");
            std::fs::write(dir.join("ok.md"), "text\n").expect("write");
        },
        &["--config", &file, "ok.md"],
    );
    assert_eq!(
        err,
        format!("error[config-io]: cannot read {safe}.toml: not valid UTF-8\n")
    );
}

/// A config that is refused after parsing: the unknown category is reported with the file's name.
#[rstest]
#[case::esc(ESC_RAW, ESC_SAFE)]
#[case::newline(FORGE_RAW, FORGE_SAFE)]
#[case::bidi(BIDI_RAW, BIDI_SAFE)]
fn a_rejected_config_is_refused_with_an_escaped_name(#[case] raw: &str, #[case] safe: &str) {
    let file = format!("{raw}.toml");
    let err = refusal(
        |dir| {
            std::fs::write(
                dir.join(&file),
                "schema_version = 3\n\n[categories]\nnope = \"off\"\n",
            )
            .expect("write");
            std::fs::write(dir.join("ok.md"), "text\n").expect("write");
        },
        &["--config", &file, "ok.md"],
    );
    assert_eq!(
        err,
        format!(
            "error[config-unknown-category]: project {safe}.toml:4:1: unknown category \
             \"nope\"; use a category from `slopvac rules`\n"
        )
    );
}

/// A terms file that cannot be read is refused under its own name.
#[rstest]
#[case::esc(ESC_RAW, ESC_SAFE)]
#[case::newline(FORGE_RAW, FORGE_SAFE)]
#[case::bidi(BIDI_RAW, BIDI_SAFE)]
fn a_bad_terms_file_is_refused_with_an_escaped_name(#[case] raw: &str, #[case] safe: &str) {
    let file = format!("{raw}.toml");
    let err = refusal(
        |dir| {
            let name = toml::Value::String(file.clone());
            std::fs::write(
                dir.join("slopvac.toml"),
                format!("schema_version = 3\n\n[terms]\nfiles = [{name}]\n"),
            )
            .expect("write");
            std::fs::write(dir.join("ok.md"), "text\n").expect("write");
        },
        &["ok.md"],
    );
    assert_eq!(
        err,
        format!("error[terms-invalid]: {safe}.toml: cannot load the terms file: cannot be read\n")
    );
}

#[rstest]
#[case::esc(ESC_RAW, ESC_SAFE)]
#[case::newline(FORGE_RAW, FORGE_SAFE)]
#[case::bidi(BIDI_RAW, BIDI_SAFE)]
fn an_rst_conversion_failure_is_reported_with_an_escaped_name(
    #[case] raw: &str,
    #[case] safe: &str,
) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = format!("{raw}.rst");
    std::fs::write(dir.path().join(&file), "Title\n=====\n\ntext\n").expect("write");
    let converter = dir.path().join("no-such-python");
    let cmd = slopvac(dir.path()).env("SLOPVAC_RST_CONVERTER", &converter);
    let (_, err) = run_in(cmd, &["lint", &file]);
    // One error line: a newline in the name never starts another.
    assert_eq!(err.lines().count(), 1, "{err:?}");
    assert!(
        err.starts_with(&format!("error[check-incomplete]: {safe}.rst: ")),
        "{err:?}"
    );
    assert!(
        !err.trim_end()
            .chars()
            .any(|c| c.is_control() || c == '\u{202e}'),
        "{err:?}"
    );
    assert!(!err.contains("\\\\u{"), "escaped twice: {err:?}");
}

#[rstest]
#[case::hostile_extension("x.\u{1b}]0;x\u{7}b", "'.\\u{1b}]0;x\\u{7}b'")]
#[case::newline_extension("x.a\nb", "'.a\\u{a}b'")]
#[case::bidi_extension("x.a\u{202e}b", "'.a\\u{202e}b'")]
#[case::extensionless_esc("\u{1b}]0;x\u{7}b", "'\\u{1b}]0;x\\u{7}b'")]
#[case::extensionless_newline("a\nb", "'a\\u{a}b'")]
#[case::extensionless_bidi("a\u{202e}b", "'a\\u{202e}b'")]
fn an_unsupported_source_is_refused_with_an_escaped_reason(
    #[case] file: &str,
    #[case] shown_reason: &str,
) {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join(file), "x\n").expect("write");
    // A plain unsupported name gives the supported-extension list this build prints.
    std::fs::write(dir.path().join("plain.zzz"), "x\n").expect("write");
    let (_, baseline) = run_in(
        slopvac(dir.path()),
        &["lint", "--mode", "code-comments", "plain.zzz"],
    );
    let list = baseline
        .split_once("supported extensions: ")
        .expect("baseline names the list")
        .1;
    let (_, err) = run_in(
        slopvac(dir.path()),
        &["lint", "--mode", "code-comments", file],
    );
    assert_eq!(
        err,
        format!(
            "error[input-unreadable]: {}: unsupported source language for \
             {shown_reason}; supported extensions: {list}",
            escaped(file)
        )
    );
}

#[test]
fn a_notice_is_escaped_once_in_the_text_report_and_raw_in_json() {
    let file = format!("{FORGE_RAW}.md");
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::File::create(dir.path().join(&file))
        .and_then(|f| f.set_len(CAP + 1))
        .expect("sparse file");
    let (text, _) = run_in(slopvac(dir.path()), &["lint", &file]);
    let want = format!("{FORGE_SAFE}.md: larger than the 64 MiB document limit");
    assert!(text.contains(&want), "{text:?}");
    assert!(!text.contains("\\\\u{"), "escaped twice: {text:?}");

    let (json, _) = run_in(slopvac(dir.path()), &["lint", "--format", "json", &file]);
    let report: serde_json::Value = serde_json::from_str(&json).expect("json report");
    let notice = &report["notices"][0];
    assert_eq!(
        notice["message"],
        format!("{FORGE_RAW}.md: larger than the 64 MiB document limit")
    );
    assert_eq!(notice["path"], file);
}

/// A config refusal reaches the report as raw text too, so the text report escapes it once.
#[test]
fn a_config_notice_is_escaped_once_in_the_text_report_and_raw_in_json() {
    let file = format!("{FORGE_RAW}.toml");
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join(&file), [0xff, 0xfe, b'\n']).expect("write");
    std::fs::write(dir.path().join("ok.md"), "text\n").expect("write");
    let (text, _) = run_in(slopvac(dir.path()), &["lint", "--config", &file, "ok.md"]);
    assert!(text.contains(FORGE_SAFE), "{text:?}");
    assert!(!text.contains("\\\\u{"), "escaped twice: {text:?}");
    let (json, _) = run_in(
        slopvac(dir.path()),
        &["lint", "--format", "json", "--config", &file, "ok.md"],
    );
    let report: serde_json::Value = serde_json::from_str(&json).expect("json report");
    let message = report["notices"][0]["message"].as_str().expect("message");
    assert!(message.contains(FORGE_RAW), "{message:?}");
}

/// A config that fails schema validation, so the refusal comes from the rejected-config
/// renderer. The file name and the quoted value are both hostile. `toml_value` is `raw` as a
/// TOML basic-string body.
#[rstest]
#[case::esc(ESC_RAW, ESC_SAFE, "a\\u001b]0;x\\u0007b")]
#[case::newline(FORGE_RAW, FORGE_SAFE, "a\\n::error::forged")]
#[case::bidi(BIDI_RAW, BIDI_SAFE, "a\\u202eb")]
fn a_schema_invalid_config_escapes_its_name_and_value_once(
    #[case] raw: &str,
    #[case] safe: &str,
    #[case] toml_value: &str,
) {
    let file = format!("{raw}.toml");
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join(&file),
        format!("schema_version = 3\nlocale = \"{toml_value}\"\n"),
    )
    .expect("write");
    std::fs::write(dir.path().join("ok.md"), "text\n").expect("write");
    let args = ["--config", &file, "ok.md"];

    let (_, err) = run_in(slopvac(dir.path()), &[&["lint"], &args[..]].concat());
    assert_eq!(
        err,
        format!(
            "error[config-invalid-value]: `{safe}` is not an accepted value\n \
             --> {safe}.toml:2:10\nhelp: use one of `en-US`, `en-GB`, `und`\n"
        )
    );

    // The report notice keeps the raw text for the format that escapes it.
    let (json, _) = run_in(
        slopvac(dir.path()),
        &[&["lint", "--format", "json"], &args[..]].concat(),
    );
    let report: serde_json::Value = serde_json::from_str(&json).expect("json report");
    let message = report["notices"][0]["message"].as_str().expect("message");
    assert!(
        message.contains(&format!("`{raw}` is not an accepted value")),
        "{message:?}"
    );
    assert!(
        message.contains(&format!("--> {raw}.toml:2:10")),
        "{message:?}"
    );
}
