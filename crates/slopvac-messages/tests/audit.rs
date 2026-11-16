//! The registry audit over hand-built entries.

use slopvac_messages::{Entry, Kind, Param, ParamKind, audit};

const PATH: &[Param] = &[Param::new("path", ParamKind::Path)];

fn entry(code: &'static str, template: &'static str, summary: &'static str) -> Entry {
    Entry::new(code, Kind::Error, "E::V", template, PATH, summary)
}

fn problems(catalogs: &[(&str, &[Entry])]) -> Vec<String> {
    audit(catalogs).iter().map(ToString::to_string).collect()
}

#[test]
fn a_clean_registry_has_no_violations() {
    let a = [entry("one", "read {path}", "Reading failed.")];
    let b = [entry("two", "write {path}", "Writing failed.")];
    assert_eq!(problems(&[("a", &a), ("b", &b)]), Vec::<String>::new());
}

#[test]
fn a_code_defined_in_two_catalogs_is_reported_once_at_the_second() {
    let a = [entry("same", "read {path}", "Reading failed.")];
    let b = [entry("same", "write {path}", "Writing failed.")];
    assert_eq!(
        problems(&[("a", &a), ("b", &b)]),
        ["b / same: the code is already defined in `a`"]
    );
}

#[test]
fn a_blank_summary_is_reported() {
    let a = [entry("one", "read {path}", "  ")];
    assert_eq!(problems(&[("a", &a)]), ["a / one: the summary is empty"]);
}

#[test]
fn a_placeholder_without_a_parameter_is_reported() {
    let a = [entry("one", "read {file}", "Reading failed.")];
    assert_eq!(
        problems(&[("a", &a)]),
        ["a / one: the template uses `{file}`, which is not a parameter"]
    );
}

#[test]
fn escaped_braces_are_not_placeholders() {
    let a = [entry("one", "write {{file}} for {path}", "Reading failed.")];
    assert_eq!(problems(&[("a", &a)]), Vec::<String>::new());
}

#[rstest::rstest]
#[case::bead("see slopvac-8l4.21.6 for why", "slopvac-8l4.")]
#[case::pull_request("fixed in #470", "#470")]
fn internal_ids_in_a_template_are_reported(#[case] template: &'static str, #[case] id: &str) {
    let a = [entry("one", template, "Reading failed.")];
    assert_eq!(
        problems(&[("a", &a)]),
        [format!(
            "a / one: the template names the internal id `{id}`"
        )]
    );
}

#[rstest::rstest]
#[case::crate_name("run slopvac-lint to check", true)]
#[case::crate_name_before_comma("see slopvac-lint, then retry", true)]
#[case::heading_hash("use # for headings", true)]
#[case::bead("see slopvac-8l4.21", false)]
fn only_internal_ids_are_rejected(#[case] template: &'static str, #[case] clean: bool) {
    let a = [entry("one", template, "Reading failed.")];
    assert_eq!(audit(&[("a", &a)]).is_empty(), clean);
}
