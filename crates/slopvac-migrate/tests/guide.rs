//! The guide: one source for the prime topic and the doc, and coverage of every
//! removed surface in the compatibility dispositions.

use std::fs;
use std::path::PathBuf;

use slopvac_migrate::{guide_doc, prime_migrate};
use toml_edit::DocumentMut;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn doc_matches_the_guide_source() {
    let path = repo().join("docs/migrate-2-to-3.md");
    if std::env::var_os("UPDATE_DOCS").is_some() {
        fs::write(&path, guide_doc()).unwrap();
    }
    let on_disk = fs::read(&path).unwrap();
    assert!(
        on_disk == guide_doc().as_bytes(),
        "docs/migrate-2-to-3.md is stale; run UPDATE_DOCS=1 cargo test -p slopvac-migrate --test guide"
    );
}

const DOC_URL: &str = "https://github.com/slopvac/slopvac/blob/main/docs/migrate-2-to-3.md";

/// The `## ` sections of the generated doc, each with its heading and body.
fn doc_sections() -> Vec<(String, String)> {
    let doc = guide_doc();
    let mut sections: Vec<(String, String)> = Vec::new();
    for line in doc.split_inclusive('\n') {
        if let Some(heading) = line.strip_prefix("## ") {
            sections.push((heading.trim_end().to_owned(), String::new()));
        }
        if let Some((_, text)) = sections.last_mut() {
            text.push_str(line);
        }
    }
    sections
}

#[test]
fn prime_prints_the_decision_sections_in_full_and_in_order() {
    let prime = prime_migrate();
    let sections = doc_sections();
    let mut from = 0;
    for heading in [
        "1. Find the 2.x surfaces",
        "2. Preview",
        "3. Resolve blockers",
        "5. Verify and undo",
    ] {
        let (_, text) = sections
            .iter()
            .find(|(h, _)| h == heading)
            .unwrap_or_else(|| panic!("doc lacks {heading}"));
        assert!(text.lines().count() > 3, "{heading} has a body");
        let at = prime[from..]
            .find(text.as_str())
            .unwrap_or_else(|| panic!("prime lacks the full {heading} section, in order"));
        from += at + text.len();
    }
}

#[test]
fn prime_links_the_reference_sections_instead_of_printing_them() {
    let prime = prime_migrate();
    for (heading, anchor) in [
        ("4. Apply", "4-apply"),
        ("6. Removed in 3.0", "6-removed-in-30"),
    ] {
        let link = format!("Read \"{heading}\" at {DOC_URL}#{anchor}.");
        assert_eq!(prime.matches(&link).count(), 1, "{heading}");
        assert!(!prime.contains(&format!("## {heading}")), "{heading}");
        // The linked anchor is a heading of the generated doc.
        assert!(
            guide_doc().contains(&format!("\n## {heading}\n")),
            "{heading}"
        );
    }
    assert!(
        !prime.contains("| Surface |"),
        "the removed-surface table is linked"
    );
    assert!(!prime.contains("--write"), "the apply steps are linked");
}

#[test]
fn markers_stay_out_of_the_doc_and_prime() {
    assert!(!guide_doc().contains("<!-- prime:"));
    assert!(!prime_migrate().contains("<!-- prime:"));
}

#[test]
fn the_preview_section_labels_the_counts_as_the_3_0_engine_on_both_sides() {
    let sections = doc_sections();
    let (_, preview) = sections
        .iter()
        .find(|(h, _)| h == "2. Preview")
        .expect("doc has a Preview section");
    let text = preview.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        text.contains("from the 3.0 engine on your files before and after migration"),
        "{text}"
    );
    // Each of these words would present the before side as a 2.x run.
    for claim in ["baseline", "preserve", "same as 2.x"] {
        assert!(!text.contains(claim), "{claim}: {text}");
    }
}

/// The token a removed surface is named by in the guide.
fn token(row: &str) -> Option<String> {
    let mut parts = row.split(':');
    match (parts.next()?, parts.next(), parts.next()) {
        ("opt", Some(_), Some(opt)) if opt.starts_with("--") => Some(opt.to_owned()),
        ("cmd", Some(cmd), None) => Some(format!("`{cmd}`")),
        ("action", Some("input"), Some(input)) => Some(format!("`{input}`")),
        ("hook", Some(id), None) => Some(id.to_owned()),
        _ => None,
    }
}

#[test]
fn guide_names_every_removed_surface() {
    let text = fs::read_to_string(repo().join("docs/contract/compat/dispositions.toml")).unwrap();
    let doc: DocumentMut = text.parse().unwrap();
    let mut checked = 0;
    for (row, item) in doc.iter() {
        let removed = item
            .as_inline_table()
            .and_then(|t| t.get("disposition"))
            .and_then(|v| v.as_str())
            == Some("remove");
        if !removed {
            continue;
        }
        let Some(tok) = token(row) else { continue };
        assert!(
            guide_doc().contains(&tok),
            "guide does not mention {tok} ({row})"
        );
        checked += 1;
    }
    assert!(checked >= 20, "only {checked} removed surfaces checked");
}
