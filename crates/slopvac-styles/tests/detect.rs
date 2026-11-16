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

const GOOGLE_LINE: &str = "We follow the Google developer documentation style guide.\n";

#[test]
fn an_oversized_file_is_skipped() {
    let padding = "x".repeat(1024 * 1024 + 1);
    let dir = repo(&[("STYLE.md", &format!("{GOOGLE_LINE}{padding}"))]);
    assert_eq!(detect_house_styles(dir.path()), vec![]);
}

#[test]
fn a_file_at_the_size_cap_is_still_read() {
    let mut text = String::from(GOOGLE_LINE);
    text.push_str(&"x".repeat(1024 * 1024 - text.len()));
    let dir = repo(&[("STYLE.md", &text)]);
    assert_eq!(detect_house_styles(dir.path()).len(), 1);
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::process::Command;

    fn mkfifo(path: &std::path::Path) {
        let status = Command::new("mkfifo").arg(path).status().unwrap();
        assert!(status.success());
    }

    #[test]
    fn a_fifo_is_skipped_without_blocking() {
        let dir = tempfile::tempdir().unwrap();
        mkfifo(&dir.path().join("STYLE.md"));
        assert_eq!(detect_house_styles(dir.path()), vec![]);
    }

    #[test]
    fn a_symlink_to_a_fifo_inside_the_root_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        mkfifo(&dir.path().join("pipe"));
        symlink("pipe", dir.path().join("STYLE.md")).unwrap();
        assert_eq!(detect_house_styles(dir.path()), vec![]);
    }

    #[test]
    fn a_symlink_to_dev_zero_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        symlink("/dev/zero", dir.path().join("STYLE.md")).unwrap();
        assert_eq!(detect_house_styles(dir.path()), vec![]);
    }

    #[test]
    fn a_symlink_that_escapes_the_root_is_skipped() {
        let outside = repo(&[("secret.md", GOOGLE_LINE)]);
        let dir = tempfile::tempdir().unwrap();
        symlink(
            outside.path().join("secret.md"),
            dir.path().join("STYLE.md"),
        )
        .unwrap();
        assert_eq!(detect_house_styles(dir.path()), vec![]);
    }

    #[test]
    fn a_symlink_that_stays_inside_the_root_is_followed() {
        let dir = repo(&[("docs.md", GOOGLE_LINE)]);
        symlink("docs.md", dir.path().join("STYLE.md")).unwrap();
        let found = detect_house_styles(dir.path());
        assert_eq!(found[0].evidence.file, "STYLE.md");
    }
}

#[test]
fn inline_vale_comments_do_not_become_style_names() {
    for line in [
        "BasedOnStyles = Google ; house rules",
        "BasedOnStyles = Google # house rules",
        "BasedOnStyles = Vale, Google ;x",
    ] {
        let dir = repo(&[(".vale.ini", &format!("{line}\n"))]);
        let found = detect_house_styles(dir.path());
        assert_eq!(found.len(), 1, "{line}");
        assert_eq!(found[0].preset.as_deref(), Some("google"), "{line}");
        assert_eq!(found[0].evidence.text, line);
    }
}

#[test]
fn a_comment_only_value_names_no_style() {
    let dir = repo(&[(".vale.ini", "BasedOnStyles = ; none yet\n")]);
    assert_eq!(detect_house_styles(dir.path()), vec![]);
}

#[test]
fn rejection_and_example_lines_are_not_adoption() {
    for line in [
        "We do not follow the Google developer documentation style guide.",
        "We don't use the Google developer documentation style guide.",
        "Unlike the Google developer documentation style guide, we allow semicolons.",
        "We never adopted the Google developer documentation style guide.",
        "For example, the Google developer documentation style guide says X.",
    ] {
        for file in ["STYLE.md", "_STYLE.md"] {
            let dir = repo(&[(file, &format!("{line}\n"))]);
            assert_eq!(detect_house_styles(dir.path()), vec![], "{file}: {line}");
        }
        let dir = repo(&[("CONTRIBUTING.md", &format!("## Style\n\n{line}\n"))]);
        assert_eq!(detect_house_styles(dir.path()), vec![], "{line}");
    }
}

#[test]
fn a_mention_inside_a_code_fence_is_not_adoption() {
    let dir = repo(&[(
        "STYLE.md",
        "```text\nGoogle developer documentation style guide\n```\n",
    )]);
    assert_eq!(detect_house_styles(dir.path()), vec![]);
}

#[test]
fn indented_and_tab_separated_headings_open_a_style_section() {
    for heading in ["   ## Style", "##\tStyle", "## Style ##"] {
        let dir = repo(&[("CONTRIBUTING.md", &format!("{heading}\n\n{GOOGLE_LINE}"))]);
        let found = detect_house_styles(dir.path());
        assert_eq!(found.len(), 1, "{heading:?}");
        assert_eq!(found[0].evidence.line, 3);
    }
}

#[test]
fn four_space_indent_is_code_not_a_heading() {
    let dir = repo(&[("CONTRIBUTING.md", &format!("    ## Style\n\n{GOOGLE_LINE}"))]);
    assert_eq!(detect_house_styles(dir.path()), vec![]);
}

