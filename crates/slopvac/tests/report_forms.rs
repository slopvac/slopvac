//! The GitHub issue forms are generated from the report field definitions.
//!
//! This test regenerates every form and fails when a committed file differs, when a form has no
//! definition, or when a definition has no form. Regenerate after editing
//! `crates/slopvac/src/report/fields.rs`:
//!
//! ```sh
//! SLOPVAC_UPDATE_FORMS=1 cargo test -p slopvac --test report_forms
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use slopvac_cli::report::{categories, forms};

fn form_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/ISSUE_TEMPLATE")
}

#[test]
fn committed_issue_forms_equal_the_generated_ones() {
    let update = std::env::var_os("SLOPVAC_UPDATE_FORMS").is_some();
    let mut stale = Vec::new();
    for category in categories() {
        let path = form_dir().join(category.form_file);
        let generated = forms::render(&category);
        let committed = fs::read_to_string(&path).unwrap_or_default();
        if committed != generated {
            if update {
                fs::write(&path, &generated).unwrap();
            } else {
                stale.push(category.form_file);
            }
        }
    }
    assert!(
        stale.is_empty(),
        "stale issue forms: {stale:?}. Regenerate with SLOPVAC_UPDATE_FORMS=1 cargo test -p slopvac --test report_forms"
    );
}

#[test]
fn every_form_file_has_a_definition_and_the_reverse() {
    let mut on_disk: Vec<String> = fs::read_dir(form_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| Path::new(n).extension().is_some_and(|e| e == "yml") && n != "config.yml")
        .collect();
    on_disk.sort();
    let mut defined: Vec<String> = categories()
        .iter()
        .map(|c| c.form_file.to_owned())
        .collect();
    defined.sort();
    assert_eq!(on_disk, defined);
}

#[test]
fn generated_forms_are_valid_yaml_with_unique_field_ids_github_accepts() {
    for category in categories() {
        let text = forms::render(&category);
        let mut ids: Vec<&str> = category.fields.iter().map(|f| f.id).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "{}: duplicate field id", category.key);
        for id in ids {
            assert!(
                id.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "{}: `{id}` is not a valid form id",
                category.key
            );
        }
        assert!(text.starts_with(forms::HEADER), "{}", category.key);
        assert!(text.ends_with('\n'), "{}", category.key);
    }
}
/// The text in backticks, in the order it appears.
fn code_spans(text: &str) -> Vec<&str> {
    text.split('`').skip(1).step_by(2).collect()
}

/// The code spans of every field description of one category.
fn description_spans(key: &str) -> Vec<String> {
    let spans: Vec<String> = categories()
        .into_iter()
        .filter(|c| c.key == key)
        .flat_map(|c| c.fields)
        .filter_map(|f| f.description)
        .flat_map(|d| {
            code_spans(d)
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(!spans.is_empty(), "no code spans for category {key}");
    spans
}

/// Whether `flag` is a visible long option of `command` or of any command below it.
fn has_visible_flag(command: &clap::Command, flag: &str) -> bool {
    command
        .get_arguments()
        .any(|a| !a.is_hide_set() && a.get_long() == Some(flag))
        || command
            .get_subcommands()
            .any(|sub| has_visible_flag(sub, flag))
}

#[rstest::rstest]
#[case::bug("bug")]
#[case::false_positive("false-positive")]
#[case::missed_slop("missed-slop")]
#[case::mis_tagged("mis-tagged")]
#[case::wrong_fix("wrong-fix")]
#[case::new_rule("new-rule")]
#[case::rule_improvement("rule-improvement")]
fn every_command_a_form_names_is_a_valid_invocation_of_the_binary(#[case] key: &str) {
    let mut commands = 0;
    for span in description_spans(key) {
        let tokens: Vec<&str> = span.split_whitespace().collect();
        let launcher = tokens.first().copied().unwrap_or_default();
        if tokens.len() < 2 || !matches!(launcher, "slopvac" | "uvx") {
            continue;
        }
        commands += 1;
        assert_eq!(
            launcher, "slopvac",
            "{key}: `{span}` runs another launcher, not the installed binary"
        );
        let parsed = slopvac_cli::cli::command().try_get_matches_from(&tokens);
        if let Err(e) = parsed {
            assert!(
                matches!(
                    e.kind(),
                    clap::error::ErrorKind::DisplayVersion | clap::error::ErrorKind::DisplayHelp
                ),
                "{key}: `{span}` is not a valid invocation: {e}"
            );
        }
    }
    assert!(commands > 0, "{key}: the form names no command to check");
}

#[rstest::rstest]
#[case::bug("bug")]
#[case::false_positive("false-positive")]
#[case::missed_slop("missed-slop")]
#[case::mis_tagged("mis-tagged")]
#[case::wrong_fix("wrong-fix")]
#[case::new_rule("new-rule")]
#[case::rule_improvement("rule-improvement")]
fn every_option_a_form_names_is_a_visible_option_of_the_binary(#[case] key: &str) {
    let mut root = slopvac_cli::cli::command();
    // `build` adds the generated `--help` and `--version`.
    root.build();
    for span in description_spans(key) {
        for token in span.split_whitespace() {
            let Some(flag) = token.strip_prefix("--") else {
                continue;
            };
            let flag = flag.split('=').next().unwrap_or_default();
            assert!(
                has_visible_flag(&root, flag),
                "{key}: `{span}` names --{flag}, which the binary does not offer"
            );
        }
    }
}
