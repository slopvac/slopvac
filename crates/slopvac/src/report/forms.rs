//! Render the GitHub issue forms from the field definitions.
//!
//! The output is YAML in the JSON-compatible subset: every string is double-quoted with JSON
//! escapes, so no value needs YAML quoting rules.

use std::fmt::Write as _;

use super::fields::INTRO;
use super::{Category, Field, Kind};

/// The comment at the top of every generated form.
pub const HEADER: &str = "# Generated from crates/slopvac/src/report/fields.rs. Do not edit by hand.\n# Regenerate with: SLOPVAC_UPDATE_FORMS=1 cargo test -p slopvac --test report_forms\n";

fn q(s: &str) -> String {
    serde_json::to_string(s).expect("a string serialises")
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Input => "input",
        Kind::Textarea => "textarea",
        Kind::Dropdown => "dropdown",
        Kind::Checkboxes => "checkboxes",
    }
}

fn element(out: &mut String, f: &Field) {
    // ast-grep-ignore: user-text-literal
    let _ = writeln!(out, "  - type: {}", kind_name(f.kind));
    // ast-grep-ignore: user-text-literal
    let _ = writeln!(out, "    id: {}", f.id);
    out.push_str("    attributes:\n");
    // ast-grep-ignore: user-text-literal
    let _ = writeln!(out, "      label: {}", q(f.label));
    if let Some(d) = f.description {
        // ast-grep-ignore: user-text-literal
        let _ = writeln!(out, "      description: {}", q(d));
    }
    if let Some(r) = f.render {
        // ast-grep-ignore: user-text-literal
        let _ = writeln!(out, "      render: {}", q(r));
    }
    if f.multiple {
        out.push_str("      multiple: true\n");
    }
    match f.kind {
        Kind::Dropdown => {
            out.push_str("      options:\n");
            for o in f.options {
                let _ = writeln!(out, "        - {}", q(o));
            }
        }
        Kind::Checkboxes => {
            out.push_str("      options:\n");
            for (label, required) in f.checkbox_options {
                // ast-grep-ignore: user-text-literal
                let _ = writeln!(out, "        - label: {}", q(label));
                // ast-grep-ignore: user-text-literal
                let _ = writeln!(out, "          required: {required}");
            }
        }
        Kind::Input | Kind::Textarea => {}
    }
    if f.kind != Kind::Checkboxes {
        out.push_str("    validations:\n");
        // ast-grep-ignore: user-text-literal
        let _ = writeln!(out, "      required: {}", f.required);
    }
}

/// The text of `.github/ISSUE_TEMPLATE/<form_file>`.
#[must_use]
pub fn render(category: &Category) -> String {
    let mut out = String::from(HEADER);
    // ast-grep-ignore: user-text-literal
    let _ = writeln!(out, "name: {}", q(category.form_name));
    // ast-grep-ignore: user-text-literal
    let _ = writeln!(out, "description: {}", q(category.description));
    // ast-grep-ignore: user-text-literal
    let _ = writeln!(out, "title: {}", q(category.title_prefix));
    let labels: Vec<String> = category.labels.iter().map(|l| q(l)).collect();
    let _ = writeln!(out, "labels: [{}]", labels.join(", "));
    out.push_str("body:\n");
    out.push_str("  - type: markdown\n    attributes:\n");
    // ast-grep-ignore: user-text-literal
    let _ = writeln!(out, "      value: {}", q(INTRO));
    for f in &category.fields {
        element(&mut out, f);
    }
    out
}
