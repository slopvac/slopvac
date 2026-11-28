//! The text of `slopvac report` that the message catalog owns.
//!
//! The catalog tests pin the wording of each entry. The binary tests pin the words a user sees,
//! so a call site that picks the wrong entry fails. Every expected value is written out by hand.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use assert_cmd::Command;
use rstest::rstest;
use slopvac_cli::report::gate::Kind as GateKind;
use slopvac_cli::report::gh::GhError;
use slopvac_cli::report::issue::Report;
use slopvac_cli::report::messages::{ReportPhrase, ReportText};
use slopvac_messages::render;

#[rstest]
#[case::no_response(ReportPhrase::NoResponse, "_No response_")]
#[case::body_file_stdin(
    ReportPhrase::BodyFileStdin,
    "`-` means standard input to gh; write ./- for a file of that name"
)]
#[case::body_file_not_utf8(
    ReportPhrase::BodyFileNotUtf8,
    "the path must be UTF-8 without control characters, so the command can be printed"
)]
#[case::body_file_create(ReportPhrase::BodyFileCreate, "cannot create a file there")]
#[case::body_file_write(ReportPhrase::BodyFileWrite, "cannot write the file")]
#[case::body_file_replace(ReportPhrase::BodyFileReplace, "cannot replace that path")]
#[case::gh_timed_out(ReportPhrase::GhTimedOut, "it failed or timed out")]
#[case::gh_missing(
    ReportPhrase::GhMissingShort,
    "the GitHub CLI `gh` was not found on PATH"
)]
#[case::gh_login(
    ReportPhrase::GhNotLoggedInShort,
    "`gh auth status` failed; run `gh auth login`"
)]
#[case::gh_failed_timeout(ReportPhrase::GhFailedTimeout, "`gh issue create` failed or timed out")]
#[case::sample_open(ReportPhrase::SampleOpen, "cannot open the file")]
#[case::sample_not_regular(ReportPhrase::SampleNotRegular, "not a regular file")]
#[case::sample_read(ReportPhrase::SampleRead, "cannot read the file")]
#[case::sample_too_large(ReportPhrase::SampleTooLarge, "the file is larger than 256 KiB")]
#[case::sample_not_utf8(ReportPhrase::SampleNotUtf8, "the file is not valid UTF-8")]
#[case::field_syntax(ReportPhrase::InvalidFieldSyntax, "--field takes ID=VALUE")]
#[case::input_field(
    ReportPhrase::InvalidInputField,
    "the `input` field holds document text; pass it with --include-text FILE so the preview shows it"
)]
#[case::control(
    ReportPhrase::InvalidControl,
    "a field or the document text holds a control character such as an escape sequence"
)]
#[case::title_line(ReportPhrase::InvalidTitleLine, "--title takes one non-empty line")]
#[case::title_control(ReportPhrase::InvalidTitleControl, "--title holds a control character")]
#[case::body_written(ReportPhrase::BodyWritten, "Wrote the body to the file you named.")]
#[case::footer_not_posted(
    ReportPhrase::FooterNotPosted,
    "Nothing was posted. Add --body-file FILE --gh to print the GitHub CLI command, and --yes to run it."
)]
fn a_phrase_renders_as_published(#[case] phrase: ReportPhrase, #[case] text: &str) {
    assert_eq!(render(&phrase), text);
}