#[test]
fn a_style_section_covers_its_subsections_and_ends_at_a_sibling_heading() {
    let text = "# Contributing\n\
        Simplified Technical English is not used here.\n\
        ## Writing style\n\
        Follow the Google developer documentation style guide.\n\
        ### Headings\n\
        Microsoft Writing Style Guide covers tone.\n\
        ## Testing\n\
        Simplified Technical English applies to test names.\n";
    let dir = repo(&[("CONTRIBUTING.md", text)]);
    let found = detect_house_styles(dir.path());
    let seen: Vec<(Option<&str>, usize)> = found
        .iter()
        .map(|s| (s.preset.as_deref(), s.evidence.line))
        .collect();
    assert_eq!(seen, [(Some("google"), 4), (None, 6)]);
}

#[test]
fn a_heading_inside_a_fence_does_not_open_a_style_section() {
    let text = format!("# Contributing\n\n```md\n## Style\n```\n\n{GOOGLE_LINE}");
    let dir = repo(&[("CONTRIBUTING.md", &text)]);
    assert_eq!(detect_house_styles(dir.path()), vec![]);
}

#[test]
fn a_heading_inside_a_fence_does_not_close_a_style_section() {
    let text = format!("## Style\n\n```md\n## Testing\n```\n\n{GOOGLE_LINE}");
    let dir = repo(&[("CONTRIBUTING.md", &text)]);
    let found = detect_house_styles(dir.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].evidence.line, 7);
}

#[test]
fn a_long_fence_is_not_closed_by_a_shorter_one() {
    let text = format!("````md\n```\n{GOOGLE_LINE}```\n{GOOGLE_LINE}````\n");
    let dir = repo(&[("STYLE.md", &text)]);
    assert_eq!(detect_house_styles(dir.path()), vec![]);
}

#[test]
fn a_fence_closes_on_the_same_marker_with_at_least_the_opening_length() {
    let text = format!("```\nx\n`````\n{GOOGLE_LINE}");
    let dir = repo(&[("STYLE.md", &text)]);
    assert_eq!(detect_house_styles(dir.path()).len(), 1);
}

#[test]
fn a_closing_fence_line_with_text_after_it_does_not_close() {
    let text = format!("```\n``` not a close\n{GOOGLE_LINE}```\n");
    let dir = repo(&[("STYLE.md", &text)]);
    assert_eq!(detect_house_styles(dir.path()), vec![]);
}

#[test]
fn a_cue_in_another_clause_does_not_reject_the_guide() {
    for line in [
        "We follow the Google developer documentation style guide, but never use emoji.",
        "We follow the Google developer documentation style guide, except for headings.",
        "We follow the Google developer documentation style guide; we don't use emoji.",
        "We follow the Google developer documentation style guide and avoid jargon.",
        "We don't use emoji. We follow the Google developer documentation style guide.",
        "We don't use emoji, but we follow the Google developer documentation style guide.",
    ] {
        let dir = repo(&[("STYLE.md", &format!("{line}\n"))]);
        let found = detect_house_styles(dir.path());
        assert_eq!(found.len(), 1, "{line}");
        assert_eq!(found[0].preset.as_deref(), Some("google"), "{line}");
    }
}

#[test]
fn a_cue_before_the_guide_in_its_clause_rejects_it() {
    for line in [
        "We do not follow the Google developer documentation style guide.",
        "We follow Microsoft, not the Google developer documentation style guide.",
        "We use STE instead of the Google developer documentation style guide.",
    ] {
        let dir = repo(&[("STYLE.md", &format!("{line}\n"))]);
        assert!(
            detect_house_styles(dir.path())
                .iter()
                .all(|s| s.preset.as_deref() != Some("google")),
            "{line}"
        );
    }
}

fn presets(line: &str) -> Vec<Option<String>> {
    let dir = repo(&[("STYLE.md", &format!("{line}\n"))]);
    detect_house_styles(dir.path())
        .into_iter()
        .map(|s| s.preset)
        .collect()
}

#[test]
fn a_fence_is_not_closed_by_the_other_marker() {
    let text = format!("```\n~~~\n{GOOGLE_LINE}```\n");
    let dir = repo(&[("STYLE.md", &text)]);
    assert_eq!(detect_house_styles(dir.path()), vec![]);
    let text = format!("~~~\n```\n{GOOGLE_LINE}~~~\n");
    let dir = repo(&[("STYLE.md", &text)]);
    assert_eq!(detect_house_styles(dir.path()), vec![]);
}

#[test]
fn without_and_except_before_the_name_reject_it() {
    for line in [
        "We write without the Google developer documentation style guide.",
        "We follow everything except the Google developer documentation style guide.",
    ] {
        assert_eq!(presets(line), vec![], "{line}");
    }
}

#[test]
fn except_after_the_name_does_not_reject_it() {
    assert_eq!(
        presets("We follow the Google developer documentation style guide, except for headings."),
        vec![Some("google".to_owned())]
    );
}

#[test]
fn a_rejection_carries_across_a_list_of_guides() {
    for line in [
        "We do not follow the Google developer documentation style guide, or Simplified Technical English.",
        "We follow neither, not Google developer documentation, Simplified Technical English or the Microsoft Writing Style Guide.",
        "Not the Google developer documentation style guide, and Simplified Technical English.",
    ] {
        assert_eq!(presets(line), vec![], "{line}");
    }
}

#[test]
fn a_list_after_an_adopted_guide_is_adopted_with_it() {
    assert_eq!(
        presets(
            "We follow the Google developer documentation style guide, Simplified Technical English."
        ),
        vec![Some("google".to_owned()), Some("ste".to_owned())]
    );
}

#[test]
fn a_rejection_after_a_comma_ends_the_earlier_adoption() {
    assert_eq!(
        presets(
            "We follow the Google developer documentation style guide, not Simplified Technical English."
        ),
        vec![Some("google".to_owned())]
    );
}
