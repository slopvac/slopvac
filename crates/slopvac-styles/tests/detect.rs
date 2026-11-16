//! House-style detection over fixture repositories.

use std::fs;

use slopvac_styles::{BuiltinSource, detect_house_styles};

fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in files {
        fs::write(dir.path().join(name), text).unwrap();
    }
    dir
}

#[test]
fn vale_google_suggests_the_google_preset_with_its_line() {
    let dir = repo(&[(
        ".vale.ini",
        "StylesPath = styles\nMinAlertLevel = suggestion\n\n[*.md]\nBasedOnStyles = Vale, Google\n",
    )]);
    let found = detect_house_styles(dir.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].preset.as_deref(), Some("google"));
    assert_eq!(found[0].evidence.file, ".vale.ini");
    assert_eq!(found[0].evidence.line, 5);
    assert_eq!(found[0].evidence.text, "BasedOnStyles = Vale, Google");
}

#[test]
fn vale_microsoft_is_reported_without_a_preset() {
    let dir = repo(&[(".vale.ini", "[*.md]\nBasedOnStyles = Microsoft\n")]);
    let found = detect_house_styles(dir.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].preset, None);
    assert_eq!(found[0].evidence.line, 2);
    assert!(found[0].describe().contains("no bundled preset"));
}

#[test]
fn an_unknown_vale_style_is_reported_by_its_own_name() {
    let dir = repo(&[(".vale.ini", "BasedOnStyles = Acme\n")]);
    let found = detect_house_styles(dir.path());
    assert_eq!(found[0].style, "Acme");
    assert_eq!(found[0].preset, None);
}

#[test]
fn a_commented_out_vale_line_is_ignored() {
    let dir = repo(&[(
        ".vale.ini",
        "# BasedOnStyles = Google\n; BasedOnStyles = Google\n",
    )]);
    assert_eq!(detect_house_styles(dir.path()), vec![]);
}

#[test]
fn style_md_naming_the_google_guide_suggests_google() {
    let dir = repo(&[(
        "STYLE.md",
        "# Writing\n\nWe follow the Google Developer Documentation Style Guide.\n",
    )]);
    let found = detect_house_styles(dir.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].preset.as_deref(), Some("google"));
    assert_eq!(found[0].evidence.file, "STYLE.md");
    assert_eq!(found[0].evidence.line, 3);
}

#[test]
fn underscore_style_md_naming_ste_suggests_ste() {
    let dir = repo(&[("_STYLE.md", "Write in Simplified Technical English.\n")]);
    let found = detect_house_styles(dir.path());
    assert_eq!(found[0].preset.as_deref(), Some("ste"));
    assert_eq!(found[0].evidence.file, "_STYLE.md");
}

#[test]
fn contributing_counts_only_inside_a_style_section() {
    let outside = repo(&[(
        "CONTRIBUTING.md",
        "# Contributing\n\nSee developers.google.com/style for Go code.\n",
    )]);
    assert_eq!(detect_house_styles(outside.path()), vec![]);

    let inside = repo(&[(
        "CONTRIBUTING.md",
        "# Contributing\n\n## Documentation style\n\nFollow https://developers.google.com/style.\n\n## Tests\n",
    )]);
    let found = detect_house_styles(inside.path());
    assert_eq!(found[0].preset.as_deref(), Some("google"));
    assert_eq!(found[0].evidence.line, 5);
}

#[test]
fn the_same_style_from_two_files_is_reported_once_at_the_first_source() {
    let dir = repo(&[
        (".vale.ini", "BasedOnStyles = Google\n"),
        ("STYLE.md", "Google developer documentation style guide\n"),
    ]);
    let found = detect_house_styles(dir.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].evidence.file, ".vale.ini");
}

#[test]
fn a_repo_with_no_style_files_has_no_suggestions() {
    let dir = repo(&[("README.md", "# Hello\n")]);
    assert_eq!(detect_house_styles(dir.path()), vec![]);
}

#[test]
fn every_suggested_preset_is_bundled() {
    let dir = repo(&[(".vale.ini", "BasedOnStyles = Google, STE\n")]);
    let found = detect_house_styles(dir.path());
    assert_eq!(found.len(), 2);
    for s in found {
        let preset = s.preset.expect("both have presets");
        assert!(BuiltinSource::names().any(|n| n == preset));
    }
}
