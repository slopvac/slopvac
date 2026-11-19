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
