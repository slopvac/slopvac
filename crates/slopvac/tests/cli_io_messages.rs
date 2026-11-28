//! The CLI's input and output messages: each is registered, locked and worded as published.

use slopvac_cli::messages::catalogs;
use slopvac_messages::{Entry, Kind};

const LOCK: &str = include_str!("../../../docs/reference/error-codes.lock");

fn entries() -> &'static [Entry] {
    catalogs()
        .into_iter()
        .find(|(name, _)| *name == "cli io")
        .map(|(_, entries)| entries)
        .expect("the `cli io` catalog is registered")
}

fn entry(code: &str) -> &'static Entry {
    entries()
        .iter()
        .find(|e| e.code == code)
        .unwrap_or_else(|| panic!("`{code}` is not registered"))
}

#[test]
fn every_registered_code_is_in_the_lock_with_its_kind() {
    let missing: Vec<String> = entries()
        .iter()
        .map(|e| {
            let kind = match e.kind {
                Kind::Error => "error",
                Kind::Text => "text",
                other => panic!("unexpected kind {other:?} for `{}`", e.code),
            };
            format!("{} {kind}", e.code)
        })
        .filter(|line| !LOCK.lines().any(|l| l == line))
        .collect();
    assert_eq!(missing, Vec::<String>::new());
}

#[test]
fn the_catalog_registers_exactly_these_codes() {
    let mut codes: Vec<&str> = entries().iter().map(|e| e.code).collect();
    codes.sort_unstable();
    assert_eq!(
        codes,
        [
            "cli-open-exited",
            "cli-open-failed",
            "cli-open-needs-html",
            "cli-open-no-opener",
            "cli-open-skipped-ci",
            "cli-output-report-path",
            "cli-report-write-failed",
            "cli-stdin-document-too-large",
            "cli-stdin-fix-unsupported",
            "cli-stdin-language-invalid",
            "cli-stdin-language-required",
            "cli-stdin-not-readable",
            "cli-stdin-not-utf8",
            "cli-stdin-option-unsupported",
            "cli-stdin-path-given",
            "input-not-utf8",
            "input-unreadable",
            "output-failed",
            "removed-option",
            "render-failed",
            "usage",
        ]
    );
}

#[rstest::rstest]
#[case::usage("usage")]
#[case::removed_option("removed-option")]
#[case::render_failed("render-failed")]
#[case::output_failed("output-failed")]
#[case::input_unreadable("input-unreadable")]
#[case::input_not_utf8("input-not-utf8")]
fn a_general_code_is_an_error_that_carries_the_detail_of_its_site(#[case] code: &str) {
    let e = entry(code);
    assert_eq!((e.kind, e.template), (Kind::Error, "{detail}"));
}

#[rstest::rstest]
#[case::path_given("cli-stdin-path-given", "PATH must be empty when --stdin is set.")]
#[case::option(
    "cli-stdin-option-unsupported",
    "--{option} is not supported with --stdin."
)]
#[case::fix(
    "cli-stdin-fix-unsupported",
    "--fix is not supported with --stdin because stdin has no file to write back to."
)]
#[case::language_required(
    "cli-stdin-language-required",
    "--language is required with --stdin --mode code-comments."
)]
#[case::language_invalid(
    "cli-stdin-language-invalid",
    "--language {language} is not valid for mode {mode}. Valid values: {valid}."
)]
#[case::too_large(
    "cli-stdin-document-too-large",
    "{shown}: larger than the 64 MiB document limit"
)]
#[case::not_readable("cli-stdin-not-readable", "{shown}: cannot read")]
#[case::not_utf8("cli-stdin-not-utf8", "{shown}: not valid UTF-8 (byte {offset})")]
#[case::open_needs_html(
    "cli-open-needs-html",
    "--open shows an HTML report; drop --format or use --format html."
)]
#[case::report_path("cli-output-report-path", "report: {path}")]
#[case::write_failed("cli-report-write-failed", "cannot write the report: {kind}")]
#[case::skipped_ci("cli-open-skipped-ci", "not opening the report: CI is set.")]
#[case::open_failed("cli-open-failed", "warning: could not open the report: {detail}")]
#[case::no_opener("cli-open-no-opener", "no opener is known for this platform")]
#[case::exited("cli-open-exited", "{program} exited with {status}")]
fn a_sentence_keeps_its_published_wording(#[case] code: &str, #[case] template: &str) {
    let e = entry(code);
    assert_eq!((e.kind, e.template), (Kind::Text, template));
}
