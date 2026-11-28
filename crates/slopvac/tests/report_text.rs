//! The text of `slopvac report` that the message catalog owns: each entry keeps its published
//! words and is registered as text under its code.
//!
//! Expected strings are written out by hand, apart from the templates in the catalog.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use rstest::rstest;
use slopvac_cli::messages::catalogs;
use slopvac_cli::report::gate::Kind as GateKind;
use slopvac_cli::report::gh::GhError;
use slopvac_cli::report::issue::Report;
use slopvac_cli::report::messages::{ReportPhrase, ReportText};
use slopvac_messages::{Catalog, Kind, render};

fn assert_registered_text(code: &str) {
    let found = catalogs()
        .into_iter()
        .flat_map(|(_, entries)| entries.iter())
        .find(|e| e.code == code)
        .unwrap_or_else(|| panic!("`{code}` is not in the registry"));
    assert_eq!(found.kind, Kind::Text, "{code}");
}

#[rstest]
#[case::about(
    ReportPhrase::About,
    "cli-report-about",
    "Preview an issue report, and file it with the GitHub CLI."
)]
#[case::category_help(
    ReportPhrase::CategoryHelp,
    "cli-report-category-help",
    "The kind of report; it picks the issue form."
)]
#[case::title_help(
    ReportPhrase::TitleHelp,
    "cli-report-title-help",
    "The issue title, without the category prefix."
)]
#[case::field_help(
    ReportPhrase::FieldHelp,
    "cli-report-field-help",
    "A form field. Repeat for several fields, or several dropdown options."
)]
#[case::include_text_help(
    ReportPhrase::IncludeTextHelp,
    "cli-report-include-text-help",
    "Read the document sample for the `input` field from this file."
)]
#[case::body_file_help(
    ReportPhrase::BodyFileHelp,
    "cli-report-body-file-help",
    "Write the previewed body to this file. The write replaces the file in one step."
)]
#[case::gh_help(
    ReportPhrase::GhHelp,
    "cli-report-gh-help",
    "Print the exact `gh issue create` command for the body file."
)]
#[case::yes_help(
    ReportPhrase::YesHelp,
    "cli-report-yes-help",
    "With --gh: run that command after the preview. Offline mode refuses it."
)]
#[case::open_help(
    ReportPhrase::OpenHelp,
    "cli-report-open-help",
    "Open the prefilled issue form in a browser."
)]
#[case::consent_about(
    ReportPhrase::ConsentAbout,
    "cli-report-consent-about",
    "Record whether agents may report slopvac problems for you."
)]
#[case::consent_on(
    ReportPhrase::ConsentOnAbout,
    "cli-report-consent-on-about",
    "Allow agents to report slopvac problems."
)]
#[case::consent_off(
    ReportPhrase::ConsentOffAbout,
    "cli-report-consent-off-about",
    "Do not allow agents to report slopvac problems."
)]
#[case::consent_show(
    ReportPhrase::ConsentShowAbout,
    "cli-report-consent-show-about",
    "Print the recorded answer and when it was recorded."
)]
#[case::body_file_stdin(
    ReportPhrase::BodyFileStdin,
    "cli-report-body-file-stdin",
    "`-` means standard input to gh; write ./- for a file of that name"
)]
#[case::body_file_not_utf8(
    ReportPhrase::BodyFileNotUtf8,
    "cli-report-body-file-not-utf8",
    "the path must be UTF-8 without control characters, so the command can be printed"
)]
#[case::body_file_create(
    ReportPhrase::BodyFileCreate,
    "cli-report-body-file-create",
    "cannot create a file there"
)]
#[case::body_file_write(
    ReportPhrase::BodyFileWrite,
    "cli-report-body-file-write",
    "cannot write the file"
)]
#[case::body_file_replace(
    ReportPhrase::BodyFileReplace,
    "cli-report-body-file-replace",
    "cannot replace that path"
)]
#[case::gh_timed_out(
    ReportPhrase::GhTimedOut,
    "cli-report-gh-timed-out",
    "it failed or timed out"
)]
#[case::gh_missing(
    ReportPhrase::GhMissingShort,
    "cli-report-gh-missing-short",
    "the GitHub CLI `gh` was not found on PATH"
)]
#[case::gh_login(
    ReportPhrase::GhNotLoggedInShort,
    "cli-report-gh-not-logged-in-short",
    "`gh auth status` failed; run `gh auth login`"
)]
#[case::gh_failed_timeout(
    ReportPhrase::GhFailedTimeout,
    "cli-report-gh-failed-timeout",
    "`gh issue create` failed or timed out"
)]
#[case::sample_open(
    ReportPhrase::SampleOpen,
    "cli-report-sample-open",
    "cannot open the file"
)]
#[case::sample_not_regular(
    ReportPhrase::SampleNotRegular,
    "cli-report-sample-not-regular",
    "not a regular file"
)]
#[case::sample_read(
    ReportPhrase::SampleRead,
    "cli-report-sample-read",
    "cannot read the file"
)]
#[case::sample_too_large(
    ReportPhrase::SampleTooLarge,
    "cli-report-sample-too-large",
    "the file is larger than 256 KiB"
)]
#[case::sample_not_utf8(
    ReportPhrase::SampleNotUtf8,
    "cli-report-sample-not-utf8",
    "the file is not valid UTF-8"
)]
#[case::field_syntax(
    ReportPhrase::InvalidFieldSyntax,
    "cli-report-invalid-field-syntax",
    "--field takes ID=VALUE"
)]
#[case::input_field(
    ReportPhrase::InvalidInputField,
    "cli-report-invalid-input-field",
    "the `input` field holds document text; pass it with --include-text FILE so the preview shows it"
)]
#[case::control(
    ReportPhrase::InvalidControl,
    "cli-report-invalid-control",
    "a field or the document text holds a control character such as an escape sequence"
)]
#[case::title_line(
    ReportPhrase::InvalidTitleLine,
    "cli-report-invalid-title-line",
    "--title takes one non-empty line"
)]
#[case::title_control(
    ReportPhrase::InvalidTitleControl,
    "cli-report-invalid-title-control",
    "--title holds a control character"
)]
#[case::kind_credential(
    ReportPhrase::KindCredential,
    "cli-report-kind-credential",
    "credential"
)]
#[case::kind_repository(
    ReportPhrase::KindRepository,
    "cli-report-kind-repository",
    "private repository"
)]
#[case::kind_path(ReportPhrase::KindPath, "cli-report-kind-path", "private path")]
#[case::kind_local_name(
    ReportPhrase::KindLocalName,
    "cli-report-kind-local-name",
    "local name"
)]
#[case::body_written(
    ReportPhrase::BodyWritten,
    "cli-report-body-written",
    "Wrote the body to the file you named."
)]
#[case::footer_not_posted(
    ReportPhrase::FooterNotPosted,
    "cli-report-footer-not-posted",
    "Nothing was posted. Add --body-file FILE --gh to print the GitHub CLI command, and --yes to run it."
)]
fn a_phrase_keeps_its_text_and_is_registered(
    #[case] phrase: ReportPhrase,
    #[case] code: &str,
    #[case] text: &str,
) {
    assert_eq!(phrase.code(), code);
    assert_eq!(phrase.as_static(), text);
    assert_registered_text(code);
}