#[rstest]
#[case::gh_exit(ReportText::GhExit { code: "3".into() }, "exit 3")]
#[case::gh_failed_exit(ReportText::GhFailedExit { code: "3".into() }, "`gh issue create` failed (exit 3)")]
#[case::unknown_field(
    ReportText::InvalidUnknownField { category: "bug".into(), known: "agent, model".into() },
    "unknown field id for `bug`; its fields are: agent, model"
)]
#[case::one_option(ReportText::InvalidOneOption { id: "latest".into() }, "`latest` takes one option")]
#[case::options(
    ReportText::InvalidOptions { id: "latest".into(), options: "Yes | No".into() },
    "`latest` must be one of: Yes | No"
)]
#[case::single_line(ReportText::InvalidSingleLine { id: "command".into() }, "`command` is a single line")]
#[case::repeated(ReportText::InvalidRepeated { id: "expected".into() }, "`expected` was given more than once")]
#[case::title_long(
    ReportText::InvalidTitleLong { max: 200 },
    "--title is longer than 200 characters; GitHub caps a title at 256 with its prefix"
)]
#[case::no_text_field(
    ReportText::InvalidNoTextField { category: "idea".into() },
    "`idea` has no document text field, so --include-text does not apply"
)]
#[case::possible(
    ReportText::Possible { kind: "private path".into(), field: "command".into() },
    "possible private path in field `command`: review it before posting"
)]
#[case::preview(ReportText::Preview { repository: "slopvac/slopvac".into() }, "Target: slopvac/slopvac\nTitle:")]
#[case::warning(ReportText::Warning { text: "w".into() }, "warning: w")]
#[case::gh_command(ReportText::GhCommand, "GitHub CLI command:")]
#[case::created(ReportText::Created, "Created:")]
#[case::footer_url(ReportText::FooterUrl, "Prefilled issue form:")]
#[case::footer_labels(ReportText::FooterLabels { labels: "a, b".into() }, "The issue form adds the labels: a, b")]
#[case::footer_omitted(
    ReportText::FooterOmitted { fields: "x, y".into() },
    "The URL is too long to hold these fields; paste them from the preview: x, y"
)]
#[case::output_failed(ReportText::OutputFailed { kind: "broken pipe".into() }, "cannot write output: broken pipe")]
fn a_text_with_parameters_renders_as_published(#[case] text: ReportText, #[case] expected: &str) {
    assert_eq!(render(&text), expected);
}

#[rstest]
#[case::credential(GateKind::Credential, "credential")]
#[case::repository(GateKind::Repository, "private repository")]
#[case::path(GateKind::Path, "private path")]
#[case::local_name(GateKind::LocalName, "local name")]
fn a_gate_finding_is_named_as_published(#[case] kind: GateKind, #[case] label: &str) {
    assert_eq!(kind.label(), label);
}

#[rstest]
#[case::missing(GhError::Missing, "the GitHub CLI `gh` was not found on PATH")]
#[case::login(GhError::NotLoggedIn, "`gh auth status` failed; run `gh auth login`")]
#[case::exit(GhError::Failed(Some(4)), "`gh issue create` failed (exit 4)")]
#[case::timeout(GhError::Failed(None), "`gh issue create` failed or timed out")]
fn a_gh_error_displays_as_published(#[case] error: GhError, #[case] text: &str) {
    assert_eq!(error.to_string(), text);
}

#[test]
fn the_preview_prints_target_title_and_body() {
    let report = Report {
        title: "[bug]: crash".to_owned(),
        body: "### What\n\nit broke".to_owned(),
        answers: std::collections::BTreeMap::new(),
        warnings: Vec::new(),
    };
    assert_eq!(
        report.preview(),
        "Target: slopvac/slopvac\nTitle: [bug]: crash\n\n### What\n\nit broke\n"
    );
}

#[test]
fn an_empty_optional_field_reads_no_response_in_the_body() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(dir.path(), &full_bug(&[]));
    // `output` is an optional textarea of the bug form and the run gives it no answer.
    assert!(
        out.stdout.contains("### Output\n\n_No response_\n"),
        "{}",
        out.stdout
    );
}

// ---- The built binary ----

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run(dir: &Path, args: &[String]) -> Run {
    run_with(dir, args, &[])
}

