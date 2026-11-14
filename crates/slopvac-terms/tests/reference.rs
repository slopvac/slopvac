//! The generated built-in terms reference page.

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// The built example binary. The test builds it first, so it also works when
/// only this test target runs (`cargo test --test reference` skips examples).
fn example() -> PathBuf {
    static BUILD: std::sync::Once = std::sync::Once::new();
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    BUILD.call_once(|| {
        let status = Command::new(env!("CARGO"))
            .args([
                "build",
                "--quiet",
                "--example",
                "reference",
                "--manifest-path",
            ])
            .arg(manifest)
            .status()
            .unwrap();
        assert!(status.success(), "building the example failed");
    });
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap().parent().unwrap().join("examples");
    let path = dir.join(format!("reference{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.exists(),
        "example binary not built: {}",
        path.display()
    );
    path
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("slopvac-terms-ref-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_example_writes_terms_index_md_under_the_out_dir() {
    let out = scratch("write");
    let status = Command::new(example()).arg(&out).status().unwrap();
    assert!(status.success());
    let written = std::fs::read_to_string(out.join("terms/index.md")).unwrap();
    assert_eq!(written, page());
    // Running again over existing output succeeds and rewrites the page.
    assert!(
        Command::new(example())
            .arg(&out)
            .status()
            .unwrap()
            .success()
    );
    std::fs::remove_dir_all(&out).unwrap();
}

#[test]
fn the_example_fails_without_writing_when_the_out_dir_is_a_file() {
    let dir = scratch("io-failure");
    let blocker: &Path = &dir.join("not-a-dir");
    std::fs::write(blocker, "x").unwrap();
    let output = Command::new(example()).arg(blocker).output().unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).is_empty());
    assert_eq!(std::fs::read_to_string(blocker).unwrap(), "x");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_example_needs_an_out_dir_argument() {
    let output = Command::new(example()).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage: reference <out-dir>"));
}
