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
    let on_disk = fs::read_to_string(&path).unwrap();
    assert_eq!(
        on_disk,
        guide_doc(),
        "docs/migrate-2-to-3.md is stale; run UPDATE_DOCS=1 cargo test -p slopvac-migrate --test guide"
    );
}

#[test]
fn doc_content_is_the_prime_output() {
    assert!(guide_doc().ends_with(prime_migrate()));
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
            prime_migrate().contains(&tok),
            "guide does not mention {tok} ({row})"
        );
        checked += 1;
    }
    assert!(checked >= 20, "only {checked} removed surfaces checked");
}