fn run_with(dir: &Path, args: &[String], env: &[(&str, &str)]) -> Run {
    let mut cmd = Command::cargo_bin("slopvac").unwrap();
    cmd.current_dir(dir)
        .env("PATH", dir)
        .env("NO_COLOR", "1")
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .env_remove("SLOPVAC_OFFLINE")
        .env_remove("SLOPVAC_LOG")
        .arg("report")
        .args(args);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let output = cmd.output().unwrap();
    Run {
        code: output.status.code(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_owned()).collect()
}

/// Every required answer of the bug form, so a run reaches its output.
fn full_bug(extra: &[&str]) -> Vec<String> {
    let mut args = strings(&[
        "bug",
        "--title",
        "crash on empty input",
        "--field",
        "agent=Claude Code",
        "--field",
        "model=test-model",
        "--field",
        "harness=test-harness 1.0",
        "--field",
        "latest=Yes",
        "--field",
        "duplicates=true",
        "--field",
        "environment={\"ok\":true}",
        "--field",
        "command=slopvac lint --stdin",
        "--field",
        "what-happened=it crashed",
        "--field",
        "expected=no crash",
        "--field",
        "reporter=true",
    ]);
    args.extend(extra.iter().map(|s| (*s).to_owned()));
    args
}

fn fake_gh(dir: &Path, script: &str) {
    let gh = dir.join("gh");
    std::fs::write(&gh, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The `error[...]` line on stderr.
fn error_line(run: &Run) -> &str {
    assert_eq!(run.code, Some(2), "{}", run.stderr);
    run.stderr
        .lines()
        .find(|l| l.starts_with("error["))
        .unwrap_or_else(|| panic!("no error line in {:?}", run.stderr))
}

#[rstest]
#[case::no_equals(&["bug", "--title", "x", "--field", "no-equals"], "--field takes ID=VALUE")]
#[case::unknown_field(
    &["bug", "--title", "x", "--field", "nope=1"],
    "unknown field id for `bug`; its fields are: reporter, agent, model, harness, version, latest, duplicates, environment, command, what-happened, expected, output, config"
)]
#[case::input_field(
    &["false-positive", "--title", "x", "--field", "input=y"],
    "the `input` field holds document text; pass it with --include-text FILE so the preview shows it"
)]
#[case::dropdown_twice(
    &["bug", "--title", "x", "--field", "latest=Yes", "--field", "latest=Not checked"],
    "`latest` takes one option"
)]
#[case::dropdown_value(
    &["bug", "--title", "x", "--field", "latest=Maybe"],
    "`latest` must be one of: Yes | Not checked"
)]
#[case::input_lines(
    &["bug", "--title", "x", "--field", "agent=a\nb"],
    "`agent` is a single line"
)]
#[case::textarea_twice(
    &["bug", "--title", "x", "--field", "command=a", "--field", "command=b"],
    "`command` was given more than once"
)]
#[case::field_control(
    &["bug", "--title", "x", "--field", "agent=a\u{1b}b"],
    "a field or the document text holds a control character such as an escape sequence"
)]
#[case::title_empty(&["bug", "--title", " "], "--title takes one non-empty line")]
#[case::title_lines(&["bug", "--title", "a\nb"], "--title takes one non-empty line")]
#[case::title_control(&["bug", "--title", "a\u{1b}b"], "--title holds a control character")]
fn a_refused_report_input_names_its_problem(#[case] args: &[&str], #[case] detail: &str) {
    let dir = tempfile::tempdir().unwrap();
    let out = run(dir.path(), &strings(args));
    assert_eq!(
        error_line(&out),
        format!("error[report-invalid]: invalid report input: {detail}")
    );
}

#[test]
fn a_title_over_the_limit_is_refused_with_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let long = "a".repeat(201);
    let out = run(dir.path(), &strings(&["bug", "--title", &long]));
    assert_eq!(
        error_line(&out),
        "error[report-invalid]: invalid report input: --title is longer than 200 characters; GitHub caps a title at 256 with its prefix"
    );
}

#[test]
fn a_sample_for_a_form_without_a_document_field_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let sample = dir.path().join("sample.txt");
    std::fs::write(&sample, "text").unwrap();
    let out = run(
        dir.path(),
        &full_bug(&["--include-text", sample.to_str().unwrap()]),
    );
    assert_eq!(
        error_line(&out),
        "error[report-invalid]: invalid report input: `bug` has no document text field, so --include-text does not apply"
    );
}

