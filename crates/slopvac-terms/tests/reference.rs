//! The generated built-in terms reference page.

use slopvac_terms::{TermsFile, reference_pages};

fn page() -> String {
    let pages = reference_pages();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].0, "terms/index.md");
    pages[0].1.clone()
}

#[test]
fn every_builtin_term_appears_with_its_variants_and_mode() {
    let text = page();
    for t in TermsFile::builtin().terms {
        let escaped = t.term.replace('|', "\\|");
        assert!(text.contains(&format!("| `{escaped}` | ")), "{}", t.term);
        for v in &t.variants {
            assert!(text.contains(&format!("`{v}`")), "{}: {v}", t.term);
        }
    }
}

#[test]
fn the_page_states_the_count_provenance_and_modes() {
    let text = page();
    let n = TermsFile::builtin().terms.len();
    assert!(
        text.starts_with(&format!("# Built-in terms\n\nslopvac ships {n} terms")),
        "{text}"
    );
    assert!(text.contains("Apache-2.0"));
    assert!(text.contains("**exact spelling only**"));
}

#[test]
fn rows_show_case_modes_and_group_by_kind() {
    let text = page();
    assert!(
        text.contains("| `Python` | - | exact spelling only |"),
        "{text}"
    );
    assert!(
        text.contains("| `GitHub` | - | flags wrong case |"),
        "{text}"
    );
    assert!(
        text.contains("| `Kubernetes` | `K8s` | flags wrong case |"),
        "{text}"
    );
    assert!(text.contains("\n## protocol\n\n| Term |"), "{text}");
}