#[rstest]
#[case::long_about(
    ReportPhrase::LongAbout,
    "cli-report-long-about",
    "Preview an issue report for the slopvac repository. The command prints the exact target, title and body,"
)]
#[case::consent_long(
    ReportPhrase::ConsentLongAbout,
    "cli-report-consent-long-about",
    "Record, for your user account, whether an agent may report slopvac problems. `on` and `off` write"
)]
fn a_long_help_phrase_starts_as_published_and_is_registered(
    #[case] phrase: ReportPhrase,
    #[case] code: &str,
    #[case] start: &str,
) {
    assert_eq!(phrase.code(), code);
    assert!(phrase.as_static().starts_with(start), "{code}");
    assert_registered_text(code);
}

#[rstest]
#[case::gh_exit(ReportText::GhExit { code: "3".into() }, "cli-report-gh-exit", "exit 3")]
#[case::gh_failed_exit(ReportText::GhFailedExit { code: "3".into() }, "cli-report-gh-failed-exit", "`gh issue create` failed (exit 3)")]
#[case::unknown_field(
    ReportText::InvalidUnknownField { category: "bug".into(), known: "agent, model".into() },
    "cli-report-invalid-unknown-field",
    "unknown field id for `bug`; its fields are: agent, model"
)]
#[case::one_option(ReportText::InvalidOneOption { id: "latest".into() }, "cli-report-invalid-one-option", "`latest` takes one option")]
#[case::options(
    ReportText::InvalidOptions { id: "latest".into(), options: "Yes | No".into() },
    "cli-report-invalid-options",
    "`latest` must be one of: Yes | No"
)]
#[case::single_line(ReportText::InvalidSingleLine { id: "command".into() }, "cli-report-invalid-single-line", "`command` is a single line")]
#[case::repeated(ReportText::InvalidRepeated { id: "expected".into() }, "cli-report-invalid-repeated", "`expected` was given more than once")]
#[case::title_long(
    ReportText::InvalidTitleLong { max: 200 },
    "cli-report-invalid-title-long",
    "--title is longer than 200 characters; GitHub caps a title at 256 with its prefix"
)]
#[case::no_text_field(
    ReportText::InvalidNoTextField { category: "idea".into() },
    "cli-report-invalid-no-text-field",
    "`idea` has no document text field, so --include-text does not apply"
)]
#[case::possible(
    ReportText::Possible { kind: "private path".into(), field: "command".into() },
    "cli-report-possible",
    "possible private path in field `command`: review it before posting"
)]
#[case::preview(
    ReportText::Preview { repository: "slopvac/slopvac".into(), title: "[bug]: t".into(), body: "B".into() },
    "cli-report-preview",
    "Target: slopvac/slopvac\nTitle: [bug]: t\n\nB"
)]
#[case::warning(ReportText::Warning { text: "w".into() }, "cli-report-warning", "warning: w")]
#[case::gh_command(ReportText::GhCommand { line: "gh x".into() }, "cli-report-gh-command", "GitHub CLI command:\ngh x")]
#[case::created(ReportText::Created { url: "https://x/1".into() }, "cli-report-created", "Created: https://x/1")]
#[case::footer_url(ReportText::FooterUrl { url: "https://x/y".into() }, "cli-report-footer-url", "Prefilled issue form: https://x/y")]
#[case::footer_labels(ReportText::FooterLabels { labels: "a, b".into() }, "cli-report-footer-labels", "The issue form adds the labels: a, b")]
#[case::footer_omitted(
    ReportText::FooterOmitted { fields: "x, y".into() },
    "cli-report-footer-omitted",
    "The URL is too long to hold these fields; paste them from the preview: x, y"
)]
#[case::output_failed(ReportText::OutputFailed { kind: "broken pipe".into() }, "cli-report-output-failed", "cannot write output: broken pipe")]
fn a_text_with_parameters_renders_as_published_and_is_registered(
    #[case] text: ReportText,
    #[case] code: &str,
    #[case] expected: &str,
) {
    assert_eq!(text.code(), code);
    assert_eq!(render(&text), expected);
    assert_registered_text(code);
}

#[rstest]
#[case::credential(GateKind::Credential, "credential")]
#[case::repository(GateKind::Repository, "private repository")]
#[case::path(GateKind::Path, "private path")]
#[case::local_name(GateKind::LocalName, "local name")]
fn a_gate_finding_is_named_from_the_catalog(#[case] kind: GateKind, #[case] label: &str) {
    assert_eq!(kind.label(), label);
}

#[rstest]
#[case::missing(GhError::Missing, "the GitHub CLI `gh` was not found on PATH")]
#[case::login(GhError::NotLoggedIn, "`gh auth status` failed; run `gh auth login`")]
#[case::exit(GhError::Failed(Some(4)), "`gh issue create` failed (exit 4)")]
#[case::timeout(GhError::Failed(None), "`gh issue create` failed or timed out")]
fn a_gh_error_displays_from_the_catalog(#[case] error: GhError, #[case] text: &str) {
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