#[rstest]
#[case::missing("missing", "cannot open the file")]
#[case::directory("directory", "not a regular file")]
#[case::too_large("large", "the file is larger than 256 KiB")]
#[case::not_utf8("binary", "the file is not valid UTF-8")]
fn an_unusable_sample_gives_its_reason(#[case] kind: &str, #[case] reason: &str) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample");
    match kind {
        "directory" => std::fs::create_dir(&path).unwrap(),
        "large" => std::fs::write(&path, vec![b'a'; 256 * 1024 + 1]).unwrap(),
        "binary" => std::fs::write(&path, [0xff, 0xfe, 0xfd]).unwrap(),
        _ => {}
    }
    let out = run(
        dir.path(),
        &strings(&[
            "bug",
            "--title",
            "x",
            "--include-text",
            path.to_str().unwrap(),
        ]),
    );
    assert_eq!(
        error_line(&out),
        format!("error[report-sample-unreadable]: --include-text: {reason}")
    );
}

#[test]
fn a_body_file_of_dash_is_refused_because_gh_reads_standard_input() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(
        dir.path(),
        &strings(&["bug", "--title", "x", "--body-file", "-"]),
    );
    assert_eq!(
        error_line(&out),
        "error[report-body-file]: --body-file: `-` means standard input to gh; write ./- for a file of that name"
    );
}

#[test]
fn a_body_file_in_a_missing_directory_cannot_be_created() {
    let dir = tempfile::tempdir().unwrap();
    let body = dir.path().join("absent").join("body.md");
    let out = run(
        dir.path(),
        &full_bug(&["--body-file", body.to_str().unwrap()]),
    );
    assert_eq!(
        error_line(&out),
        "error[report-body-file]: --body-file: cannot create a file there"
    );
}

#[test]
fn a_body_file_that_is_a_directory_cannot_be_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let body = dir.path().join("body.md");
    std::fs::create_dir(&body).unwrap();
    let out = run(
        dir.path(),
        &full_bug(&["--body-file", body.to_str().unwrap()]),
    );
    assert_eq!(
        error_line(&out),
        "error[report-body-file]: --body-file: cannot replace that path"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_body_file_path_that_is_not_utf8_cannot_be_printed_in_a_command() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let dir = tempfile::tempdir().unwrap();
    let body = dir.path().join(OsString::from_vec(b"b\xffody.md".to_vec()));
    let mut cmd = Command::cargo_bin("slopvac").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("PATH", dir.path())
        .env("NO_COLOR", "1")
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .env_remove("SLOPVAC_OFFLINE")
        .arg("report")
        .args(full_bug(&[]))
        .arg("--body-file")
        .arg(&body)
        .arg("--gh")
        .output()
        .unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    let line = stderr.lines().find(|l| l.starts_with("error[")).unwrap();
    assert_eq!(
        line,
        "error[report-body-file]: --body-file: the path must be UTF-8 without control characters, so the command can be printed"
    );
}

#[rstest]
#[case::exit_status("[ \"$1\" = auth ] && exit 0\nexit 3", "exit 3")]
#[case::killed("[ \"$1\" = auth ] && exit 0\nkill -9 $$", "it failed or timed out")]
fn a_failed_gh_create_gives_its_reason(#[case] script: &str, #[case] reason: &str) {
    let dir = tempfile::tempdir().unwrap();
    fake_gh(dir.path(), script);
    let body = dir.path().join("body.md");
    let out = run(
        dir.path(),
        &full_bug(&["--body-file", body.to_str().unwrap(), "--gh", "--yes"]),
    );
    assert_eq!(
        error_line(&out),
        format!("error[report-gh-failed]: `gh issue create` failed: {reason}")
    );
}

#[test]
fn a_plain_run_ends_with_the_footer() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(dir.path(), &full_bug(&[]));
    assert_eq!(out.code, Some(0), "{}", out.stderr);
    let lines: Vec<&str> = out.stdout.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.starts_with("Nothing was posted."))
        .unwrap_or_else(|| panic!("no footer in {}", out.stdout));
    assert_eq!(
        lines[at],
        "Nothing was posted. Add --body-file FILE --gh to print the GitHub CLI command, and --yes to run it."
    );
    assert!(
        lines[at + 1].starts_with(
            "Prefilled issue form: https://github.com/slopvac/slopvac/issues/new?template=bug.yml&title="
        ),
        "{}",
        lines[at + 1]
    );
    assert_eq!(
        lines[at + 2],
        "The issue form adds the labels: bug, needs-triage"
    );
    assert_eq!(lines.len(), at + 3);
}

#[test]
fn a_field_too_long_for_the_url_is_named_in_the_footer() {
    let dir = tempfile::tempdir().unwrap();
    let mut args = full_bug(&[]);
    let at = args
        .iter()
        .position(|a| a.starts_with("what-happened="))
        .unwrap();
    args[at] = format!("what-happened={}", "word ".repeat(2000));
    let out = run(dir.path(), &args);
    assert_eq!(out.code, Some(0), "{}", out.stderr);
    assert!(
        out.stdout.lines().any(|l| l
            == "The URL is too long to hold these fields; paste them from the preview: what-happened"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_body_file_with_gh_prints_the_heading_then_the_command() {
    let dir = tempfile::tempdir().unwrap();
    let body = dir.path().join("body.md");
    let out = run(
        dir.path(),
        &full_bug(&["--body-file", body.to_str().unwrap(), "--gh"]),
    );
    assert_eq!(out.code, Some(0), "{}", out.stderr);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert!(lines.contains(&"Wrote the body to the file you named."));
    let at = lines
        .iter()
        .position(|l| *l == "GitHub CLI command:")
        .unwrap();
    assert!(
        lines[at + 1].starts_with(
            "gh issue create -R github.com/slopvac/slopvac --title '[bug]: crash on empty input' --body-file "
        ),
        "{}",
        lines[at + 1]
    );
}

#[test]
fn a_created_issue_prints_what_gh_printed() {
    let dir = tempfile::tempdir().unwrap();
    fake_gh(
        dir.path(),
        "[ \"$1\" = auth ] && exit 0\necho https://github.com/slopvac/slopvac/issues/7",
    );
    let body = dir.path().join("body.md");
    let out = run(
        dir.path(),
        &full_bug(&["--body-file", body.to_str().unwrap(), "--gh", "--yes"]),
    );
    assert_eq!(out.code, Some(0), "{}", out.stderr);
    assert_eq!(
        out.stdout.lines().last(),
        Some("Created: https://github.com/slopvac/slopvac/issues/7")
    );
}

#[test]
fn a_credential_in_the_title_is_warned_about() {
    let dir = tempfile::tempdir().unwrap();
    let mut args = full_bug(&[]);
    args[2] = "ghp_abcdefghijklmnopqrstuvwxyzABCDEFGHIJ".to_owned();
    let out = run(dir.path(), &args);
    assert_eq!(out.code, Some(0), "{}", out.stderr);
    assert!(
        out.stderr
            .lines()
            .any(|l| l == "warning: possible credential in field `title`: review it before posting"),
        "{:?}",
        out.stderr
    );
}

/// Document text is not path-cleaned, so a path or repository that stays in the sample is
/// warned about under the `input` field.
#[rstest]
#[case::path(
    "see ~/notes/todo\n",
    "warning: possible private path in field `input`: review it before posting"
)]
#[case::repository(
    "see //github.com/marker-priv/repo for it\n",
    "warning: possible private repository in field `input`: review it before posting"
)]
fn a_finding_in_the_sample_is_warned_about_by_its_kind(
    #[case] sample_text: &str,
    #[case] warning: &str,
) {
    let dir = tempfile::tempdir().unwrap();
    let sample = dir.path().join("sample.txt");
    std::fs::write(&sample, sample_text).unwrap();
    let mut args = strings(&[
        "false-positive",
        "--title",
        "flagged",
        "--field",
        "agent=a",
        "--field",
        "model=m",
        "--field",
        "harness=h",
        "--field",
        "latest=Yes",
        "--field",
        "duplicates=true",
        "--field",
        "environment=e",
        "--field",
        "command=c",
        "--field",
        "rule-id=core:x",
        "--field",
        "context=reference normal",
        "--field",
        "finding={}",
        "--field",
        "why=w",
        "--include-text",
    ]);
    args.push(sample.display().to_string());
    let out = run(dir.path(), &args);
    assert_eq!(out.code, Some(0), "{}", out.stderr);
    assert!(out.stderr.lines().any(|l| l == warning), "{:?}", out.stderr);
}

const REPORT_HELP: &str = concat!(
    "Preview an issue report for the slopvac repository. The command prints the exact target, title and body, with paths, user and host names and secrets removed as far as it can tell, and warns about text that may still hold a credential. It posts nothing. It prints a prefilled issue-form URL. --body-file writes the body to a file, --gh prints the exact `gh issue create` command for that file, and --gh --yes runs it through the GitHub CLI after you have read the preview. Slopvac never reads a token: `gh` holds the login. Run `slopvac prime report` for the workflow an agent follows, including the user's approval of the exact text.\n",
    "\n",
    "Usage: slopvac report [OPTIONS] --title <TEXT> <CATEGORY>\n",
    "       slopvac report <COMMAND>\n",
    "\n",
    "Commands:\n",
    "  consent  Record whether agents may report slopvac problems for you.\n",
    "  help     Print this message or the help of the given subcommand(s)\n",
    "\n",
    "Arguments:\n",
    "  <CATEGORY>\n",
    "          The kind of report; it picks the issue form.\n",
    "          \n",
    "          [possible values: bug, false-positive, missed-slop, mis-tagged, wrong-fix, new-rule, rule-improvement]\n",
    "\n",
    "Options:\n",
    "      --offline\n",
    "          Never use the network. SLOPVAC_OFFLINE=1 and `[network] offline = true` do the same, and nothing turns it off.\n",
    "\n",
    "      --color <WHEN>\n",
    "          Colour output. NO_COLOR is honoured.\n",
    "          \n",
    "          [possible values: auto, always, never]\n",
    "\n",
    "      --title <TEXT>\n",
    "          The issue title, without the category prefix.\n",
    "\n",
    "      --field <ID=VALUE>\n",
    "          A form field. Repeat for several fields, or several dropdown options.\n",
    "\n",
    "      --no-color\n",
    "          Alias for --color never.\n",
    "\n",
    "      --include-text <FILE>\n",
    "          Read the document sample for the `input` field from this file.\n",
    "\n",
    "      --body-file <FILE>\n",
    "          Write the previewed body to this file. The write replaces the file in one step.\n",
    "\n",
    "      --gh\n",
    "          Print the exact `gh issue create` command for the body file.\n",
    "\n",
    "      --yes\n",
    "          With --gh: run that command after the preview. Offline mode refuses it.\n",
    "\n",
    "      --open\n",
    "          Open the prefilled issue form in a browser.\n",
    "\n",
    "  -h, --help\n",
    "          Print help (see a summary with '-h')\n",
);

const CONSENT_HELP: &str = concat!(
    "Record, for your user account, whether an agent may report slopvac problems. `on` and `off` write the answer to a file in the state directory, and `show` prints it with the time it was recorded. The answer is unset until you record one. The command refuses to change the answer in CI.\n",
    "\n",
    "Usage: slopvac report consent [OPTIONS] <COMMAND>\n",
    "\n",
    "Commands:\n",
    "  on    Allow agents to report slopvac problems.\n",
    "  off   Do not allow agents to report slopvac problems.\n",
    "  show  Print the recorded answer and when it was recorded.\n",
    "  help  Print this message or the help of the given subcommand(s)\n",
    "\n",
    "Options:\n",
    "      --offline\n",
    "          Never use the network. SLOPVAC_OFFLINE=1 and `[network] offline = true` do the same, and nothing turns it off.\n",
    "\n",
    "      --color <WHEN>\n",
    "          Colour output. NO_COLOR is honoured.\n",
    "          \n",
    "          [possible values: auto, always, never]\n",
    "\n",
    "      --no-color\n",
    "          Alias for --color never.\n",
    "\n",
    "  -h, --help\n",
    "          Print help (see a summary with '-h')\n",
);

#[rstest]
#[case::report(&["report", "--help"], REPORT_HELP)]
#[case::consent(&["report", "consent", "--help"], CONSENT_HELP)]
fn the_long_help_reads_as_published(#[case] args: &[&str], #[case] help: &str) {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::cargo_bin("slopvac")
        .unwrap()
        .current_dir(dir.path())
        .env("NO_COLOR", "1")
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .args(args)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), help);
}
